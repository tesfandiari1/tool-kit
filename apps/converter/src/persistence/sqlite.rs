use std::{collections::HashSet, path::Path, time::Duration};

use sha2::{Digest, Sha256};
use sqlx::{
    migrate::MigrateError,
    sqlite::{
        SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions, SqliteRow,
        SqliteSynchronous,
    },
    Row, Sqlite, SqlitePool, Transaction,
};
use thiserror::Error;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tool_kit_worker_protocol::is_lowercase_sha256;
use uuid::Uuid;

use super::model::{
    AttemptState, CommitOperation, ConversionState, CreateOutcome, DocumentClassification,
    EngineRecord, FailedResult, FailureStage, LocalAnalysis, LocalStart, NewArtifact,
    NewConversion, RecoveryCandidate, RequeueOutcome, StoredArtifact, StoredAttempt,
    StoredConversion, StoredFailure, StoredSource,
};

pub const DATABASE_FILENAME: &str = "converter.sqlite";
const AUTH_SCOPE: &str = "bootstrap";
const MAX_POOL_CONNECTIONS: u32 = 4;
/// Admission and `accepting_jobs` must count the same rows.
const ACTIVE_COUNT_SQL: &str = "SELECT COUNT(*) AS active_count
     FROM conversions
     WHERE status IN ('queued', 'converting_local', 'finalizing')";

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone, Debug)]
pub struct SqliteRepository {
    pool: SqlitePool,
    max_active_jobs: u32,
}

impl SqliteRepository {
    pub async fn open(
        data_dir: &Path,
        max_active_jobs: u32,
        busy_timeout: Duration,
    ) -> Result<Self, RepositoryError> {
        if !data_dir.is_absolute() {
            return Err(RepositoryError::DataDirectoryMustBeAbsolute);
        }
        if max_active_jobs == 0 {
            return Err(RepositoryError::InvalidActiveJobLimit);
        }

        let metadata = std::fs::metadata(data_dir).map_err(RepositoryError::DataDirectory)?;
        if !metadata.is_dir() {
            return Err(RepositoryError::DataDirectoryNotDirectory);
        }

        let database_path = data_dir.join(DATABASE_FILENAME);
        let options = SqliteConnectOptions::new()
            .filename(&database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(busy_timeout);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_POOL_CONNECTIONS)
            .connect_with(options)
            .await?;

        MIGRATOR.run(&pool).await?;
        probe_database(&pool).await?;

        Ok(Self {
            pool,
            max_active_jobs,
        })
    }

    pub async fn create_or_replay(
        &self,
        input: NewConversion,
    ) -> Result<CreateOutcome, RepositoryError> {
        validate_new_conversion(&input)?;

        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let existing = sqlx::query(
            "SELECT id, request_fingerprint
             FROM conversions
             WHERE auth_scope = ?1 AND idempotency_key_hash = ?2",
        )
        .bind(AUTH_SCOPE)
        .bind(&input.idempotency_key_sha256)
        .fetch_optional(&mut *transaction)
        .await?;

        if let Some(row) = existing {
            let existing_id = parse_uuid(row.try_get::<String, _>("id")?, "conversions.id")?;
            let existing_fingerprint: String = row.try_get("request_fingerprint")?;
            if existing_fingerprint != input.request_fingerprint {
                transaction.commit().await?;
                return Ok(CreateOutcome::Conflict);
            }
            let existing = load_conversion(&mut transaction, existing_id)
                .await?
                .ok_or(RepositoryError::MissingCommittedConversion)?;
            transaction.commit().await?;
            return Ok(CreateOutcome::Replay(existing));
        }

        let active_count: i64 = sqlx::query(ACTIVE_COUNT_SQL)
            .fetch_one(&mut *transaction)
            .await?
            .try_get("active_count")?;
        if active_count >= i64::from(self.max_active_jobs) {
            transaction.commit().await?;
            return Ok(CreateOutcome::Capacity);
        }

        let created_at = now_rfc3339()?;
        let conversion_id = input.id.hyphenated().to_string();
        let attempt_id = input.initial_attempt_id.hyphenated().to_string();
        let client_run_id = input.client_run_id.hyphenated().to_string();
        let source_byte_length = i64::try_from(input.source.byte_length)
            .map_err(|_| RepositoryError::InvalidInput("source byte length is too large"))?;

        sqlx::query(
            "INSERT INTO conversions (
                id, client_run_id, auth_scope, idempotency_key_hash,
                request_fingerprint, status, source_relative_path,
                source_media_type, source_byte_length, source_sha256,
                reason_codes_json, warnings_json,
                origin_request_id, created_at, updated_at,
                ocr_language_correction, ocr_custom_words, speaker_count,
                speech_locale
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7, ?8, ?9,
                '[]', '[]', ?10, ?11, ?11, ?12, ?13, ?14, ?15
             )",
        )
        .bind(&conversion_id)
        .bind(&client_run_id)
        .bind(AUTH_SCOPE)
        .bind(&input.idempotency_key_sha256)
        .bind(&input.request_fingerprint)
        .bind(&input.source.relative_path)
        .bind(&input.source.media_type)
        .bind(source_byte_length)
        .bind(&input.source.sha256)
        .bind(&input.origin_request_id)
        .bind(&created_at)
        .bind(input.ocr_language_correction)
        .bind(&input.ocr_custom_words)
        .bind(input.speaker_count.map(i64::from))
        .bind(&input.speech_locale)
        .execute(&mut *transaction)
        .await?;

        sqlx::query(
            "INSERT INTO attempts (
                id, conversion_id, attempt_number, state, recovery_count,
                reason_codes_json, warnings_json, created_at, updated_at
             ) VALUES (?1, ?2, 1, 'queued', 0, '[]', '[]', ?3, ?3)",
        )
        .bind(&attempt_id)
        .bind(&conversion_id)
        .bind(&created_at)
        .execute(&mut *transaction)
        .await?;

        let updated = sqlx::query(
            "UPDATE conversions
             SET active_attempt_id = ?1
             WHERE id = ?2 AND active_attempt_id IS NULL",
        )
        .bind(&attempt_id)
        .bind(&conversion_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(RepositoryError::CorruptData(
                "conversions.active_attempt_id",
            ));
        }

        // Complete every fallible database read before commit. Once commit
        // succeeds, the caller may safely treat the source tree as owned.
        let created = load_conversion(&mut transaction, input.id)
            .await?
            .ok_or(RepositoryError::MissingCommittedConversion)?;
        transaction
            .commit()
            .await
            .map_err(|source| RepositoryError::CommitOutcomeUncertain {
                operation: CommitOperation::CreateConversion,
                conversion_id: input.id,
                attempt_id: input.initial_attempt_id,
                source,
            })?;
        Ok(CreateOutcome::Created(created))
    }

    pub async fn start_local(
        &self,
        conversion_id: Uuid,
        attempt_id: Uuid,
        start: LocalStart,
    ) -> Result<StoredConversion, RepositoryError> {
        validate_local_start(&start)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        apply_start_local(&mut transaction, conversion_id, attempt_id, &start).await?;
        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::StartLocal,
            conversion_id,
            attempt_id,
        )
        .await?;
        Ok(conversion)
    }

    /// Claims the oldest queued row this process can actually run.
    ///
    /// `servable_media_types` is the boot's engine set, not the contract's:
    /// a row whose engine is absent here is left queued for a boot that has
    /// it, while the rows behind it still run. Failing it instead would
    /// destroy an audio job admitted while the worker was up.
    pub async fn claim_next_queued(
        &self,
        servable_media_types: &[&str],
        start_for: impl FnOnce(&str) -> Option<LocalStart>,
    ) -> Result<Option<StoredConversion>, RepositoryError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        // The list is bound as one JSON array rather than spliced into the
        // SQL: the statement stays a literal, and the set varies per boot.
        let servable = serde_json::to_string(servable_media_types)
            .map_err(|_| RepositoryError::InvalidInput("servable media types are invalid"))?;
        let next = sqlx::query(
            "SELECT c.id AS conversion_id, a.id AS attempt_id, c.source_media_type
             FROM conversions AS c
             JOIN attempts AS a
               ON a.conversion_id = c.id
              AND a.id = c.active_attempt_id
             WHERE c.auth_scope = ?1
               AND c.status = 'queued'
               AND a.state = 'queued'
               AND c.source_media_type IN (SELECT value FROM json_each(?2))
             ORDER BY a.queue_seq ASC
             LIMIT 1",
        )
        .bind(AUTH_SCOPE)
        .bind(&servable)
        .fetch_optional(&mut *transaction)
        .await?;

        let Some(next) = next else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let conversion_id = parse_uuid(next.try_get("conversion_id")?, "conversions.id")?;
        let attempt_id = parse_uuid(next.try_get("attempt_id")?, "attempts.id")?;
        let source_media_type: String = next.try_get("source_media_type")?;
        let Some(start) = start_for(&source_media_type) else {
            // The SELECT already skipped every media type this boot cannot
            // serve, so reaching here means no build knows the type at all.
            // Fail it in the same transaction so it never poisons the queue.
            fail_unclaimable_source(&mut transaction, conversion_id, attempt_id).await?;
            commit_transition(
                transaction,
                CommitOperation::ClaimNextQueued,
                conversion_id,
                attempt_id,
            )
            .await?;
            return Ok(None);
        };
        validate_local_start(&start)?;
        apply_start_local(&mut transaction, conversion_id, attempt_id, &start).await?;
        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::ClaimNextQueued,
            conversion_id,
            attempt_id,
        )
        .await?;
        Ok(Some(conversion))
    }

    pub async fn mark_finalizing(
        &self,
        conversion_id: Uuid,
        attempt_id: Uuid,
        analysis: LocalAnalysis,
        // The staged Markdown's size and digest, as the engine measured them.
        markdown: (u64, &str),
        // The engine that wrote the Markdown, when it is not the one the claim
        // recorded: a scanned PDF Vision read publishes under Vision's name.
        relabel: Option<&LocalStart>,
    ) -> Result<StoredConversion, RepositoryError> {
        let analysis = encode_local_analysis(&analysis)?;
        let markdown_byte_length = i64::try_from(markdown.0)
            .ok()
            .filter(|length| *length > 0)
            .ok_or(RepositoryError::InvalidInput(
                "markdown byte length is invalid",
            ))?;
        validate_sha256(markdown.1, "markdown hash is invalid")?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_active_state(
            &mut transaction,
            conversion_id,
            attempt_id,
            ConversionState::ConvertingLocal,
            AttemptState::ConvertingLocal,
        )
        .await?;
        let updated_at = now_rfc3339()?;
        if let Some(start) = relabel {
            validate_local_start(start)?;
            let attempt = sqlx::query(
                "UPDATE attempts SET engine_name = ?1, engine_version = ?2, route = ?3
                 WHERE conversion_id = ?4 AND id = ?5",
            )
            .bind(&start.engine.name)
            .bind(&start.engine.version)
            .bind(&start.route)
            .bind(conversion_id.hyphenated().to_string())
            .bind(attempt_id.hyphenated().to_string())
            .execute(&mut *transaction)
            .await?;
            require_one_transition_row(attempt.rows_affected(), "attempts.route")?;
            let conversion = sqlx::query(
                "UPDATE conversions SET route = ?1
                 WHERE id = ?2 AND auth_scope = ?3 AND active_attempt_id = ?4",
            )
            .bind(&start.route)
            .bind(conversion_id.hyphenated().to_string())
            .bind(AUTH_SCOPE)
            .bind(attempt_id.hyphenated().to_string())
            .execute(&mut *transaction)
            .await?;
            require_one_transition_row(conversion.rows_affected(), "conversions.route")?;
        }
        let recorded = sqlx::query(
            "UPDATE attempts SET markdown_byte_length = ?1, markdown_sha256 = ?2
             WHERE conversion_id = ?3 AND id = ?4",
        )
        .bind(markdown_byte_length)
        .bind(markdown.1)
        .bind(conversion_id.hyphenated().to_string())
        .bind(attempt_id.hyphenated().to_string())
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(recorded.rows_affected(), "attempts.markdown")?;

        update_attempt_analysis(
            &mut transaction,
            ActiveIds {
                conversion_id,
                attempt_id,
            },
            AttemptState::ConvertingLocal,
            AttemptState::Finalizing,
            &analysis,
            None,
            &updated_at,
        )
        .await?;
        update_conversion_analysis(
            &mut transaction,
            conversion_id,
            attempt_id,
            ConversionState::ConvertingLocal,
            ConversionState::Finalizing,
            &analysis,
            &updated_at,
        )
        .await?;

        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::MarkFinalizing,
            conversion_id,
            attempt_id,
        )
        .await?;
        Ok(conversion)
    }

    /// An engine that gave up. The attempt keeps the analysis it reported, and
    /// both rows fail with the engine's reason as the code.
    pub async fn finish_gave_up(
        &self,
        conversion_id: Uuid,
        attempt_id: Uuid,
        analysis: LocalAnalysis,
        failure: StoredFailure,
    ) -> Result<StoredConversion, RepositoryError> {
        validate_failure(&failure)?;
        let analysis = encode_local_analysis(&analysis)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_active_state(
            &mut transaction,
            conversion_id,
            attempt_id,
            ConversionState::ConvertingLocal,
            AttemptState::ConvertingLocal,
        )
        .await?;
        let updated_at = now_rfc3339()?;

        update_attempt_analysis(
            &mut transaction,
            ActiveIds {
                conversion_id,
                attempt_id,
            },
            AttemptState::ConvertingLocal,
            AttemptState::ConvertingLocal,
            &analysis,
            Some(&failure.code),
            &updated_at,
        )
        .await?;
        update_conversion_analysis(
            &mut transaction,
            conversion_id,
            attempt_id,
            ConversionState::ConvertingLocal,
            ConversionState::ConvertingLocal,
            &analysis,
            &updated_at,
        )
        .await?;
        fail_rows(
            &mut transaction,
            ActiveIds {
                conversion_id,
                attempt_id,
            },
            FailureStage::ConvertingLocal,
            &failure.code,
            &failure.message,
        )
        .await?;

        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::FinishFailed,
            conversion_id,
            attempt_id,
        )
        .await?;
        Ok(conversion)
    }

    pub async fn finish_failed(
        &self,
        conversion_id: Uuid,
        attempt_id: Uuid,
        result: FailedResult,
    ) -> Result<StoredConversion, RepositoryError> {
        validate_failure(&result.failure)?;
        let expected_conversion = result.stage.conversion_state();
        let expected_attempt = result.stage.attempt_state();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_active_state(
            &mut transaction,
            conversion_id,
            attempt_id,
            expected_conversion,
            expected_attempt,
        )
        .await?;
        fail_rows(
            &mut transaction,
            ActiveIds {
                conversion_id,
                attempt_id,
            },
            result.stage,
            &result.failure.code,
            &result.failure.message,
        )
        .await?;

        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::FinishFailed,
            conversion_id,
            attempt_id,
        )
        .await?;
        Ok(conversion)
    }

    pub async fn finish_succeeded(
        &self,
        conversion_id: Uuid,
        attempt_id: Uuid,
        markdown: NewArtifact,
    ) -> Result<StoredConversion, RepositoryError> {
        let markdown_length = validate_artifact(conversion_id, attempt_id, &markdown)?;
        let conversion_id_text = conversion_id.hyphenated().to_string();
        let attempt_id_text = attempt_id.hyphenated().to_string();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_active_state(
            &mut transaction,
            conversion_id,
            attempt_id,
            ConversionState::Finalizing,
            AttemptState::Finalizing,
        )
        .await?;
        let updated_at = now_rfc3339()?;

        let existing_artifacts: i64 =
            sqlx::query("SELECT COUNT(*) AS artifact_count FROM artifacts WHERE attempt_id = ?1")
                .bind(&attempt_id_text)
                .fetch_one(&mut *transaction)
                .await?
                .try_get("artifact_count")?;
        if existing_artifacts != 0 {
            return Err(RepositoryError::ArtifactSetAlreadyExists {
                conversion_id,
                attempt_id,
            });
        }

        let inserted = sqlx::query(
            "INSERT INTO artifacts (
                attempt_id, kind, relative_path, media_type, byte_length,
                sha256, created_at
             ) VALUES (?1, 'markdown', ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&attempt_id_text)
        .bind(&markdown.relative_path)
        .bind(&markdown.media_type)
        .bind(markdown_length)
        .bind(&markdown.sha256)
        .bind(&updated_at)
        .execute(&mut *transaction)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(RepositoryError::CorruptData("artifacts.insert"));
        }

        let attempt = sqlx::query(
            "UPDATE attempts
             SET state = 'succeeded', updated_at = ?1, finished_at = ?1
             WHERE conversion_id = ?2 AND id = ?3 AND state = 'finalizing'",
        )
        .bind(&updated_at)
        .bind(&conversion_id_text)
        .bind(&attempt_id_text)
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(attempt.rows_affected(), "attempts.state")?;

        let conversion = sqlx::query(
            "UPDATE conversions
             SET status = 'succeeded', updated_at = ?1
             WHERE id = ?2 AND auth_scope = ?3 AND active_attempt_id = ?4
               AND status = 'finalizing'",
        )
        .bind(&updated_at)
        .bind(&conversion_id_text)
        .bind(AUTH_SCOPE)
        .bind(&attempt_id_text)
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(conversion.rows_affected(), "conversions.status")?;

        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::FinishSucceeded,
            conversion_id,
            attempt_id,
        )
        .await?;
        Ok(conversion)
    }

    pub async fn mark_artifact_integrity_failed(
        &self,
        conversion_id: Uuid,
        attempt_id: Uuid,
        failure: StoredFailure,
    ) -> Result<StoredConversion, RepositoryError> {
        validate_failure(&failure)?;
        if failure.code != "artifact_integrity_failed" {
            return Err(RepositoryError::InvalidInput(
                "artifact integrity failure code is invalid",
            ));
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        require_active_state(
            &mut transaction,
            conversion_id,
            attempt_id,
            ConversionState::Succeeded,
            AttemptState::Succeeded,
        )
        .await?;
        let updated_at = now_rfc3339()?;

        let conversion = sqlx::query(
            "UPDATE conversions
             SET status = 'failed', failure_code = ?1, failure_message = ?2,
                 updated_at = ?3
             WHERE id = ?4 AND auth_scope = ?5 AND active_attempt_id = ?6
               AND status = 'succeeded'",
        )
        .bind(&failure.code)
        .bind(&failure.message)
        .bind(&updated_at)
        .bind(conversion_id.hyphenated().to_string())
        .bind(AUTH_SCOPE)
        .bind(attempt_id.hyphenated().to_string())
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(conversion.rows_affected(), "conversions.status")?;

        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::MarkArtifactIntegrityFailed,
            conversion_id,
            attempt_id,
        )
        .await?;
        Ok(conversion)
    }

    /// Fail one conversion whose stored state startup recovery cannot trust.
    /// No state precondition on purpose: the stored state is the thing that
    /// does not make sense. Attempt rows are left for audit, as
    /// `mark_artifact_integrity_failed` leaves them.
    pub async fn quarantine_unrecoverable(
        &self,
        conversion_id: &str,
        failure: StoredFailure,
    ) -> Result<(), RepositoryError> {
        validate_failure(&failure)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let updated_at = now_rfc3339()?;
        let conversion = sqlx::query(
            "UPDATE conversions
             SET status = 'failed', failure_code = ?1, failure_message = ?2,
                 updated_at = ?3
             WHERE id = ?4 AND auth_scope = ?5 AND status <> 'failed'",
        )
        .bind(&failure.code)
        .bind(&failure.message)
        .bind(&updated_at)
        .bind(conversion_id)
        .bind(AUTH_SCOPE)
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(conversion.rows_affected(), "conversions.status")?;
        transaction
            .commit()
            .await
            .map_err(|source| RepositoryError::QuarantineCommitFailed {
                conversion_id: conversion_id.to_owned(),
                source,
            })
    }

    pub async fn interrupt_and_requeue(
        &self,
        conversion_id: Uuid,
        expected_attempt_id: Uuid,
        new_attempt_id: Uuid,
        recovery_limit: usize,
    ) -> Result<RequeueOutcome, RepositoryError> {
        if new_attempt_id == expected_attempt_id {
            return Err(RepositoryError::InvalidInput(
                "recovery attempt id must be new",
            ));
        }
        let recovery_limit = u32::try_from(recovery_limit)
            .map_err(|_| RepositoryError::InvalidInput("recovery limit is too large"))?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let active =
            load_active_attempt_header(&mut transaction, conversion_id, expected_attempt_id)
                .await?;
        let recoverable_state = match (active.conversion_state, active.attempt_state) {
            (ConversionState::ConvertingLocal, AttemptState::ConvertingLocal) => {
                ConversionState::ConvertingLocal
            }
            (ConversionState::Finalizing, AttemptState::Finalizing) => ConversionState::Finalizing,
            _ => {
                return Err(RepositoryError::IllegalRecoveryState {
                    conversion_id,
                    attempt_id: expected_attempt_id,
                    actual_conversion: active.conversion_state,
                    actual_attempt: active.attempt_state,
                });
            }
        };

        if active.highest_recovery_count >= recovery_limit {
            let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
            transaction.rollback().await?;
            return Ok(RequeueOutcome::LimitReached(conversion));
        }

        let next_attempt_number = active
            .highest_attempt_number
            .checked_add(1)
            .ok_or(RepositoryError::CorruptData("attempts.attempt_number"))?;
        let next_recovery_count = active
            .highest_recovery_count
            .checked_add(1)
            .ok_or(RepositoryError::CorruptData("attempts.recovery_count"))?;
        let updated_at = now_rfc3339()?;
        let conversion_id_text = conversion_id.hyphenated().to_string();
        let expected_attempt_id_text = expected_attempt_id.hyphenated().to_string();
        let new_attempt_id_text = new_attempt_id.hyphenated().to_string();

        let interrupted = sqlx::query(
            "UPDATE attempts
             SET state = 'interrupted', updated_at = ?1, finished_at = ?1
             WHERE conversion_id = ?2 AND id = ?3 AND state = ?4
               AND attempt_number = ?5 AND recovery_count = ?6",
        )
        .bind(&updated_at)
        .bind(&conversion_id_text)
        .bind(&expected_attempt_id_text)
        .bind(active.attempt_state)
        .bind(i64::from(active.attempt_number))
        .bind(i64::from(active.recovery_count))
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(interrupted.rows_affected(), "attempts.state")?;

        sqlx::query(
            "INSERT INTO attempts (
                id, conversion_id, attempt_number, state, recovery_count,
                reason_codes_json, warnings_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'queued', ?4, '[]', '[]', ?5, ?5)",
        )
        .bind(&new_attempt_id_text)
        .bind(&conversion_id_text)
        .bind(i64::from(next_attempt_number))
        .bind(i64::from(next_recovery_count))
        .bind(&updated_at)
        .execute(&mut *transaction)
        .await?;

        let requeued = sqlx::query(
            "UPDATE conversions
             SET active_attempt_id = ?1, status = 'queued', route = NULL,
                 reason_codes_json = '[]', warnings_json = '[]',
                 failure_code = NULL, failure_message = NULL, updated_at = ?2
             WHERE id = ?3 AND auth_scope = ?4 AND active_attempt_id = ?5
               AND status = ?6",
        )
        .bind(&new_attempt_id_text)
        .bind(&updated_at)
        .bind(&conversion_id_text)
        .bind(AUTH_SCOPE)
        .bind(&expected_attempt_id_text)
        .bind(recoverable_state)
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(requeued.rows_affected(), "conversions.status")?;

        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::InterruptAndRequeue,
            conversion_id,
            new_attempt_id,
        )
        .await?;
        Ok(RequeueOutcome::Requeued(conversion))
    }

    /// List what startup recovery must reconcile.
    ///
    /// **Each row decodes independently.** A row whose id, JSON, or enum will
    /// not parse comes back as `Undecodable` rather than aborting the listing.
    /// Containing per-job reconciliation errors is worthless if one malformed
    /// column can still stop the boot before reconciliation begins.
    pub async fn list_recovery_candidates(
        &self,
    ) -> Result<Vec<RecoveryCandidate>, RepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT c.id
             FROM conversions AS c
             JOIN attempts AS a
               ON a.conversion_id = c.id AND a.id = c.active_attempt_id
             WHERE c.auth_scope = ?1
               AND c.status IN (
                   'queued', 'converting_local', 'finalizing', 'succeeded'
               )
             ORDER BY a.queue_seq ASC",
        )
        .bind(AUTH_SCOPE)
        .fetch_all(&mut *transaction)
        .await?;
        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows {
            let raw_id: String = match row.try_get("id") {
                Ok(value) => value,
                Err(error) => {
                    // No id means no way to name the row. Nothing can claim it
                    // either, so record it and move on.
                    tracing::error!(%error, "a conversion row has no readable id");
                    continue;
                }
            };
            let Ok(conversion_id) = Uuid::parse_str(&raw_id) else {
                candidates.push(RecoveryCandidate::Undecodable {
                    id: raw_id,
                    reason: "conversions.id is not a UUID".to_owned(),
                });
                continue;
            };
            match load_required_conversion(&mut transaction, conversion_id).await {
                Ok(conversion) => candidates.push(RecoveryCandidate::Loaded(Box::new(conversion))),
                // A decode failure is this row's problem, so quarantine it and
                // keep booting.
                Err(error) if error.is_row_shape() => {
                    candidates.push(RecoveryCandidate::Undecodable {
                        id: raw_id,
                        reason: error.to_string(),
                    });
                }
                // A busy pool, an IO blip, or a timeout says nothing about this
                // row. Quarantine is terminal and strands the source and the
                // artifacts, so stop the boot rather than destroy a healthy job
                // over a database that was briefly unreadable.
                Err(error) => return Err(error),
            }
        }
        transaction.commit().await?;
        Ok(candidates)
    }

    /// Ids that own storage. An unparseable id names no directory, so it is
    /// skipped rather than aborting orphan quarantine.
    pub async fn list_conversion_ids(&self) -> Result<HashSet<Uuid>, RepositoryError> {
        let rows = sqlx::query("SELECT id FROM conversions WHERE auth_scope = ?1")
            .bind(AUTH_SCOPE)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let raw: String = row.try_get("id").ok()?;
                Uuid::parse_str(&raw).ok()
            })
            .collect())
    }

    pub async fn health_check(&self) -> Result<(), RepositoryError> {
        probe_database(&self.pool).await
    }

    pub async fn get(
        &self,
        conversion_id: Uuid,
    ) -> Result<Option<StoredConversion>, RepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let conversion = load_conversion(&mut transaction, conversion_id).await?;
        transaction.commit().await?;
        Ok(conversion)
    }

    pub async fn accepting_jobs(&self) -> Result<bool, RepositoryError> {
        let active_count: i64 = sqlx::query(ACTIVE_COUNT_SQL)
            .fetch_one(&self.pool)
            .await?
            .try_get("active_count")?;
        Ok(active_count < i64::from(self.max_active_jobs))
    }
}

#[derive(Debug)]
struct EncodedLocalAnalysis {
    classification: DocumentClassification,
    inspection_json: String,
    reason_codes_json: String,
    warnings_json: String,
}

#[derive(Clone, Copy, Debug)]
struct ActiveIds {
    conversion_id: Uuid,
    attempt_id: Uuid,
}

#[derive(Clone, Copy, Debug)]
struct ActiveAttemptHeader {
    conversion_state: ConversionState,
    attempt_state: AttemptState,
    attempt_number: u32,
    recovery_count: u32,
    highest_attempt_number: u32,
    highest_recovery_count: u32,
}

/// Fails a queued conversion in place when no build of this service has an
/// engine for its source media type. Defense in depth: upload validation keeps
/// such rows out, so this only fires on corrupted or hand-edited data. A row
/// whose engine is merely absent on this boot never reaches here.
async fn fail_unclaimable_source(
    transaction: &mut Transaction<'_, Sqlite>,
    conversion_id: Uuid,
    attempt_id: Uuid,
) -> Result<(), RepositoryError> {
    fail_rows(
        transaction,
        ActiveIds {
            conversion_id,
            attempt_id,
        },
        FailureStage::Queued,
        "unsupported_source_media_type",
        "No local engine handles this source media type.",
    )
    .await
}

/// Writes the failure to both rows, guarded on the state the caller expects.
async fn fail_rows(
    transaction: &mut Transaction<'_, Sqlite>,
    ids: ActiveIds,
    expected: FailureStage,
    code: &str,
    message: &str,
) -> Result<(), RepositoryError> {
    let updated_at = now_rfc3339()?;
    let conversion_id = ids.conversion_id.hyphenated().to_string();
    let attempt_id = ids.attempt_id.hyphenated().to_string();
    let attempt = sqlx::query(
        "UPDATE attempts
         SET state = 'failed', failure_code = ?1, failure_message = ?2,
             updated_at = ?3, finished_at = ?3
         WHERE conversion_id = ?4 AND id = ?5 AND state = ?6",
    )
    .bind(code)
    .bind(message)
    .bind(&updated_at)
    .bind(&conversion_id)
    .bind(&attempt_id)
    .bind(expected.attempt_state())
    .execute(&mut **transaction)
    .await?;
    require_one_transition_row(attempt.rows_affected(), "attempts.state")?;

    let conversion = sqlx::query(
        "UPDATE conversions
         SET status = 'failed', failure_code = ?1, failure_message = ?2,
             updated_at = ?3
         WHERE id = ?4 AND auth_scope = ?5 AND active_attempt_id = ?6
           AND status = ?7",
    )
    .bind(code)
    .bind(message)
    .bind(&updated_at)
    .bind(&conversion_id)
    .bind(AUTH_SCOPE)
    .bind(&attempt_id)
    .bind(expected.conversion_state())
    .execute(&mut **transaction)
    .await?;
    require_one_transition_row(conversion.rows_affected(), "conversions.status")?;
    Ok(())
}

async fn apply_start_local(
    transaction: &mut Transaction<'_, Sqlite>,
    conversion_id: Uuid,
    attempt_id: Uuid,
    start: &LocalStart,
) -> Result<(), RepositoryError> {
    require_active_state(
        transaction,
        conversion_id,
        attempt_id,
        ConversionState::Queued,
        AttemptState::Queued,
    )
    .await?;
    let updated_at = now_rfc3339()?;

    let conversion_id = conversion_id.hyphenated().to_string();
    let attempt_id = attempt_id.hyphenated().to_string();
    let attempt = sqlx::query(
        "UPDATE attempts
         SET state = 'converting_local', engine_name = ?1, engine_version = ?2,
             route = ?3, updated_at = ?4, started_at = ?4
         WHERE conversion_id = ?5 AND id = ?6 AND state = 'queued'",
    )
    .bind(&start.engine.name)
    .bind(&start.engine.version)
    .bind(&start.route)
    .bind(&updated_at)
    .bind(&conversion_id)
    .bind(&attempt_id)
    .execute(&mut **transaction)
    .await?;
    require_one_transition_row(attempt.rows_affected(), "attempts.state")?;

    let conversion = sqlx::query(
        "UPDATE conversions
         SET status = 'converting_local', route = ?1, updated_at = ?2
         WHERE id = ?3 AND auth_scope = ?4 AND active_attempt_id = ?5
           AND status = 'queued'",
    )
    .bind(&start.route)
    .bind(&updated_at)
    .bind(&conversion_id)
    .bind(AUTH_SCOPE)
    .bind(&attempt_id)
    .execute(&mut **transaction)
    .await?;
    require_one_transition_row(conversion.rows_affected(), "conversions.status")
}

async fn require_active_state(
    transaction: &mut Transaction<'_, Sqlite>,
    conversion_id: Uuid,
    attempt_id: Uuid,
    expected_conversion: ConversionState,
    expected_attempt: AttemptState,
) -> Result<(), RepositoryError> {
    let active = load_active_attempt_header(transaction, conversion_id, attempt_id).await?;
    if active.conversion_state != expected_conversion || active.attempt_state != expected_attempt {
        return Err(RepositoryError::IllegalTransition {
            conversion_id,
            attempt_id,
            expected_conversion,
            expected_attempt,
            actual_conversion: active.conversion_state,
            actual_attempt: active.attempt_state,
        });
    }
    Ok(())
}

async fn load_active_attempt_header(
    transaction: &mut Transaction<'_, Sqlite>,
    conversion_id: Uuid,
    attempt_id: Uuid,
) -> Result<ActiveAttemptHeader, RepositoryError> {
    let row = sqlx::query(
        "SELECT c.active_attempt_id, c.status, a.state AS attempt_state,
                a.attempt_number, a.recovery_count,
                (SELECT MAX(history.attempt_number)
                 FROM attempts AS history WHERE history.conversion_id = c.id)
                    AS highest_attempt_number,
                (SELECT MAX(history.recovery_count)
                 FROM attempts AS history WHERE history.conversion_id = c.id)
                    AS highest_recovery_count
         FROM conversions AS c
         LEFT JOIN attempts AS a
           ON a.conversion_id = c.id AND a.id = c.active_attempt_id
         WHERE c.auth_scope = ?1 AND c.id = ?2",
    )
    .bind(AUTH_SCOPE)
    .bind(conversion_id.hyphenated().to_string())
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else {
        return Err(RepositoryError::ConversionNotFound { conversion_id });
    };

    let active_attempt = parse_uuid(
        row.try_get::<Option<String>, _>("active_attempt_id")?
            .ok_or(RepositoryError::CorruptData(
                "conversions.active_attempt_id",
            ))?,
        "conversions.active_attempt_id",
    )?;
    if active_attempt != attempt_id {
        return Err(RepositoryError::StaleActiveAttempt {
            conversion_id,
            attempt_id,
            active_attempt_id: active_attempt,
        });
    }

    let conversion_state: ConversionState = row.try_get("status")?;
    let attempt_state = row
        .try_get::<Option<AttemptState>, _>("attempt_state")?
        .ok_or(RepositoryError::CorruptData("attempts.state"))?;
    let attempt_number = nonnegative_u32(
        row.try_get::<Option<i64>, _>("attempt_number")?
            .ok_or(RepositoryError::CorruptData("attempts.attempt_number"))?,
        "attempts.attempt_number",
    )?;
    let recovery_count = nonnegative_u32(
        row.try_get::<Option<i64>, _>("recovery_count")?
            .ok_or(RepositoryError::CorruptData("attempts.recovery_count"))?,
        "attempts.recovery_count",
    )?;
    let highest_attempt_number = nonnegative_u32(
        row.try_get::<Option<i64>, _>("highest_attempt_number")?
            .ok_or(RepositoryError::CorruptData("attempts.attempt_number"))?,
        "attempts.attempt_number",
    )?;
    let highest_recovery_count = nonnegative_u32(
        row.try_get::<Option<i64>, _>("highest_recovery_count")?
            .ok_or(RepositoryError::CorruptData("attempts.recovery_count"))?,
        "attempts.recovery_count",
    )?;
    Ok(ActiveAttemptHeader {
        conversion_state,
        attempt_state,
        attempt_number,
        recovery_count,
        highest_attempt_number,
        highest_recovery_count,
    })
}

async fn update_attempt_analysis(
    transaction: &mut Transaction<'_, Sqlite>,
    ids: ActiveIds,
    expected_state: AttemptState,
    target_state: AttemptState,
    analysis: &EncodedLocalAnalysis,
    fallback_reason: Option<&str>,
    updated_at: &str,
) -> Result<(), RepositoryError> {
    let conversion_id = ids.conversion_id.hyphenated().to_string();
    let attempt_id = ids.attempt_id.hyphenated().to_string();
    let result = if let Some(fallback_reason) = fallback_reason {
        sqlx::query(
            "UPDATE attempts
             SET state = ?1, classification = ?2, inspection_json = ?3,
                 reason_codes_json = ?4, warnings_json = ?5,
                 fallback_reason = ?6, updated_at = ?7, finished_at = ?7
             WHERE conversion_id = ?8 AND id = ?9 AND state = ?10",
        )
        .bind(target_state)
        .bind(analysis.classification)
        .bind(&analysis.inspection_json)
        .bind(&analysis.reason_codes_json)
        .bind(&analysis.warnings_json)
        .bind(fallback_reason)
        .bind(updated_at)
        .bind(&conversion_id)
        .bind(&attempt_id)
        .bind(expected_state)
        .execute(&mut **transaction)
        .await?
    } else {
        sqlx::query(
            "UPDATE attempts
             SET state = ?1, classification = ?2, inspection_json = ?3,
                 reason_codes_json = ?4, warnings_json = ?5, updated_at = ?6
             WHERE conversion_id = ?7 AND id = ?8 AND state = ?9",
        )
        .bind(target_state)
        .bind(analysis.classification)
        .bind(&analysis.inspection_json)
        .bind(&analysis.reason_codes_json)
        .bind(&analysis.warnings_json)
        .bind(updated_at)
        .bind(&conversion_id)
        .bind(&attempt_id)
        .bind(expected_state)
        .execute(&mut **transaction)
        .await?
    };
    require_one_transition_row(result.rows_affected(), "attempts.state")
}

async fn update_conversion_analysis(
    transaction: &mut Transaction<'_, Sqlite>,
    conversion_id: Uuid,
    attempt_id: Uuid,
    expected_state: ConversionState,
    target_state: ConversionState,
    analysis: &EncodedLocalAnalysis,
    updated_at: &str,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE conversions
         SET status = ?1, reason_codes_json = ?2, warnings_json = ?3,
             updated_at = ?4
         WHERE id = ?5 AND auth_scope = ?6 AND active_attempt_id = ?7
           AND status = ?8",
    )
    .bind(target_state)
    .bind(&analysis.reason_codes_json)
    .bind(&analysis.warnings_json)
    .bind(updated_at)
    .bind(conversion_id.hyphenated().to_string())
    .bind(AUTH_SCOPE)
    .bind(attempt_id.hyphenated().to_string())
    .bind(expected_state)
    .execute(&mut **transaction)
    .await?;
    require_one_transition_row(result.rows_affected(), "conversions.status")
}

async fn load_required_conversion(
    transaction: &mut Transaction<'_, Sqlite>,
    conversion_id: Uuid,
) -> Result<StoredConversion, RepositoryError> {
    load_conversion(transaction, conversion_id)
        .await?
        .ok_or(RepositoryError::ConversionNotFound { conversion_id })
}

async fn commit_transition(
    transaction: Transaction<'_, Sqlite>,
    operation: CommitOperation,
    conversion_id: Uuid,
    attempt_id: Uuid,
) -> Result<(), RepositoryError> {
    transaction
        .commit()
        .await
        .map_err(|source| RepositoryError::CommitOutcomeUncertain {
            operation,
            conversion_id,
            attempt_id,
            source,
        })
}

fn require_one_transition_row(
    rows_affected: u64,
    field: &'static str,
) -> Result<(), RepositoryError> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(RepositoryError::CorruptData(field))
    }
}

fn validate_local_start(start: &LocalStart) -> Result<(), RepositoryError> {
    validate_bounded_text(&start.engine.name, 128, "engine name is invalid")?;
    validate_bounded_text(&start.engine.version, 128, "engine version is invalid")?;
    validate_bounded_text(&start.route, 128, "route is invalid")
}

fn encode_local_analysis(
    analysis: &LocalAnalysis,
) -> Result<EncodedLocalAnalysis, RepositoryError> {
    if !analysis.inspection.is_object() {
        return Err(RepositoryError::InvalidInput(
            "inspection must be a JSON object",
        ));
    }
    let inspection_json = serde_json::to_string(&analysis.inspection)
        .map_err(|_| RepositoryError::InvalidInput("inspection JSON is invalid"))?;
    if inspection_json.len() > 1_048_576 {
        return Err(RepositoryError::InvalidInput(
            "inspection JSON is too large",
        ));
    }
    let reason_codes_json = encode_string_array(
        &analysis.reason_codes,
        1,
        128,
        8_192,
        "reason codes are invalid",
    )?;
    let warnings_json =
        encode_string_array(&analysis.warnings, 0, 1_024, 65_536, "warnings are invalid")?;
    Ok(EncodedLocalAnalysis {
        classification: analysis.classification,
        inspection_json,
        reason_codes_json,
        warnings_json,
    })
}

fn encode_string_array(
    values: &[String],
    minimum_items: usize,
    maximum_item_length: usize,
    maximum_encoded_length: usize,
    message: &'static str,
) -> Result<String, RepositoryError> {
    if values.len() < minimum_items
        || values.len() > 256
        || values
            .iter()
            .any(|value| validate_bounded_text(value, maximum_item_length, message).is_err())
    {
        return Err(RepositoryError::InvalidInput(message));
    }
    let encoded =
        serde_json::to_string(values).map_err(|_| RepositoryError::InvalidInput(message))?;
    if encoded.len() > maximum_encoded_length {
        return Err(RepositoryError::InvalidInput(message));
    }
    Ok(encoded)
}

fn validate_failure(failure: &StoredFailure) -> Result<(), RepositoryError> {
    validate_bounded_text(&failure.code, 128, "failure code is invalid")?;
    validate_bounded_text(&failure.message, 1_024, "failure message is invalid")
}

fn validate_artifact(
    conversion_id: Uuid,
    attempt_id: Uuid,
    artifact: &NewArtifact,
) -> Result<i64, RepositoryError> {
    let expected_path = format!(
        "jobs/{}/attempts/{}/artifacts/result.md",
        conversion_id.hyphenated(),
        attempt_id.hyphenated()
    );
    if artifact.relative_path != expected_path {
        return Err(RepositoryError::InvalidInput(
            "artifact path does not belong to the active attempt",
        ));
    }
    if artifact.media_type != "text/markdown; charset=utf-8" {
        return Err(RepositoryError::InvalidInput(
            "artifact media type is invalid",
        ));
    }
    validate_sha256(&artifact.sha256, "artifact hash is invalid")?;
    match i64::try_from(artifact.byte_length) {
        Ok(length) if length > 0 => Ok(length),
        _ => Err(RepositoryError::InvalidInput(
            "artifact byte length is invalid",
        )),
    }
}

fn validate_bounded_text(
    value: &str,
    maximum_length: usize,
    message: &'static str,
) -> Result<(), RepositoryError> {
    if value.is_empty() || value.len() > maximum_length || value.chars().any(char::is_control) {
        return Err(RepositoryError::InvalidInput(message));
    }
    Ok(())
}

pub fn hash_idempotency_key(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

async fn probe_database(pool: &SqlitePool) -> Result<(), RepositoryError> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("UPDATE conversions SET updated_at = updated_at WHERE 0")
        .execute(&mut *transaction)
        .await?;
    let _: i64 = sqlx::query("SELECT COUNT(*) AS conversion_count FROM conversions")
        .fetch_one(&mut *transaction)
        .await?
        .try_get("conversion_count")?;
    transaction.rollback().await?;
    Ok(())
}

async fn load_conversion(
    connection: &mut SqliteConnection,
    conversion_id: Uuid,
) -> Result<Option<StoredConversion>, RepositoryError> {
    let row = sqlx::query(
        "SELECT
            c.id AS conversion_id,
            c.client_run_id,
            c.status AS conversion_state,
            c.source_relative_path,
            c.source_media_type,
            c.source_byte_length,
            c.source_sha256,
            c.route AS conversion_route,
            c.reason_codes_json AS conversion_reason_codes_json,
            c.warnings_json AS conversion_warnings_json,
            c.failure_code AS conversion_failure_code,
            c.failure_message AS conversion_failure_message,
            c.origin_request_id,
            c.created_at AS conversion_created_at,
            c.updated_at AS conversion_updated_at,
            c.ocr_language_correction,
            c.ocr_custom_words,
            c.speaker_count,
            c.speech_locale,
            a.id AS attempt_id,
            a.queue_seq,
            a.attempt_number,
            a.state AS attempt_state,
            a.recovery_count,
            a.engine_name,
            a.engine_version,
            a.route AS attempt_route,
            a.classification,
            a.inspection_json,
            a.reason_codes_json AS attempt_reason_codes_json,
            a.warnings_json AS attempt_warnings_json,
            a.fallback_reason,
            a.failure_code AS attempt_failure_code,
            a.failure_message AS attempt_failure_message,
            a.markdown_byte_length,
            a.markdown_sha256,
            a.created_at AS attempt_created_at,
            a.updated_at AS attempt_updated_at,
            a.started_at,
            a.finished_at
         FROM conversions AS c
         JOIN attempts AS a
           ON a.conversion_id = c.id
          AND a.id = c.active_attempt_id
         WHERE c.auth_scope = ?1 AND c.id = ?2",
    )
    .bind(AUTH_SCOPE)
    .bind(conversion_id.hyphenated().to_string())
    .fetch_optional(&mut *connection)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let mut conversion = decode_conversion(&row)?;
    conversion.artifacts = load_artifacts(connection, conversion.active_attempt.id).await?;
    Ok(Some(conversion))
}

async fn load_artifacts(
    connection: &mut SqliteConnection,
    attempt_id: Uuid,
) -> Result<Vec<StoredArtifact>, RepositoryError> {
    let rows = sqlx::query(
        "SELECT attempt_id, relative_path, media_type, byte_length, sha256, created_at
         FROM artifacts
         WHERE attempt_id = ?1",
    )
    .bind(attempt_id.hyphenated().to_string())
    .fetch_all(&mut *connection)
    .await?;
    rows.iter().map(decode_artifact).collect()
}

fn validate_new_conversion(input: &NewConversion) -> Result<(), RepositoryError> {
    validate_sha256(
        &input.idempotency_key_sha256,
        "idempotency key hash is invalid",
    )?;
    validate_sha256(&input.request_fingerprint, "request fingerprint is invalid")?;
    validate_sha256(&input.source.sha256, "source hash is invalid")?;

    let expected_source_path = format!("jobs/{}/source/input", input.id.hyphenated());
    if input.source.relative_path != expected_source_path {
        return Err(RepositoryError::InvalidInput(
            "source path does not belong to the conversion",
        ));
    }
    // The media-type policy lives in the conversion domain's source-format
    // table; the ledger keeps storage invariants only. Claim-time engine
    // resolution fails closed on any media type with no engine.
    validate_bounded_text(
        &input.source.media_type,
        127,
        "source media type is invalid",
    )?;
    if input.source.byte_length == 0 || input.source.byte_length > i64::MAX as u64 {
        return Err(RepositoryError::InvalidInput(
            "source byte length is invalid",
        ));
    }
    validate_bounded_text(
        &input.origin_request_id,
        128,
        "origin request id is invalid",
    )
}

fn validate_sha256(value: &str, message: &'static str) -> Result<(), RepositoryError> {
    if !is_lowercase_sha256(value) {
        return Err(RepositoryError::InvalidInput(message));
    }
    Ok(())
}

fn decode_conversion(row: &SqliteRow) -> Result<StoredConversion, RepositoryError> {
    let conversion_failure = decode_failure(
        row.try_get("conversion_failure_code")?,
        row.try_get("conversion_failure_message")?,
        "conversions.failure",
    )?;
    let attempt_failure = decode_failure(
        row.try_get("attempt_failure_code")?,
        row.try_get("attempt_failure_message")?,
        "attempts.failure",
    )?;
    let engine = decode_engine(row.try_get("engine_name")?, row.try_get("engine_version")?)?;
    let inspection = row
        .try_get::<Option<String>, _>("inspection_json")?
        .map(|encoded| {
            serde_json::from_str(&encoded)
                .map_err(|_| RepositoryError::CorruptData("attempts.inspection_json"))
        })
        .transpose()?;

    Ok(StoredConversion {
        id: parse_uuid(row.try_get("conversion_id")?, "conversions.id")?,
        client_run_id: parse_uuid(row.try_get("client_run_id")?, "conversions.client_run_id")?,
        state: row.try_get("conversion_state")?,
        source: StoredSource {
            relative_path: row.try_get("source_relative_path")?,
            media_type: row.try_get("source_media_type")?,
            byte_length: positive_u64(
                row.try_get("source_byte_length")?,
                "conversions.source_byte_length",
            )?,
            sha256: row.try_get("source_sha256")?,
        },
        active_attempt: StoredAttempt {
            id: parse_uuid(row.try_get("attempt_id")?, "attempts.id")?,
            number: nonnegative_u32(row.try_get("attempt_number")?, "attempts.attempt_number")?,
            queue_sequence: row.try_get("queue_seq")?,
            state: row.try_get("attempt_state")?,
            recovery_count: nonnegative_u32(
                row.try_get("recovery_count")?,
                "attempts.recovery_count",
            )?,
            engine,
            route: row.try_get("attempt_route")?,
            classification: row.try_get("classification")?,
            inspection,
            reason_codes: decode_string_array(
                row.try_get("attempt_reason_codes_json")?,
                "attempts.reason_codes_json",
            )?,
            warnings: decode_string_array(
                row.try_get("attempt_warnings_json")?,
                "attempts.warnings_json",
            )?,
            fallback_reason: row.try_get("fallback_reason")?,
            failure: attempt_failure,
            markdown_byte_length: row
                .try_get::<Option<i64>, _>("markdown_byte_length")?
                .map(|length| positive_u64(length, "attempts.markdown_byte_length"))
                .transpose()?,
            markdown_sha256: row.try_get("markdown_sha256")?,
            created_at: row.try_get("attempt_created_at")?,
            updated_at: row.try_get("attempt_updated_at")?,
            started_at: row.try_get("started_at")?,
            finished_at: row.try_get("finished_at")?,
        },
        route: row.try_get("conversion_route")?,
        reason_codes: decode_string_array(
            row.try_get("conversion_reason_codes_json")?,
            "conversions.reason_codes_json",
        )?,
        warnings: decode_string_array(
            row.try_get("conversion_warnings_json")?,
            "conversions.warnings_json",
        )?,
        failure: conversion_failure,
        artifacts: Vec::new(),
        origin_request_id: row.try_get("origin_request_id")?,
        created_at: row.try_get("conversion_created_at")?,
        updated_at: row.try_get("conversion_updated_at")?,
        ocr_language_correction: row.try_get("ocr_language_correction")?,
        ocr_custom_words: row.try_get("ocr_custom_words")?,
        speaker_count: row
            .try_get::<Option<i64>, _>("speaker_count")?
            .map(|count| nonnegative_u32(count, "conversions.speaker_count"))
            .transpose()?,
        speech_locale: row.try_get("speech_locale")?,
    })
}

fn decode_artifact(row: &SqliteRow) -> Result<StoredArtifact, RepositoryError> {
    Ok(StoredArtifact {
        attempt_id: parse_uuid(row.try_get("attempt_id")?, "artifacts.attempt_id")?,
        relative_path: row.try_get("relative_path")?,
        media_type: row.try_get("media_type")?,
        byte_length: positive_u64(row.try_get("byte_length")?, "artifacts.byte_length")?,
        sha256: row.try_get("sha256")?,
        created_at: row.try_get("created_at")?,
    })
}

fn decode_failure(
    code: Option<String>,
    message: Option<String>,
    field: &'static str,
) -> Result<Option<StoredFailure>, RepositoryError> {
    match (code, message) {
        (None, None) => Ok(None),
        (Some(code), Some(message)) => Ok(Some(StoredFailure { code, message })),
        _ => Err(RepositoryError::CorruptData(field)),
    }
}

fn decode_engine(
    name: Option<String>,
    version: Option<String>,
) -> Result<Option<EngineRecord>, RepositoryError> {
    match (name, version) {
        (None, None) => Ok(None),
        (Some(name), Some(version)) => Ok(Some(EngineRecord { name, version })),
        _ => Err(RepositoryError::CorruptData("attempts.engine")),
    }
}

fn decode_string_array(
    encoded: String,
    field: &'static str,
) -> Result<Vec<String>, RepositoryError> {
    serde_json::from_str(&encoded).map_err(|_| RepositoryError::CorruptData(field))
}

fn parse_uuid(value: String, field: &'static str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(&value).map_err(|_| RepositoryError::CorruptData(field))
}

fn positive_u64(value: i64, field: &'static str) -> Result<u64, RepositoryError> {
    if value <= 0 {
        return Err(RepositoryError::CorruptData(field));
    }
    u64::try_from(value).map_err(|_| RepositoryError::CorruptData(field))
}

fn nonnegative_u32(value: i64, field: &'static str) -> Result<u32, RepositoryError> {
    u32::try_from(value).map_err(|_| RepositoryError::CorruptData(field))
}

fn now_rfc3339() -> Result<String, RepositoryError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(RepositoryError::Timestamp)
}

impl RepositoryError {
    /// True when the error describes one row's stored shape, not the health of
    /// the database.
    ///
    /// Startup recovery quarantines a row it cannot decode, and quarantine is
    /// terminal: the job never becomes a candidate again and its source and
    /// artifacts are stranded. So only a shape problem may take that path. A
    /// busy pool, an IO error, or a timeout is "unreadable now", which is not
    /// proof the row is wrong.
    pub fn is_row_shape(&self) -> bool {
        match self {
            Self::CorruptData(_) | Self::ConversionNotFound { .. } => true,
            Self::Database(error) => matches!(
                error,
                sqlx::Error::ColumnDecode { .. }
                    | sqlx::Error::ColumnNotFound(_)
                    | sqlx::Error::ColumnIndexOutOfBounds { .. }
                    | sqlx::Error::Decode(_)
                    | sqlx::Error::RowNotFound
            ),
            _ => false,
        }
    }
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("data directory must be an absolute path")]
    DataDirectoryMustBeAbsolute,
    #[error("cannot access the data directory")]
    DataDirectory(#[source] std::io::Error),
    #[error("data directory path is not a directory")]
    DataDirectoryNotDirectory,
    #[error("active job limit must be greater than zero")]
    InvalidActiveJobLimit,
    #[error("invalid conversion input: {0}")]
    InvalidInput(&'static str),
    #[error("database contains invalid data in {0}")]
    CorruptData(&'static str),
    #[error("conversion {conversion_id} does not exist")]
    ConversionNotFound { conversion_id: Uuid },
    #[error(
        "attempt {attempt_id} is stale for conversion {conversion_id}; active attempt is {active_attempt_id}"
    )]
    StaleActiveAttempt {
        conversion_id: Uuid,
        attempt_id: Uuid,
        active_attempt_id: Uuid,
    },
    #[error(
        "illegal transition for conversion {conversion_id} attempt {attempt_id}: expected {expected_conversion:?}/{expected_attempt:?}, found {actual_conversion:?}/{actual_attempt:?}"
    )]
    IllegalTransition {
        conversion_id: Uuid,
        attempt_id: Uuid,
        expected_conversion: ConversionState,
        expected_attempt: AttemptState,
        actual_conversion: ConversionState,
        actual_attempt: AttemptState,
    },
    #[error(
        "conversion {conversion_id} attempt {attempt_id} cannot be recovered from {actual_conversion:?}/{actual_attempt:?}"
    )]
    IllegalRecoveryState {
        conversion_id: Uuid,
        attempt_id: Uuid,
        actual_conversion: ConversionState,
        actual_attempt: AttemptState,
    },
    #[error(
        "artifact metadata already exists for conversion {conversion_id} attempt {attempt_id}"
    )]
    ArtifactSetAlreadyExists {
        conversion_id: Uuid,
        attempt_id: Uuid,
    },
    #[error("a committed conversion could not be read back")]
    MissingCommittedConversion,
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    /// SQLite may report an I/O error while committing without giving the
    /// caller enough information to decide whether durable ownership changed.
    /// Reconcile creation by idempotency key and transitions by conversion and
    /// active-attempt IDs. Do not clean owned files or begin execution until
    /// the persisted outcome is known.
    #[error(
        "database commit outcome for {operation:?} on conversion {conversion_id} attempt {attempt_id} is uncertain; reconcile before cleanup or execution"
    )]
    CommitOutcomeUncertain {
        operation: CommitOperation,
        conversion_id: Uuid,
        attempt_id: Uuid,
        #[source]
        source: sqlx::Error,
    },
    #[error("database migration failed")]
    Migration(#[from] MigrateError),
    #[error("cannot create a timestamp")]
    Timestamp(#[source] time::error::Format),
    #[error("quarantine of conversion {conversion_id} did not commit")]
    QuarantineCommitFailed {
        conversion_id: String,
        #[source]
        source: sqlx::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc, time::Duration};

    use sqlx::Row;
    use tempfile::TempDir;
    use tokio::sync::Barrier;
    use uuid::Uuid;

    use super::{
        hash_idempotency_key, RepositoryError, SqliteConnection, SqliteRepository, AUTH_SCOPE,
        DATABASE_FILENAME, MAX_POOL_CONNECTIONS,
    };
    use crate::persistence::{
        AttemptState, ConversionState, CreateOutcome, DocumentClassification, EngineRecord,
        FailedResult, FailureStage, LocalAnalysis, LocalStart, NewArtifact, NewConversion,
        NewSource, RecoveryCandidate, RequeueOutcome, StoredFailure,
    };

    const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
    const MARKDOWN: (u64, &str) = (
        32,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );

    /// Startup quarantine is terminal, so this split decides whether a healthy
    /// job survives a database blip. Getting it backwards either strands good
    /// jobs or resurrects the boot crash loop, so pin both directions.
    #[test]
    fn only_a_row_shape_error_may_be_quarantined_at_startup() {
        // This row is wrong. Quarantine is correct.
        assert!(RepositoryError::CorruptData("conversions.state").is_row_shape());
        assert!(RepositoryError::ConversionNotFound {
            conversion_id: Uuid::nil(),
        }
        .is_row_shape());
        assert!(RepositoryError::Database(sqlx::Error::RowNotFound).is_row_shape());
        assert!(
            RepositoryError::Database(sqlx::Error::ColumnNotFound("source_media_type".into()))
                .is_row_shape()
        );

        // The database is unhappy. This says nothing about the row, so the boot
        // must stop instead of failing a job that may be perfectly fine.
        assert!(!RepositoryError::Database(sqlx::Error::PoolTimedOut).is_row_shape());
        assert!(!RepositoryError::Database(sqlx::Error::PoolClosed).is_row_shape());
        assert!(
            !RepositoryError::Database(sqlx::Error::Io(std::io::Error::other("disk")))
                .is_row_shape()
        );
        assert!(!RepositoryError::MissingCommittedConversion.is_row_shape());
    }

    /// `-- no-transaction` runs the file in autocommit, so a rebuild that
    /// drops before it renames has a window where neither table exists under
    /// the real name and `_sqlx_migrations` is unwritten. The next boot
    /// re-runs the file and dies on "table already exists". The pragma stays
    /// outside the pair, where it is not a no-op.
    #[test]
    fn every_no_transaction_migration_wraps_its_rebuild_in_one_transaction() {
        let mut checked = 0;
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations")).unwrap()
        {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            if path.extension().and_then(|value| value.to_str()) != Some("sql") {
                continue;
            }
            let sql = std::fs::read_to_string(&path).unwrap();
            if !sql.starts_with("-- no-transaction") {
                continue;
            }
            checked += 1;
            // Comments quote these keywords, so judge statements only.
            let statements: String = sql
                .lines()
                .filter(|line| !line.trim_start().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(
                statements.matches("\nBEGIN;").count(),
                1,
                "{name} runs outside a sqlx transaction and must open exactly one of its own"
            );
            assert_eq!(
                statements.matches("\nCOMMIT;").count(),
                1,
                "{name} opens a transaction it never commits"
            );
            let begin = statements.find("\nBEGIN;").unwrap();
            let commit = statements.find("\nCOMMIT;").unwrap();
            assert!(begin < commit, "{name} commits before it begins");
            for (index, _) in statements.match_indices("DROP TABLE") {
                assert!(
                    index > begin && index < commit,
                    "{name} drops a table outside its transaction"
                );
            }
            assert!(
                statements
                    .find("PRAGMA foreign_keys = OFF")
                    .is_none_or(|pragma| pragma < begin),
                "{name} disables foreign keys inside a transaction, where the pragma is a no-op"
            );
        }
        assert!(checked >= 2, "migrations were not read");
    }

    #[tokio::test]
    async fn migrations_upgrade_a_populated_m2_database() {
        use sqlx::migrate::Migrator;
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let directory = TempDir::new().unwrap();
        let database_path = directory.path().join(DATABASE_FILENAME);
        let options = SqliteConnectOptions::new()
            .filename(&database_path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .unwrap();

        // Apply only the M2 schema through the real migrator path: a directory
        // holding just 0001. Then populate it like a real M2 service.
        let m2_migrations = TempDir::new().unwrap();
        std::fs::write(
            m2_migrations.path().join("0001_conversion_jobs.sql"),
            include_str!("../../migrations/0001_conversion_jobs.sql"),
        )
        .unwrap();
        Migrator::new(m2_migrations.path())
            .await
            .unwrap()
            .run(&pool)
            .await
            .unwrap();

        let job_id = "11111111-2222-4333-8444-555555555555";
        let attempt_id = "66666666-7777-4888-8999-000000000000";
        // M2 required a profile. Migration 0007 dropped it.
        sqlx::query(
            "INSERT INTO conversions (
                id, client_run_id, auth_scope, idempotency_key_hash,
                request_fingerprint, profile, status, source_relative_path,
                source_media_type, source_byte_length, source_sha256,
                reason_codes_json, warnings_json, origin_request_id,
                created_at, updated_at
             ) VALUES (
                ?1, ?1, 'bootstrap', ?2, ?2, 'standard', 'queued', ?3,
                'application/pdf', 128, ?2, '[]', '[]', 'request-1', ?4, ?4
             )",
        )
        .bind(job_id)
        .bind("a".repeat(64))
        .bind(format!("jobs/{job_id}/source/input"))
        .bind("2026-08-18T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO attempts (
                id, conversion_id, attempt_number, state, recovery_count,
                reason_codes_json, warnings_json, created_at, updated_at
             ) VALUES (?1, ?2, 1, 'queued', 0, '[]', '[]',
                '2026-08-18T00:00:00Z', '2026-08-18T00:00:00Z')",
        )
        .bind(attempt_id)
        .bind(job_id)
        .execute(&pool)
        .await
        .unwrap();

        // The full migrator applies every later migration over the M2 rows.
        super::MIGRATOR.run(&pool).await.unwrap();
        // Every conversion seeded after the upgrade is this insert. Only the
        // columns it takes vary.
        let insert_conversion = async |id: &str,
                                       digest_character: &str,
                                       media_type: &str,
                                       byte_length: i64,
                                       origin_request_id: &str,
                                       timestamp: &str| {
            sqlx::query(
                "INSERT INTO conversions (
                    id, client_run_id, auth_scope, idempotency_key_hash,
                    request_fingerprint, status, source_relative_path,
                    source_media_type, source_byte_length, source_sha256,
                    reason_codes_json, warnings_json, origin_request_id,
                    created_at, updated_at
                 ) VALUES (
                    ?1, ?1, 'bootstrap', ?2, ?2, 'queued', ?3, ?4, ?5, ?2,
                    '[]', '[]', ?6, ?7, ?7
                 )",
            )
            .bind(id)
            .bind(digest_character.repeat(64))
            .bind(format!("jobs/{id}/source/input"))
            .bind(media_type)
            .bind(byte_length)
            .bind(origin_request_id)
            .bind(timestamp)
            .execute(&pool)
            .await
        };

        let surviving: i64 = sqlx::query("SELECT COUNT(*) AS n FROM conversions")
            .fetch_one(&pool)
            .await
            .unwrap()
            .try_get("n")
            .unwrap();
        assert_eq!(surviving, 1, "the M2 row must survive the rebuild");
        let surviving_attempts: i64 = sqlx::query("SELECT COUNT(*) AS n FROM attempts")
            .fetch_one(&pool)
            .await
            .unwrap()
            .try_get("n")
            .unwrap();
        assert_eq!(surviving_attempts, 1);
        assert!(sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty());

        // The widened constraints accept the new formats...
        let docx_id = "aaaaaaaa-1111-4111-8222-333333333333";
        insert_conversion(
            docx_id,
            "b",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            256,
            "request-2",
            "2026-08-18T00:00:01Z",
        )
        .await
        .unwrap();

        // ...including a media type that only migration 0003 admits.
        let odt_id = "aaaaaaaa-2222-4111-8222-333333333333";
        insert_conversion(
            odt_id,
            "c",
            "application/vnd.oasis.opendocument.text",
            256,
            "request-3",
            "2026-08-18T00:00:02Z",
        )
        .await
        .expect("migration 0003 must admit the OpenDocument media types");
        let docx_attempt = "bbbbbbbb-2222-4333-8444-555555555555";
        sqlx::query(
            "INSERT INTO attempts (
                id, conversion_id, attempt_number, state, recovery_count,
                classification, reason_codes_json, warnings_json,
                created_at, updated_at
             ) VALUES (?1, ?2, 1, 'queued', 0, 'structured_document', '[]', '[]',
                '2026-08-18T00:00:01Z', '2026-08-18T00:00:01Z')",
        )
        .bind(docx_attempt)
        .bind(docx_id)
        .execute(&pool)
        .await
        .unwrap();

        // ...and a media type that only migration 0004 admits, which also
        // carries the OCR settings 0004 added.
        let png_id = "aaaaaaaa-3333-4111-8222-333333333333";
        sqlx::query(
            "INSERT INTO conversions (
                id, client_run_id, auth_scope, idempotency_key_hash,
                request_fingerprint, status, source_relative_path,
                source_media_type, source_byte_length, source_sha256,
                reason_codes_json, warnings_json, origin_request_id,
                created_at, updated_at, ocr_language_correction,
                ocr_custom_words
             ) VALUES (
                ?1, ?1, 'bootstrap', ?2, ?2, 'queued', ?3,
                'image/png', 256, ?2, '[]', '[]', 'request-4',
                '2026-08-18T00:00:03Z', '2026-08-18T00:00:03Z', 0, 'Uniwise'
             )",
        )
        .bind(png_id)
        .bind("d".repeat(64))
        .bind(format!("jobs/{png_id}/source/input"))
        .execute(&pool)
        .await
        .expect("migration 0004 must admit the image media types");

        // The M2 row predates both settings, so the rebuild must have given it
        // the documented defaults rather than dropping it.
        let defaults = sqlx::query(
            "SELECT ocr_language_correction, ocr_custom_words
             FROM conversions WHERE id = ?1",
        )
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(defaults
            .try_get::<bool, _>("ocr_language_correction")
            .unwrap());
        assert_eq!(
            defaults.try_get::<String, _>("ocr_custom_words").unwrap(),
            ""
        );

        // ...and still reject nonsense.
        let bad_id = "cccccccc-3333-4333-8444-555555555555";
        let rejected = insert_conversion(
            bad_id,
            "c",
            "text/plain",
            64,
            "request-3",
            "2026-08-18T00:00:02Z",
        )
        .await;
        assert!(rejected.is_err(), "an unlisted media type must be rejected");

        pool.close().await;
    }

    /// Migration 0007 runs over ledgers that already hold finished jobs. A
    /// succeeded job keeps its Markdown row and stays readable, and its
    /// manifest row goes. A needs_remote job becomes failed with its reason as
    /// the code, and the profile column goes.
    #[tokio::test]
    async fn migration_0007_upgrades_a_ledger_with_finished_jobs() {
        use sqlx::migrate::Migrator;
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let directory = TempDir::new().unwrap();
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(directory.path().join(DATABASE_FILENAME))
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        let before_0007 = TempDir::new().unwrap();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations")).unwrap()
        {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_owned();
            if name.to_str().unwrap() < "0007" {
                std::fs::copy(&path, before_0007.path().join(name)).unwrap();
            }
        }
        Migrator::new(before_0007.path())
            .await
            .unwrap()
            .run(&pool)
            .await
            .unwrap();

        let succeeded = "11111111-2222-4333-8444-555555555555";
        let succeeded_attempt = "66666666-7777-4888-8999-000000000000";
        let at = "2026-10-01T00:00:00Z";
        sqlx::query(
            "INSERT INTO conversions (
                id, client_run_id, auth_scope, idempotency_key_hash,
                request_fingerprint, profile, status, source_relative_path,
                source_media_type, source_byte_length, source_sha256, route,
                reason_codes_json, warnings_json, origin_request_id,
                created_at, updated_at
             ) VALUES (
                ?1, ?1, 'bootstrap', ?2, ?2, 'local_only', 'succeeded',
                'jobs/' || ?1 || '/source/input', 'application/pdf', 128, ?2,
                'local_pdf', '[\"native_text_pdf\"]', '[]', 'request-1', ?3, ?3
             )",
        )
        .bind(succeeded)
        .bind("a".repeat(64))
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO attempts (
                id, conversion_id, attempt_number, state, recovery_count,
                engine_name, engine_version, route, classification,
                inspection_json, reason_codes_json, warnings_json,
                created_at, updated_at, started_at, finished_at
             ) VALUES (
                ?1, ?2, 1, 'succeeded', 0, 'pdf-inspector', '1.25.2',
                'local_pdf', 'text_based', '{}', '[\"native_text_pdf\"]', '[]',
                ?3, ?3, ?3, ?3
             )",
        )
        .bind(succeeded_attempt)
        .bind(succeeded)
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE conversions SET active_attempt_id = ?1 WHERE id = ?2")
            .bind(succeeded_attempt)
            .bind(succeeded)
            .execute(&pool)
            .await
            .unwrap();
        let published = format!("jobs/{succeeded}/attempts/{succeeded_attempt}/artifacts");
        sqlx::query(
            "INSERT INTO artifacts (
                attempt_id, kind, relative_path, media_type, byte_length,
                sha256, created_at
             ) VALUES
                (?1, 'markdown', ?2 || '/result.md',
                 'text/markdown; charset=utf-8', 9, ?3, ?5),
                (?1, 'manifest', ?2 || '/manifest.json',
                 'application/json', 64, ?4, ?5)",
        )
        .bind(succeeded_attempt)
        .bind(&published)
        .bind("b".repeat(64))
        .bind("c".repeat(64))
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();

        let gave_up = "22222222-2222-4333-8444-555555555555";
        let gave_up_attempt = "77777777-7777-4888-8999-000000000000";
        sqlx::query(
            "INSERT INTO conversions (
                id, client_run_id, auth_scope, idempotency_key_hash,
                request_fingerprint, profile, status, source_relative_path,
                source_media_type, source_byte_length, source_sha256, route,
                reason_codes_json, warnings_json, origin_request_id,
                created_at, updated_at
             ) VALUES (
                ?1, ?1, 'bootstrap', ?2, ?2, 'local_only', 'needs_remote',
                'jobs/' || ?1 || '/source/input', 'application/pdf', 128, ?2,
                'local_pdf', '[\"mixed_pdf\"]', '[]', 'request-2', ?3, ?3
             )",
        )
        .bind(gave_up)
        .bind("d".repeat(64))
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO attempts (
                id, conversion_id, attempt_number, state, recovery_count,
                engine_name, engine_version, route, classification,
                inspection_json, reason_codes_json, warnings_json,
                fallback_reason, created_at, updated_at, started_at, finished_at
             ) VALUES (
                ?1, ?2, 1, 'needs_remote', 0, 'pdf-inspector', '1.25.2',
                'local_pdf', 'mixed', '{\"pageCount\":2}', '[\"mixed_pdf\"]', '[]',
                'mixed_pdf', ?3, ?3, ?3, ?3
             )",
        )
        .bind(gave_up_attempt)
        .bind(gave_up)
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE conversions SET active_attempt_id = ?1 WHERE id = ?2")
            .bind(gave_up_attempt)
            .bind(gave_up)
            .execute(&pool)
            .await
            .unwrap();

        super::MIGRATOR.run(&pool).await.unwrap();
        let columns: Vec<String> = sqlx::query("SELECT name FROM pragma_table_info('conversions')")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|row| row.try_get("name").unwrap())
            .collect();
        assert!(!columns.iter().any(|name| name == "profile"));
        let kinds: Vec<String> = sqlx::query("SELECT kind FROM artifacts")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|row| row.try_get("kind").unwrap())
            .collect();
        assert_eq!(kinds, ["markdown"]);
        assert!(sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty());
        pool.close().await;

        let repository = SqliteRepository::open(directory.path(), 4, BUSY_TIMEOUT)
            .await
            .unwrap();
        let job = repository
            .get(Uuid::parse_str(succeeded).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.state, ConversionState::Succeeded);
        assert_eq!(job.artifacts.len(), 1);
        assert_eq!(job.artifacts[0].sha256, "b".repeat(64));
        assert_eq!(job.active_attempt.markdown_byte_length, None);

        let job = repository
            .get(Uuid::parse_str(gave_up).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.state, ConversionState::Failed);
        assert_eq!(job.active_attempt.state, AttemptState::Failed);
        let failure = StoredFailure {
            code: "mixed_pdf".to_owned(),
            message: "This file could not be converted on this Mac because some pages are \
                      scanned images."
                .to_owned(),
        };
        assert_eq!(job.failure.as_ref(), Some(&failure));
        assert_eq!(job.active_attempt.failure.as_ref(), Some(&failure));
        assert_eq!(job.reason_codes, ["mixed_pdf"]);
        assert!(job.active_attempt.inspection.is_some());
    }

    #[tokio::test]
    async fn migrations_configure_a_file_backed_database_and_repeat_cleanly() {
        let (directory, repository) = open_repository(4).await;

        assert!(directory.path().join(DATABASE_FILENAME).is_file());
        let tables: i64 = sqlx::query(
            "SELECT COUNT(*) AS table_count
             FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('conversions', 'attempts', 'artifacts')",
        )
        .fetch_one(&repository.pool)
        .await
        .unwrap()
        .try_get("table_count")
        .unwrap();
        assert_eq!(tables, 3);

        let mut pooled = repository.pool.acquire().await.unwrap();
        assert_pragmas(&mut pooled).await;
        drop(pooled);
        let sqlite_features = sqlx::query(
            "SELECT sqlite_version() AS version,
                    json_valid('[]') AS json_available,
                    json_type('[]') AS json_kind",
        )
        .fetch_one(&repository.pool)
        .await
        .unwrap();
        let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&repository.pool)
            .await
            .unwrap();
        assert!(!sqlite_features
            .try_get::<String, _>("version")
            .unwrap()
            .is_empty());
        assert_eq!(
            sqlite_features.try_get::<i64, _>("json_available").unwrap(),
            1
        );
        assert_eq!(
            sqlite_features.try_get::<String, _>("json_kind").unwrap(),
            "array"
        );
        assert!(foreign_key_violations.is_empty());

        let mut connections = Vec::new();
        for _ in 0..MAX_POOL_CONNECTIONS {
            connections.push(repository.pool.acquire().await.unwrap());
        }
        for connection in &mut connections {
            assert_pragmas(connection).await;
        }
        drop(connections);

        drop(repository);
        SqliteRepository::open(directory.path(), 4, BUSY_TIMEOUT)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_get_replay_conflict_and_capacity_are_durable() {
        let (directory, repository) = open_repository(1).await;
        let original = new_conversion("same-key", "a");
        let original_id = original.id;

        let created = repository.create_or_replay(original.clone()).await.unwrap();
        let CreateOutcome::Created(created) = created else {
            panic!("first submission must create a conversion");
        };
        assert_eq!(created.id, original_id);
        assert_eq!(created.state, ConversionState::Queued);
        assert_eq!(created.active_attempt.state, AttemptState::Queued);
        assert_eq!(created.active_attempt.number, 1);
        assert!(created.active_attempt.queue_sequence > 0);
        assert!(!repository.accepting_jobs().await.unwrap());

        let replay = repository.create_or_replay(original.clone()).await.unwrap();
        assert!(matches!(
            replay,
            CreateOutcome::Replay(conversion) if conversion.id == original_id
        ));

        let mut conflict = original;
        conflict.id = Uuid::new_v4();
        conflict.initial_attempt_id = Uuid::new_v4();
        conflict.source.relative_path = format!("jobs/{}/source/input", conflict.id);
        conflict.request_fingerprint = "b".repeat(64);
        assert!(matches!(
            repository.create_or_replay(conflict).await.unwrap(),
            CreateOutcome::Conflict
        ));

        let another = new_conversion("other-key", "c");
        assert!(matches!(
            repository.create_or_replay(another).await.unwrap(),
            CreateOutcome::Capacity
        ));

        drop(repository);
        let reopened = SqliteRepository::open(directory.path(), 1, BUSY_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(
            reopened.get(original_id).await.unwrap().unwrap().id,
            original_id
        );
        assert!(matches!(
            reopened.create_or_replay(new_conversion_with_ids(
                "same-key",
                "a",
                Uuid::new_v4(),
                Uuid::new_v4()
            )).await.unwrap(),
            CreateOutcome::Replay(conversion) if conversion.id == original_id
        ));
    }

    #[tokio::test]
    async fn concurrent_idempotent_creates_choose_one_conversion() {
        let (_directory, repository) = open_repository(16).await;
        let barrier = Arc::new(Barrier::new(8));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let repository = repository.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                let input = new_conversion("concurrent-key", "d");
                barrier.wait().await;
                repository.create_or_replay(input).await.unwrap()
            }));
        }

        let mut created_id = None;
        let mut created_count = 0;
        for task in tasks {
            match task.await.unwrap() {
                CreateOutcome::Created(conversion) => {
                    created_count += 1;
                    created_id = Some(conversion.id);
                }
                CreateOutcome::Replay(conversion) => {
                    if let Some(id) = created_id {
                        assert_eq!(conversion.id, id);
                    } else {
                        created_id = Some(conversion.id);
                    }
                }
                outcome => panic!("unexpected outcome: {outcome:?}"),
            }
        }
        assert_eq!(created_count, 1);

        let rows: i64 = sqlx::query("SELECT COUNT(*) AS row_count FROM conversions")
            .fetch_one(&repository.pool)
            .await
            .unwrap()
            .try_get("row_count")
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn two_fresh_keys_compete_for_one_active_slot_atomically() {
        let (_directory, repository) = open_repository(1).await;
        let barrier = Arc::new(Barrier::new(2));
        let mut tasks = Vec::new();
        for input in [
            new_conversion("first-fresh", "a"),
            new_conversion("second-fresh", "b"),
        ] {
            let repository = repository.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                repository.create_or_replay(input).await.unwrap()
            }));
        }

        let outcomes = futures_in_order(tasks).await;
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CreateOutcome::Created(_)))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CreateOutcome::Capacity))
                .count(),
            1
        );
        assert!(!repository.accepting_jobs().await.unwrap());

        let rows: i64 = sqlx::query("SELECT COUNT(*) AS row_count FROM conversions")
            .fetch_one(&repository.pool)
            .await
            .unwrap()
            .try_get("row_count")
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn replay_conflict_and_capacity_keep_precedence_under_concurrency() {
        let (_directory, repository) = open_repository(1).await;
        let accepted = new_conversion("accepted-key", "a");
        let accepted_id = accepted.id;
        assert!(matches!(
            repository.create_or_replay(accepted.clone()).await.unwrap(),
            CreateOutcome::Created(_)
        ));

        let replay = new_conversion_with_ids("accepted-key", "a", Uuid::new_v4(), Uuid::new_v4());
        let mut conflict = replay.clone();
        conflict.request_fingerprint = "b".repeat(64);
        let capacity = new_conversion("capacity-key", "c");
        let barrier = Arc::new(Barrier::new(3));

        let mut tasks = Vec::new();
        for input in [replay, conflict, capacity] {
            let repository = repository.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                repository.create_or_replay(input).await.unwrap()
            }));
        }
        let outcomes = futures_in_order(tasks).await;

        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, CreateOutcome::Replay(job) if job.id == accepted_id)));
        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, CreateOutcome::Conflict)));
        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, CreateOutcome::Capacity)));
    }

    #[tokio::test]
    async fn active_attempt_cannot_point_at_another_conversion() {
        let (_directory, repository) = open_repository(2).await;
        let first = queued_conversion(&repository, "first", "a").await;
        let second = queued_conversion(&repository, "second", "b").await;

        let result = sqlx::query(
            "UPDATE conversions
             SET active_attempt_id = ?1
             WHERE id = ?2",
        )
        .bind(second.active_attempt.id.hyphenated().to_string())
        .bind(first.id.hyphenated().to_string())
        .execute(&repository.pool)
        .await;
        assert!(result.is_err());

        let first_after = repository.get(first.id).await.unwrap().unwrap();
        assert_eq!(first_after.active_attempt.id, first.active_attempt.id);
        assert!(sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&repository.pool)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn raw_idempotency_keys_are_not_persisted() {
        let (_directory, repository) = open_repository(2).await;
        let input = new_conversion("do-not-store-this-key", "e");
        let expected_hash = input.idempotency_key_sha256.clone();
        repository.create_or_replay(input).await.unwrap();

        let row = sqlx::query("SELECT auth_scope, idempotency_key_hash FROM conversions LIMIT 1")
            .fetch_one(&repository.pool)
            .await
            .unwrap();
        assert_eq!(row.try_get::<String, _>("auth_scope").unwrap(), AUTH_SCOPE);
        assert_eq!(
            row.try_get::<String, _>("idempotency_key_hash").unwrap(),
            expected_hash
        );
    }

    #[tokio::test]
    async fn legal_success_path_commits_the_markdown_and_integrity_failure_retains_audit_rows() {
        let (_directory, repository) = open_repository(2).await;
        let queued = queued_conversion(&repository, "success", "a").await;
        let conversion_id = queued.id;
        let attempt_id = queued.active_attempt.id;

        let converting = repository
            .start_local(conversion_id, attempt_id, local_start())
            .await
            .unwrap();
        assert_eq!(converting.state, ConversionState::ConvertingLocal);
        assert_eq!(
            converting.active_attempt.state,
            AttemptState::ConvertingLocal
        );
        assert_eq!(converting.active_attempt.engine, Some(local_start().engine));
        assert_eq!(converting.route.as_deref(), Some("local_pdf"));
        assert!(converting.active_attempt.started_at.is_some());

        let finalizing = repository
            .mark_finalizing(conversion_id, attempt_id, local_analysis(), MARKDOWN, None)
            .await
            .unwrap();
        assert_eq!(finalizing.state, ConversionState::Finalizing);
        assert_eq!(finalizing.active_attempt.state, AttemptState::Finalizing);
        assert_eq!(
            finalizing.active_attempt.classification.as_deref(),
            Some("text_based")
        );
        assert_eq!(finalizing.reason_codes, vec!["native_text_pdf"]);
        assert_eq!(finalizing.active_attempt.markdown_byte_length, Some(32));
        assert_eq!(
            finalizing.active_attempt.markdown_sha256.as_deref(),
            Some(MARKDOWN.1)
        );

        let succeeded = repository
            .finish_succeeded(
                conversion_id,
                attempt_id,
                markdown_artifact(conversion_id, attempt_id),
            )
            .await
            .unwrap();
        assert_eq!(succeeded.state, ConversionState::Succeeded);
        assert_eq!(succeeded.active_attempt.state, AttemptState::Succeeded);
        assert!(succeeded.active_attempt.finished_at.is_some());
        assert_eq!(succeeded.artifacts.len(), 1);
        assert!(succeeded.artifacts[0].relative_path.ends_with("/result.md"));

        let integrity_failed = repository
            .mark_artifact_integrity_failed(
                conversion_id,
                attempt_id,
                StoredFailure {
                    code: "artifact_integrity_failed".to_owned(),
                    message: "Published artifacts failed integrity validation.".to_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(integrity_failed.state, ConversionState::Failed);
        assert_eq!(
            integrity_failed.active_attempt.state,
            AttemptState::Succeeded
        );
        assert_eq!(integrity_failed.artifacts.len(), 1);
        assert_eq!(
            integrity_failed.failure.unwrap().code,
            "artifact_integrity_failed"
        );
    }

    #[tokio::test]
    async fn terminal_outcomes_require_their_exact_legal_source_state() {
        let (_directory, repository) = open_repository(8).await;

        let queued_failure = queued_conversion(&repository, "queued-failure", "a").await;
        let failed = repository
            .finish_failed(
                queued_failure.id,
                queued_failure.active_attempt.id,
                failed_result(FailureStage::Queued, "source_integrity_failed"),
            )
            .await
            .unwrap();
        assert_eq!(failed.state, ConversionState::Failed);
        assert_eq!(failed.active_attempt.state, AttemptState::Failed);

        let gave_up = converting_conversion(&repository, "gave-up", "b").await;
        let gave_up = repository
            .finish_gave_up(
                gave_up.id,
                gave_up.active_attempt.id,
                scanned_analysis(),
                failed_result(FailureStage::ConvertingLocal, "scanned_pdf").failure,
            )
            .await
            .unwrap();
        assert_eq!(gave_up.state, ConversionState::Failed);
        assert_eq!(gave_up.active_attempt.state, AttemptState::Failed);
        assert_eq!(gave_up.failure.unwrap().code, "scanned_pdf");
        assert_eq!(gave_up.reason_codes, ["scanned_pdf"]);
        // The analysis stays as failure evidence.
        assert_eq!(
            gave_up.active_attempt.fallback_reason.as_deref(),
            Some("scanned_pdf")
        );
        assert_eq!(
            gave_up.active_attempt.inspection,
            Some(serde_json::json!({"pageCount": 1}))
        );

        let converting_failure =
            converting_conversion(&repository, "converting-failure", "c").await;
        let failed = repository
            .finish_failed(
                converting_failure.id,
                converting_failure.active_attempt.id,
                failed_result(FailureStage::ConvertingLocal, "engine_failed"),
            )
            .await
            .unwrap();
        assert_eq!(failed.state, ConversionState::Failed);

        let finalizing_failure =
            converting_conversion(&repository, "finalizing-failure", "d").await;
        repository
            .mark_finalizing(
                finalizing_failure.id,
                finalizing_failure.active_attempt.id,
                local_analysis(),
                MARKDOWN,
                None,
            )
            .await
            .unwrap();
        let failed = repository
            .finish_failed(
                finalizing_failure.id,
                finalizing_failure.active_attempt.id,
                failed_result(FailureStage::Finalizing, "artifact_publication_failed"),
            )
            .await
            .unwrap();
        assert_eq!(failed.state, ConversionState::Failed);
        assert_eq!(failed.active_attempt.state, AttemptState::Failed);
    }

    #[tokio::test]
    async fn stale_and_illegal_transitions_do_not_mutate_the_conversion() {
        let (_directory, repository) = open_repository(2).await;
        let queued = queued_conversion(&repository, "transition-guards", "a").await;

        assert!(matches!(
            repository
                .start_local(queued.id, Uuid::new_v4(), local_start())
                .await,
            Err(RepositoryError::StaleActiveAttempt { .. })
        ));
        assert!(matches!(
            repository
                .mark_finalizing(
                    queued.id,
                    queued.active_attempt.id,
                    local_analysis(),
                    MARKDOWN,
                    None
                )
                .await,
            Err(RepositoryError::IllegalTransition { .. })
        ));
        let still_queued = repository.get(queued.id).await.unwrap().unwrap();
        assert_eq!(still_queued.state, ConversionState::Queued);
        assert_eq!(still_queued.active_attempt.state, AttemptState::Queued);

        repository
            .start_local(queued.id, queued.active_attempt.id, local_start())
            .await
            .unwrap();
        assert!(matches!(
            repository
                .start_local(queued.id, queued.active_attempt.id, local_start())
                .await,
            Err(RepositoryError::IllegalTransition { .. })
        ));
        assert!(matches!(
            repository
                .finish_succeeded(
                    queued.id,
                    queued.active_attempt.id,
                    markdown_artifact(queued.id, queued.active_attempt.id),
                )
                .await,
            Err(RepositoryError::IllegalTransition { .. })
        ));
        let converting = repository.get(queued.id).await.unwrap().unwrap();
        assert_eq!(converting.state, ConversionState::ConvertingLocal);
        assert!(converting.artifacts.is_empty());
    }

    #[tokio::test]
    async fn a_queued_row_this_boot_cannot_serve_waits_instead_of_failing() {
        let (_directory, repository) = open_repository(2).await;
        let mut audio = new_conversion("claim-audio", "a");
        audio.source.media_type = "audio/wav".to_owned();
        let audio = created(repository.create_or_replay(audio).await.unwrap());
        let pdf = queued_conversion(&repository, "claim-pdf", "b").await;

        // A boot without the audio worker: the older audio row is skipped and
        // left queued, and the PDF behind it still runs.
        let claimed = repository
            .claim_next_queued(&["application/pdf"], |_| Some(local_start()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, pdf.id);
        let waiting = repository.get(audio.id).await.unwrap().unwrap();
        assert_eq!(waiting.state, ConversionState::Queued);

        // A boot with the worker claims it.
        let claimed = repository
            .claim_next_queued(&["application/pdf", "audio/wav"], |_| Some(local_start()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, audio.id);
    }

    #[tokio::test]
    async fn concurrent_fifo_claims_take_each_queued_attempt_once_in_order() {
        let (_directory, repository) = open_repository(4).await;
        let first = queued_conversion(&repository, "fifo-first", "a").await;
        let second = queued_conversion(&repository, "fifo-second", "b").await;
        let third = queued_conversion(&repository, "fifo-third", "c").await;
        let expected_first_two = [
            first.active_attempt.queue_sequence,
            second.active_attempt.queue_sequence,
        ];

        let barrier = Arc::new(Barrier::new(2));
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let repository = repository.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                repository
                    .claim_next_queued(&["application/pdf"], |_| Some(local_start()))
                    .await
                    .unwrap()
                    .unwrap()
            }));
        }
        let mut claimed = Vec::new();
        for task in tasks {
            claimed.push(task.await.unwrap());
        }
        claimed.sort_by_key(|conversion| conversion.active_attempt.queue_sequence);
        assert_eq!(
            claimed
                .iter()
                .map(|conversion| conversion.active_attempt.queue_sequence)
                .collect::<Vec<_>>(),
            expected_first_two
        );
        assert!(claimed
            .iter()
            .all(|conversion| conversion.state == ConversionState::ConvertingLocal));

        let last = repository
            .claim_next_queued(&["application/pdf"], |_| Some(local_start()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(last.id, third.id);
        assert!(repository
            .claim_next_queued(&["application/pdf"], |_| Some(local_start()))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn concurrent_exact_start_allows_one_transition_and_rejects_the_stale_caller() {
        let (_directory, repository) = open_repository(2).await;
        let queued = queued_conversion(&repository, "concurrent-start", "a").await;
        let conversion_id = queued.id;
        let attempt_id = queued.active_attempt.id;
        let barrier = Arc::new(Barrier::new(2));
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let repository = repository.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                repository
                    .start_local(conversion_id, attempt_id, local_start())
                    .await
            }));
        }

        let mut succeeded = 0;
        let mut rejected = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(conversion) => {
                    succeeded += 1;
                    assert_eq!(conversion.state, ConversionState::ConvertingLocal);
                }
                Err(RepositoryError::IllegalTransition { .. }) => rejected += 1,
                Err(error) => panic!("unexpected transition error: {error}"),
            }
        }
        assert_eq!(succeeded, 1);
        assert_eq!(rejected, 1);
    }

    #[tokio::test]
    async fn artifact_insert_failure_rolls_back_the_row_and_success_transition() {
        let (_directory, repository) = open_repository(2).await;
        let conversion = converting_conversion(&repository, "artifact-atomic", "a").await;
        repository
            .mark_finalizing(
                conversion.id,
                conversion.active_attempt.id,
                local_analysis(),
                MARKDOWN,
                None,
            )
            .await
            .unwrap();

        sqlx::query(
            "CREATE TRIGGER reject_markdown
             BEFORE INSERT ON artifacts
             BEGIN SELECT RAISE(ABORT, 'test artifact failure'); END",
        )
        .execute(&repository.pool)
        .await
        .unwrap();
        assert!(matches!(
            repository
                .finish_succeeded(
                    conversion.id,
                    conversion.active_attempt.id,
                    markdown_artifact(conversion.id, conversion.active_attempt.id),
                )
                .await,
            Err(RepositoryError::Database(_))
        ));
        let after = repository.get(conversion.id).await.unwrap().unwrap();
        assert_eq!(after.state, ConversionState::Finalizing);
        assert_eq!(after.active_attempt.state, AttemptState::Finalizing);
        assert!(after.artifacts.is_empty());
    }

    #[tokio::test]
    async fn conversion_update_failure_rolls_back_the_prior_attempt_transition() {
        let (_directory, repository) = open_repository(2).await;
        let conversion = queued_conversion(&repository, "transition-atomic", "a").await;
        sqlx::query(
            "CREATE TRIGGER reject_failed_conversion
             BEFORE UPDATE OF status ON conversions
             WHEN NEW.status = 'failed'
             BEGIN SELECT RAISE(ABORT, 'test conversion failure'); END",
        )
        .execute(&repository.pool)
        .await
        .unwrap();

        assert!(matches!(
            repository
                .finish_failed(
                    conversion.id,
                    conversion.active_attempt.id,
                    failed_result(FailureStage::Queued, "source_integrity_failed"),
                )
                .await,
            Err(RepositoryError::Database(_))
        ));
        let after = repository.get(conversion.id).await.unwrap().unwrap();
        assert_eq!(after.state, ConversionState::Queued);
        assert_eq!(after.active_attempt.state, AttemptState::Queued);
        assert!(after.failure.is_none());
        assert!(after.active_attempt.failure.is_none());
        assert!(after.active_attempt.finished_at.is_none());
    }

    #[tokio::test]
    async fn requeue_preserves_history_reenters_fifo_and_stops_at_the_limit() {
        let (_directory, repository) = open_repository(4).await;
        let first = converting_conversion(&repository, "requeue-first", "a").await;
        let second = queued_conversion(&repository, "requeue-second", "b").await;
        let recovered_attempt_id = Uuid::new_v4();

        let outcome = repository
            .interrupt_and_requeue(first.id, first.active_attempt.id, recovered_attempt_id, 1)
            .await
            .unwrap();
        let RequeueOutcome::Requeued(requeued) = outcome else {
            panic!("first recovery must requeue");
        };
        assert_eq!(requeued.state, ConversionState::Queued);
        assert_eq!(requeued.active_attempt.id, recovered_attempt_id);
        assert_eq!(requeued.active_attempt.number, 2);
        assert_eq!(requeued.active_attempt.recovery_count, 1);
        assert!(
            requeued.active_attempt.queue_sequence > second.active_attempt.queue_sequence,
            "recovered work must reenter at the end of FIFO"
        );
        assert!(requeued.route.is_none());
        assert!(requeued.reason_codes.is_empty());

        let old_attempt = sqlx::query(
            "SELECT state, finished_at FROM attempts WHERE conversion_id = ?1 AND id = ?2",
        )
        .bind(first.id.hyphenated().to_string())
        .bind(first.active_attempt.id.hyphenated().to_string())
        .fetch_one(&repository.pool)
        .await
        .unwrap();
        assert_eq!(
            old_attempt.try_get::<String, _>("state").unwrap(),
            "interrupted"
        );
        assert!(old_attempt
            .try_get::<Option<String>, _>("finished_at")
            .unwrap()
            .is_some());

        let claimed_second = repository
            .claim_next_queued(&["application/pdf"], |_| Some(local_start()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed_second.id, second.id);
        let claimed_recovery = repository
            .claim_next_queued(&["application/pdf"], |_| Some(local_start()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed_recovery.id, first.id);

        let limit = repository
            .interrupt_and_requeue(first.id, recovered_attempt_id, Uuid::new_v4(), 1)
            .await
            .unwrap();
        let RequeueOutcome::LimitReached(unchanged) = limit else {
            panic!("configured recovery limit must stop another attempt");
        };
        assert_eq!(unchanged.state, ConversionState::ConvertingLocal);
        assert_eq!(unchanged.active_attempt.id, recovered_attempt_id);
        assert_eq!(unchanged.active_attempt.recovery_count, 1);
        let attempt_count: i64 =
            sqlx::query("SELECT COUNT(*) AS attempt_count FROM attempts WHERE conversion_id = ?1")
                .bind(first.id.hyphenated().to_string())
                .fetch_one(&repository.pool)
                .await
                .unwrap()
                .try_get("attempt_count")
                .unwrap();
        assert_eq!(attempt_count, 2);
    }

    #[tokio::test]
    async fn requeue_accepts_finalizing_and_rejects_illegal_or_stale_attempts() {
        let (_directory, repository) = open_repository(3).await;
        let queued = queued_conversion(&repository, "requeue-guards", "a").await;
        assert!(matches!(
            repository
                .interrupt_and_requeue(queued.id, queued.active_attempt.id, Uuid::new_v4(), 3,)
                .await,
            Err(RepositoryError::IllegalRecoveryState { .. })
        ));

        repository
            .start_local(queued.id, queued.active_attempt.id, local_start())
            .await
            .unwrap();
        repository
            .mark_finalizing(
                queued.id,
                queued.active_attempt.id,
                local_analysis(),
                MARKDOWN,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            repository
                .interrupt_and_requeue(queued.id, Uuid::new_v4(), Uuid::new_v4(), 3)
                .await,
            Err(RepositoryError::StaleActiveAttempt { .. })
        ));

        let new_attempt_id = Uuid::new_v4();
        let outcome = repository
            .interrupt_and_requeue(queued.id, queued.active_attempt.id, new_attempt_id, 3)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            RequeueOutcome::Requeued(conversion)
                if conversion.active_attempt.id == new_attempt_id
                    && conversion.state == ConversionState::Queued
        ));
    }

    #[tokio::test]
    async fn concurrent_requeue_allows_one_new_active_attempt() {
        let (_directory, repository) = open_repository(2).await;
        let conversion = converting_conversion(&repository, "concurrent-requeue", "a").await;
        let new_ids = [Uuid::new_v4(), Uuid::new_v4()];
        let barrier = Arc::new(Barrier::new(2));
        let mut tasks = Vec::new();
        for new_attempt_id in new_ids {
            let repository = repository.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                repository
                    .interrupt_and_requeue(
                        conversion.id,
                        conversion.active_attempt.id,
                        new_attempt_id,
                        3,
                    )
                    .await
            }));
        }

        let mut winner = None;
        let mut stale = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(RequeueOutcome::Requeued(requeued)) => {
                    assert!(winner.replace(requeued.active_attempt.id).is_none());
                }
                Err(RepositoryError::StaleActiveAttempt { .. }) => stale += 1,
                result => panic!("unexpected concurrent recovery result: {result:?}"),
            }
        }
        assert_eq!(stale, 1);
        assert!(new_ids.contains(&winner.unwrap()));
        let attempt_count: i64 =
            sqlx::query("SELECT COUNT(*) AS attempt_count FROM attempts WHERE conversion_id = ?1")
                .bind(conversion.id.hyphenated().to_string())
                .fetch_one(&repository.pool)
                .await
                .unwrap()
                .try_get("attempt_count")
                .unwrap();
        assert_eq!(attempt_count, 2);
    }

    #[tokio::test]
    async fn recovery_listing_is_filtered_complete_and_includes_artifacts_in_one_view() {
        let (_directory, repository) = open_repository(8).await;
        let queued = queued_conversion(&repository, "list-queued", "a").await;
        let converting = converting_conversion(&repository, "list-converting", "b").await;
        let finalizing = converting_conversion(&repository, "list-finalizing", "c").await;
        repository
            .mark_finalizing(
                finalizing.id,
                finalizing.active_attempt.id,
                local_analysis(),
                MARKDOWN,
                None,
            )
            .await
            .unwrap();
        let succeeded = converting_conversion(&repository, "list-succeeded", "d").await;
        repository
            .mark_finalizing(
                succeeded.id,
                succeeded.active_attempt.id,
                local_analysis(),
                MARKDOWN,
                None,
            )
            .await
            .unwrap();
        repository
            .finish_succeeded(
                succeeded.id,
                succeeded.active_attempt.id,
                markdown_artifact(succeeded.id, succeeded.active_attempt.id),
            )
            .await
            .unwrap();
        let failed = queued_conversion(&repository, "list-failed", "e").await;
        repository
            .finish_failed(
                failed.id,
                failed.active_attempt.id,
                failed_result(FailureStage::Queued, "source_integrity_failed"),
            )
            .await
            .unwrap();
        let gave_up = converting_conversion(&repository, "list-gave-up", "f").await;
        repository
            .finish_gave_up(
                gave_up.id,
                gave_up.active_attempt.id,
                scanned_analysis(),
                failed_result(FailureStage::ConvertingLocal, "scanned_pdf").failure,
            )
            .await
            .unwrap();

        let candidates = repository.list_recovery_candidates().await.unwrap();
        let loaded = candidates
            .iter()
            .map(|candidate| match candidate {
                RecoveryCandidate::Loaded(conversion) => conversion.as_ref(),
                RecoveryCandidate::Undecodable { id, reason } => {
                    panic!("row {id} should decode: {reason}")
                }
            })
            .collect::<Vec<_>>();
        let candidate_ids = loaded
            .iter()
            .map(|conversion| conversion.id)
            .collect::<HashSet<_>>();
        assert_eq!(
            candidate_ids,
            HashSet::from([queued.id, converting.id, finalizing.id, succeeded.id])
        );
        assert_eq!(
            loaded
                .iter()
                .find(|conversion| conversion.id == succeeded.id)
                .unwrap()
                .artifacts
                .len(),
            1
        );

        let all_ids = repository.list_conversion_ids().await.unwrap();
        assert_eq!(
            all_ids,
            HashSet::from([
                queued.id,
                converting.id,
                finalizing.id,
                succeeded.id,
                failed.id,
                gave_up.id,
            ])
        );
        repository.health_check().await.unwrap();
    }

    #[tokio::test]
    async fn failed_requeue_insert_rolls_back_interruption_and_preserves_foreign_keys() {
        let (_directory, repository) = open_repository(3).await;
        let recovering = converting_conversion(&repository, "rollback-requeue", "a").await;
        let other = queued_conversion(&repository, "rollback-other", "b").await;

        assert!(matches!(
            repository
                .interrupt_and_requeue(
                    recovering.id,
                    recovering.active_attempt.id,
                    other.active_attempt.id,
                    3,
                )
                .await,
            Err(RepositoryError::Database(_))
        ));
        let unchanged = repository.get(recovering.id).await.unwrap().unwrap();
        assert_eq!(unchanged.state, ConversionState::ConvertingLocal);
        assert_eq!(unchanged.active_attempt.id, recovering.active_attempt.id);
        assert_eq!(
            unchanged.active_attempt.state,
            AttemptState::ConvertingLocal
        );
        let attempt_count: i64 =
            sqlx::query("SELECT COUNT(*) AS attempt_count FROM attempts WHERE conversion_id = ?1")
                .bind(recovering.id.hyphenated().to_string())
                .fetch_one(&repository.pool)
                .await
                .unwrap()
                .try_get("attempt_count")
                .unwrap();
        assert_eq!(attempt_count, 1);
        assert!(sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&repository.pool)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn health_check_requires_a_live_read_write_pool() {
        let (_directory, repository) = open_repository(2).await;
        repository.health_check().await.unwrap();
        repository.pool.close().await;
        assert!(matches!(
            repository.health_check().await,
            Err(RepositoryError::Database(sqlx::Error::PoolClosed))
        ));
    }

    async fn open_repository(max_active_jobs: u32) -> (TempDir, SqliteRepository) {
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), max_active_jobs, BUSY_TIMEOUT)
            .await
            .unwrap();
        (directory, repository)
    }

    async fn assert_pragmas(connection: &mut SqliteConnection) {
        let journal_mode: String = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&mut *connection)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        let foreign_keys: i64 = sqlx::query("PRAGMA foreign_keys")
            .fetch_one(&mut *connection)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        let synchronous: i64 = sqlx::query("PRAGMA synchronous")
            .fetch_one(&mut *connection)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        let busy_timeout: i64 = sqlx::query("PRAGMA busy_timeout")
            .fetch_one(&mut *connection)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        assert_eq!(journal_mode, "wal");
        assert_eq!(foreign_keys, 1);
        assert_eq!(synchronous, 2);
        assert_eq!(busy_timeout, 5_000);
    }

    fn new_conversion(key: &str, fingerprint_character: &str) -> NewConversion {
        new_conversion_with_ids(key, fingerprint_character, Uuid::new_v4(), Uuid::new_v4())
    }

    async fn queued_conversion(
        repository: &SqliteRepository,
        key: &str,
        fingerprint_character: &str,
    ) -> crate::persistence::StoredConversion {
        created(
            repository
                .create_or_replay(new_conversion(key, fingerprint_character))
                .await
                .unwrap(),
        )
    }

    /// A queued conversion walked to converting_local, the state most of the
    /// transition tests start from.
    async fn converting_conversion(
        repository: &SqliteRepository,
        key: &str,
        fingerprint_character: &str,
    ) -> crate::persistence::StoredConversion {
        let conversion = queued_conversion(repository, key, fingerprint_character).await;
        repository
            .start_local(conversion.id, conversion.active_attempt.id, local_start())
            .await
            .unwrap();
        conversion
    }

    fn created(outcome: CreateOutcome) -> crate::persistence::StoredConversion {
        match outcome {
            CreateOutcome::Created(conversion) => conversion,
            outcome => panic!("expected created conversion, got {outcome:?}"),
        }
    }

    async fn futures_in_order(
        tasks: Vec<tokio::task::JoinHandle<CreateOutcome>>,
    ) -> Vec<CreateOutcome> {
        let mut outcomes = Vec::with_capacity(tasks.len());
        for task in tasks {
            outcomes.push(task.await.unwrap());
        }
        outcomes
    }

    fn new_conversion_with_ids(
        key: &str,
        fingerprint_character: &str,
        id: Uuid,
        attempt_id: Uuid,
    ) -> NewConversion {
        NewConversion {
            id,
            initial_attempt_id: attempt_id,
            client_run_id: Uuid::nil(),
            idempotency_key_sha256: hash_idempotency_key(key),
            request_fingerprint: fingerprint_character.repeat(64),
            source: NewSource {
                relative_path: format!("jobs/{id}/source/input"),
                media_type: "application/pdf".to_owned(),
                byte_length: 128,
                sha256: "f".repeat(64),
            },
            origin_request_id: Uuid::new_v4().to_string(),
            ocr_language_correction: true,
            ocr_custom_words: String::new(),
            speaker_count: None,
            speech_locale: None,
        }
    }

    fn local_start() -> LocalStart {
        LocalStart {
            engine: EngineRecord {
                name: "pdf-inspector".to_owned(),
                version: "1.25.2".to_owned(),
            },
            route: "local_pdf".to_owned(),
        }
    }

    fn local_analysis() -> LocalAnalysis {
        LocalAnalysis {
            classification: DocumentClassification::TextBased,
            inspection: serde_json::json!({"pageCount": 1}),
            reason_codes: vec!["native_text_pdf".to_owned()],
            warnings: vec![],
        }
    }

    fn scanned_analysis() -> LocalAnalysis {
        LocalAnalysis {
            classification: DocumentClassification::Scanned,
            inspection: serde_json::json!({"pageCount": 1}),
            reason_codes: vec!["scanned_pdf".to_owned()],
            warnings: vec![],
        }
    }

    fn failed_result(stage: FailureStage, code: &str) -> FailedResult {
        FailedResult {
            stage,
            failure: StoredFailure {
                code: code.to_owned(),
                message: "The conversion could not be completed.".to_owned(),
            },
        }
    }

    fn markdown_artifact(conversion_id: Uuid, attempt_id: Uuid) -> NewArtifact {
        NewArtifact {
            relative_path: format!(
                "jobs/{conversion_id}/attempts/{attempt_id}/artifacts/result.md"
            ),
            media_type: "text/markdown; charset=utf-8".to_owned(),
            byte_length: MARKDOWN.0,
            sha256: MARKDOWN.1.to_owned(),
        }
    }
}
