use std::{path::Path, time::Duration};

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
use uuid::Uuid;

use super::model::{
    ArtifactKind, AttemptState, CommitOperation, ConversionState, CreateOutcome, EngineRecord,
    FailedResult, LocalAnalysis, LocalStart, NeedsRemoteResult, NewArtifact, NewConversion,
    Profile, StoredArtifact, StoredAttempt, StoredConversion, StoredFailure, StoredSource,
    SuccessfulArtifacts,
};

pub const DATABASE_FILENAME: &str = "converter.sqlite";
const AUTH_SCOPE: &str = "bootstrap";
const MAX_POOL_CONNECTIONS: u32 = 4;

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

        let active_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS active_count
             FROM conversions
             WHERE status IN ('queued', 'converting_local', 'finalizing')",
        )
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
                request_fingerprint, profile, status, source_relative_path,
                source_media_type, source_byte_length, source_sha256,
                reason_codes_json, warnings_json,
                origin_request_id, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7, ?8, ?9, ?10,
                '[]', '[]', ?11, ?12, ?12
             )",
        )
        .bind(&conversion_id)
        .bind(&client_run_id)
        .bind(AUTH_SCOPE)
        .bind(&input.idempotency_key_sha256)
        .bind(&input.request_fingerprint)
        .bind(input.profile.as_str())
        .bind(&input.source.relative_path)
        .bind(&input.source.media_type)
        .bind(source_byte_length)
        .bind(&input.source.sha256)
        .bind(&input.origin_request_id)
        .bind(&created_at)
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

    pub async fn claim_next_queued(
        &self,
        start: LocalStart,
    ) -> Result<Option<StoredConversion>, RepositoryError> {
        validate_local_start(&start)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let next = sqlx::query(
            "SELECT c.id AS conversion_id, a.id AS attempt_id
             FROM conversions AS c
             JOIN attempts AS a
               ON a.conversion_id = c.id
              AND a.id = c.active_attempt_id
             WHERE c.auth_scope = ?1
               AND c.status = 'queued'
               AND a.state = 'queued'
             ORDER BY a.queue_seq ASC
             LIMIT 1",
        )
        .bind(AUTH_SCOPE)
        .fetch_optional(&mut *transaction)
        .await?;

        let Some(next) = next else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let conversion_id = parse_uuid(next.try_get("conversion_id")?, "conversions.id")?;
        let attempt_id = parse_uuid(next.try_get("attempt_id")?, "attempts.id")?;
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
    ) -> Result<StoredConversion, RepositoryError> {
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

    pub async fn finish_needs_remote(
        &self,
        conversion_id: Uuid,
        attempt_id: Uuid,
        result: NeedsRemoteResult,
    ) -> Result<StoredConversion, RepositoryError> {
        validate_bounded_text(
            &result.fallback_reason,
            1,
            128,
            "fallback reason is invalid",
        )?;
        let analysis = encode_local_analysis(&result.analysis)?;
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
            AttemptState::NeedsRemote,
            &analysis,
            Some(&result.fallback_reason),
            &updated_at,
        )
        .await?;
        update_conversion_analysis(
            &mut transaction,
            conversion_id,
            attempt_id,
            ConversionState::ConvertingLocal,
            ConversionState::NeedsRemote,
            &analysis,
            &updated_at,
        )
        .await?;

        let conversion = load_required_conversion(&mut transaction, conversion_id).await?;
        commit_transition(
            transaction,
            CommitOperation::FinishNeedsRemote,
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
        let updated_at = now_rfc3339()?;

        let attempt = sqlx::query(
            "UPDATE attempts
             SET state = 'failed', failure_code = ?1, failure_message = ?2,
                 updated_at = ?3, finished_at = ?3
             WHERE conversion_id = ?4 AND id = ?5 AND state = ?6",
        )
        .bind(&result.failure.code)
        .bind(&result.failure.message)
        .bind(&updated_at)
        .bind(conversion_id.hyphenated().to_string())
        .bind(attempt_id.hyphenated().to_string())
        .bind(expected_attempt.as_str())
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(attempt.rows_affected(), "attempts.state")?;

        let conversion = sqlx::query(
            "UPDATE conversions
             SET status = 'failed', failure_code = ?1, failure_message = ?2,
                 updated_at = ?3
             WHERE id = ?4 AND auth_scope = ?5 AND active_attempt_id = ?6
               AND status = ?7",
        )
        .bind(&result.failure.code)
        .bind(&result.failure.message)
        .bind(&updated_at)
        .bind(conversion_id.hyphenated().to_string())
        .bind(AUTH_SCOPE)
        .bind(attempt_id.hyphenated().to_string())
        .bind(expected_conversion.as_str())
        .execute(&mut *transaction)
        .await?;
        require_one_transition_row(conversion.rows_affected(), "conversions.status")?;

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
        artifacts: SuccessfulArtifacts,
    ) -> Result<StoredConversion, RepositoryError> {
        let markdown_length = validate_artifact(
            conversion_id,
            attempt_id,
            ArtifactKind::Markdown,
            &artifacts.markdown,
        )?;
        let manifest_length = validate_artifact(
            conversion_id,
            attempt_id,
            ArtifactKind::Manifest,
            &artifacts.manifest,
        )?;
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
             ) VALUES
                (?1, 'markdown', ?2, ?3, ?4, ?5, ?6),
                (?1, 'manifest', ?7, ?8, ?9, ?10, ?6)",
        )
        .bind(&attempt_id_text)
        .bind(&artifacts.markdown.relative_path)
        .bind(&artifacts.markdown.media_type)
        .bind(markdown_length)
        .bind(&artifacts.markdown.sha256)
        .bind(&updated_at)
        .bind(&artifacts.manifest.relative_path)
        .bind(&artifacts.manifest.media_type)
        .bind(manifest_length)
        .bind(&artifacts.manifest.sha256)
        .execute(&mut *transaction)
        .await?;
        if inserted.rows_affected() != 2 {
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
        let active_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS active_count
             FROM conversions
             WHERE status IN ('queued', 'converting_local', 'finalizing')",
        )
        .fetch_one(&self.pool)
        .await?
        .try_get("active_count")?;
        Ok(active_count < i64::from(self.max_active_jobs))
    }
}

#[derive(Debug)]
struct EncodedLocalAnalysis {
    classification: &'static str,
    inspection_json: String,
    reason_codes_json: String,
    warnings_json: String,
}

#[derive(Clone, Copy, Debug)]
struct ActiveIds {
    conversion_id: Uuid,
    attempt_id: Uuid,
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
    let row = sqlx::query(
        "SELECT c.active_attempt_id, c.status,
                (SELECT a.state
                 FROM attempts AS a
                 WHERE a.conversion_id = c.id AND a.id = c.active_attempt_id)
                    AS attempt_state
         FROM conversions AS c
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

    let actual_conversion = ConversionState::from_database(&row.try_get::<String, _>("status")?)
        .ok_or(RepositoryError::CorruptData("conversions.status"))?;
    let actual_attempt = AttemptState::from_database(
        &row.try_get::<Option<String>, _>("attempt_state")?
            .ok_or(RepositoryError::CorruptData("attempts.state"))?,
    )
    .ok_or(RepositoryError::CorruptData("attempts.state"))?;
    if actual_conversion != expected_conversion || actual_attempt != expected_attempt {
        return Err(RepositoryError::IllegalTransition {
            conversion_id,
            attempt_id,
            expected_conversion,
            expected_attempt,
            actual_conversion,
            actual_attempt,
        });
    }
    Ok(())
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
        .bind(target_state.as_str())
        .bind(analysis.classification)
        .bind(&analysis.inspection_json)
        .bind(&analysis.reason_codes_json)
        .bind(&analysis.warnings_json)
        .bind(fallback_reason)
        .bind(updated_at)
        .bind(&conversion_id)
        .bind(&attempt_id)
        .bind(expected_state.as_str())
        .execute(&mut **transaction)
        .await?
    } else {
        sqlx::query(
            "UPDATE attempts
             SET state = ?1, classification = ?2, inspection_json = ?3,
                 reason_codes_json = ?4, warnings_json = ?5, updated_at = ?6
             WHERE conversion_id = ?7 AND id = ?8 AND state = ?9",
        )
        .bind(target_state.as_str())
        .bind(analysis.classification)
        .bind(&analysis.inspection_json)
        .bind(&analysis.reason_codes_json)
        .bind(&analysis.warnings_json)
        .bind(updated_at)
        .bind(&conversion_id)
        .bind(&attempt_id)
        .bind(expected_state.as_str())
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
    .bind(target_state.as_str())
    .bind(&analysis.reason_codes_json)
    .bind(&analysis.warnings_json)
    .bind(updated_at)
    .bind(conversion_id.hyphenated().to_string())
    .bind(AUTH_SCOPE)
    .bind(attempt_id.hyphenated().to_string())
    .bind(expected_state.as_str())
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
    validate_bounded_text(&start.engine.name, 1, 128, "engine name is invalid")?;
    validate_bounded_text(&start.engine.version, 1, 128, "engine version is invalid")?;
    validate_bounded_text(&start.route, 1, 128, "route is invalid")
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
        false,
        "reason codes are invalid",
    )?;
    let warnings_json = encode_string_array(
        &analysis.warnings,
        0,
        1_024,
        65_536,
        false,
        "warnings are invalid",
    )?;
    Ok(EncodedLocalAnalysis {
        classification: analysis.classification.as_str(),
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
    allow_empty: bool,
    message: &'static str,
) -> Result<String, RepositoryError> {
    if values.len() < minimum_items
        || values.len() > 256
        || values.iter().any(|value| {
            (!allow_empty && value.is_empty())
                || value.len() > maximum_item_length
                || value.chars().any(char::is_control)
        })
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
    validate_bounded_text(&failure.code, 1, 128, "failure code is invalid")?;
    validate_bounded_text(&failure.message, 1, 1_024, "failure message is invalid")
}

fn validate_artifact(
    conversion_id: Uuid,
    attempt_id: Uuid,
    kind: ArtifactKind,
    artifact: &NewArtifact,
) -> Result<i64, RepositoryError> {
    let file_name = match kind {
        ArtifactKind::Markdown => "result.md",
        ArtifactKind::Manifest => "manifest.json",
    };
    let expected_path = format!(
        "jobs/{}/attempts/{}/artifacts/{file_name}",
        conversion_id.hyphenated(),
        attempt_id.hyphenated()
    );
    if artifact.relative_path != expected_path {
        return Err(RepositoryError::InvalidInput(
            "artifact path does not belong to the active attempt",
        ));
    }
    let expected_media_type = match kind {
        ArtifactKind::Markdown => "text/markdown; charset=utf-8",
        ArtifactKind::Manifest => "application/json",
    };
    if artifact.media_type != expected_media_type {
        return Err(RepositoryError::InvalidInput(
            "artifact media type is invalid",
        ));
    }
    validate_sha256(&artifact.sha256, "artifact hash is invalid")?;
    if artifact.byte_length == 0 || artifact.byte_length > i64::MAX as u64 {
        return Err(RepositoryError::InvalidInput(
            "artifact byte length is invalid",
        ));
    }
    i64::try_from(artifact.byte_length)
        .map_err(|_| RepositoryError::InvalidInput("artifact byte length is invalid"))
}

fn validate_bounded_text(
    value: &str,
    minimum_length: usize,
    maximum_length: usize,
    message: &'static str,
) -> Result<(), RepositoryError> {
    if value.len() < minimum_length
        || value.len() > maximum_length
        || value.chars().any(char::is_control)
    {
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
            c.profile,
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
        "SELECT attempt_id, kind, relative_path, media_type, byte_length,
                sha256, created_at
         FROM artifacts
         WHERE attempt_id = ?1
         ORDER BY kind",
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
    if input.source.media_type != "application/pdf" {
        return Err(RepositoryError::InvalidInput(
            "source media type is unsupported",
        ));
    }
    if input.source.byte_length == 0 || input.source.byte_length > i64::MAX as u64 {
        return Err(RepositoryError::InvalidInput(
            "source byte length is invalid",
        ));
    }
    if input.origin_request_id.is_empty()
        || input.origin_request_id.len() > 128
        || input.origin_request_id.chars().any(char::is_control)
    {
        return Err(RepositoryError::InvalidInput(
            "origin request id is invalid",
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str, message: &'static str) -> Result<(), RepositoryError> {
    if value.len() != 64
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
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
        profile: Profile::from_database(&row.try_get::<String, _>("profile")?)
            .ok_or(RepositoryError::CorruptData("conversions.profile"))?,
        state: ConversionState::from_database(&row.try_get::<String, _>("conversion_state")?)
            .ok_or(RepositoryError::CorruptData("conversions.status"))?,
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
            state: AttemptState::from_database(&row.try_get::<String, _>("attempt_state")?)
                .ok_or(RepositoryError::CorruptData("attempts.state"))?,
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
    })
}

fn decode_artifact(row: &SqliteRow) -> Result<StoredArtifact, RepositoryError> {
    Ok(StoredArtifact {
        attempt_id: parse_uuid(row.try_get("attempt_id")?, "artifacts.attempt_id")?,
        kind: ArtifactKind::from_database(&row.try_get::<String, _>("kind")?)
            .ok_or(RepositoryError::CorruptData("artifacts.kind"))?,
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
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use sqlx::Row;
    use tempfile::TempDir;
    use tokio::sync::Barrier;
    use uuid::Uuid;

    use super::{
        hash_idempotency_key, RepositoryError, SqliteRepository, AUTH_SCOPE, DATABASE_FILENAME,
        MAX_POOL_CONNECTIONS,
    };
    use crate::persistence::{
        ArtifactKind, AttemptState, ConversionState, CreateOutcome, DocumentClassification,
        EngineRecord, FailedResult, FailureStage, LocalAnalysis, LocalStart, NeedsRemoteResult,
        NewArtifact, NewConversion, NewSource, Profile, StoredFailure, SuccessfulArtifacts,
    };

    const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn migrations_configure_a_file_backed_database_and_repeat_cleanly() {
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 4, BUSY_TIMEOUT)
            .await
            .unwrap();

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

        let journal_mode: String = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&repository.pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        let foreign_keys: i64 = sqlx::query("PRAGMA foreign_keys")
            .fetch_one(&repository.pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        let synchronous: i64 = sqlx::query("PRAGMA synchronous")
            .fetch_one(&repository.pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        let busy_timeout: i64 = sqlx::query("PRAGMA busy_timeout")
            .fetch_one(&repository.pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
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
        assert_eq!(journal_mode, "wal");
        assert_eq!(foreign_keys, 1);
        assert_eq!(synchronous, 2);
        assert_eq!(busy_timeout, 5_000);
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
            let journal_mode: String = sqlx::query("PRAGMA journal_mode")
                .fetch_one(&mut **connection)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
            let foreign_keys: i64 = sqlx::query("PRAGMA foreign_keys")
                .fetch_one(&mut **connection)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
            let synchronous: i64 = sqlx::query("PRAGMA synchronous")
                .fetch_one(&mut **connection)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
            let busy_timeout: i64 = sqlx::query("PRAGMA busy_timeout")
                .fetch_one(&mut **connection)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
            assert_eq!(journal_mode, "wal");
            assert_eq!(foreign_keys, 1);
            assert_eq!(synchronous, 2);
            assert_eq!(busy_timeout, 5_000);
        }
        drop(connections);

        drop(repository);
        SqliteRepository::open(directory.path(), 4, BUSY_TIMEOUT)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_get_replay_conflict_and_capacity_are_durable() {
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 1, BUSY_TIMEOUT)
            .await
            .unwrap();
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
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 16, BUSY_TIMEOUT)
            .await
            .unwrap();
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
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 1, BUSY_TIMEOUT)
            .await
            .unwrap();
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
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 1, BUSY_TIMEOUT)
            .await
            .unwrap();
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
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 2, BUSY_TIMEOUT)
            .await
            .unwrap();
        let first = created(
            repository
                .create_or_replay(new_conversion("first", "a"))
                .await
                .unwrap(),
        );
        let second = created(
            repository
                .create_or_replay(new_conversion("second", "b"))
                .await
                .unwrap(),
        );

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
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 2, BUSY_TIMEOUT)
            .await
            .unwrap();
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
    async fn legal_success_path_commits_both_artifacts_and_integrity_failure_retains_audit_rows() {
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 2, BUSY_TIMEOUT)
            .await
            .unwrap();
        let queued = created(
            repository
                .create_or_replay(new_conversion("success", "a"))
                .await
                .unwrap(),
        );
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
            .mark_finalizing(conversion_id, attempt_id, local_analysis())
            .await
            .unwrap();
        assert_eq!(finalizing.state, ConversionState::Finalizing);
        assert_eq!(finalizing.active_attempt.state, AttemptState::Finalizing);
        assert_eq!(
            finalizing.active_attempt.classification.as_deref(),
            Some("text_based")
        );
        assert_eq!(finalizing.reason_codes, vec!["native_text_pdf"]);

        let succeeded = repository
            .finish_succeeded(
                conversion_id,
                attempt_id,
                successful_artifacts(conversion_id, attempt_id),
            )
            .await
            .unwrap();
        assert_eq!(succeeded.state, ConversionState::Succeeded);
        assert_eq!(succeeded.active_attempt.state, AttemptState::Succeeded);
        assert!(succeeded.active_attempt.finished_at.is_some());
        assert_eq!(succeeded.artifacts.len(), 2);
        assert!(succeeded
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == ArtifactKind::Markdown));
        assert!(succeeded
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == ArtifactKind::Manifest));

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
        assert_eq!(integrity_failed.artifacts.len(), 2);
        assert_eq!(
            integrity_failed.failure.unwrap().code,
            "artifact_integrity_failed"
        );
    }

    #[tokio::test]
    async fn terminal_outcomes_require_their_exact_legal_source_state() {
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 8, BUSY_TIMEOUT)
            .await
            .unwrap();

        let queued_failure = created(
            repository
                .create_or_replay(new_conversion("queued-failure", "a"))
                .await
                .unwrap(),
        );
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

        let remote = created(
            repository
                .create_or_replay(new_conversion("needs-remote", "b"))
                .await
                .unwrap(),
        );
        repository
            .start_local(remote.id, remote.active_attempt.id, local_start())
            .await
            .unwrap();
        let remote = repository
            .finish_needs_remote(
                remote.id,
                remote.active_attempt.id,
                NeedsRemoteResult {
                    analysis: LocalAnalysis {
                        classification: DocumentClassification::Scanned,
                        inspection: serde_json::json!({"pageCount": 2}),
                        reason_codes: vec!["scanned_pdf".to_owned()],
                        warnings: vec![],
                    },
                    fallback_reason: "scanned_pdf".to_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(remote.state, ConversionState::NeedsRemote);
        assert_eq!(remote.active_attempt.state, AttemptState::NeedsRemote);
        assert_eq!(
            remote.active_attempt.fallback_reason.as_deref(),
            Some("scanned_pdf")
        );

        let converting_failure = created(
            repository
                .create_or_replay(new_conversion("converting-failure", "c"))
                .await
                .unwrap(),
        );
        repository
            .start_local(
                converting_failure.id,
                converting_failure.active_attempt.id,
                local_start(),
            )
            .await
            .unwrap();
        let failed = repository
            .finish_failed(
                converting_failure.id,
                converting_failure.active_attempt.id,
                failed_result(FailureStage::ConvertingLocal, "engine_failed"),
            )
            .await
            .unwrap();
        assert_eq!(failed.state, ConversionState::Failed);

        let finalizing_failure = created(
            repository
                .create_or_replay(new_conversion("finalizing-failure", "d"))
                .await
                .unwrap(),
        );
        repository
            .start_local(
                finalizing_failure.id,
                finalizing_failure.active_attempt.id,
                local_start(),
            )
            .await
            .unwrap();
        repository
            .mark_finalizing(
                finalizing_failure.id,
                finalizing_failure.active_attempt.id,
                local_analysis(),
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
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 2, BUSY_TIMEOUT)
            .await
            .unwrap();
        let queued = created(
            repository
                .create_or_replay(new_conversion("transition-guards", "a"))
                .await
                .unwrap(),
        );

        assert!(matches!(
            repository
                .start_local(queued.id, Uuid::new_v4(), local_start())
                .await,
            Err(RepositoryError::StaleActiveAttempt { .. })
        ));
        assert!(matches!(
            repository
                .mark_finalizing(queued.id, queued.active_attempt.id, local_analysis())
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
                    successful_artifacts(queued.id, queued.active_attempt.id),
                )
                .await,
            Err(RepositoryError::IllegalTransition { .. })
        ));
        let converting = repository.get(queued.id).await.unwrap().unwrap();
        assert_eq!(converting.state, ConversionState::ConvertingLocal);
        assert!(converting.artifacts.is_empty());
    }

    #[tokio::test]
    async fn concurrent_fifo_claims_take_each_queued_attempt_once_in_order() {
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 4, BUSY_TIMEOUT)
            .await
            .unwrap();
        let first = created(
            repository
                .create_or_replay(new_conversion("fifo-first", "a"))
                .await
                .unwrap(),
        );
        let second = created(
            repository
                .create_or_replay(new_conversion("fifo-second", "b"))
                .await
                .unwrap(),
        );
        let third = created(
            repository
                .create_or_replay(new_conversion("fifo-third", "c"))
                .await
                .unwrap(),
        );
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
                    .claim_next_queued(local_start())
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
            .claim_next_queued(local_start())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(last.id, third.id);
        assert!(repository
            .claim_next_queued(local_start())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn concurrent_exact_start_allows_one_transition_and_rejects_the_stale_caller() {
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 2, BUSY_TIMEOUT)
            .await
            .unwrap();
        let queued = created(
            repository
                .create_or_replay(new_conversion("concurrent-start", "a"))
                .await
                .unwrap(),
        );
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
    async fn artifact_insert_failure_rolls_back_both_rows_and_success_transition() {
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 2, BUSY_TIMEOUT)
            .await
            .unwrap();
        let conversion = created(
            repository
                .create_or_replay(new_conversion("artifact-atomic", "a"))
                .await
                .unwrap(),
        );
        repository
            .start_local(conversion.id, conversion.active_attempt.id, local_start())
            .await
            .unwrap();
        repository
            .mark_finalizing(
                conversion.id,
                conversion.active_attempt.id,
                local_analysis(),
            )
            .await
            .unwrap();

        sqlx::query(
            "CREATE TRIGGER reject_manifest
             BEFORE INSERT ON artifacts
             WHEN NEW.kind = 'manifest'
             BEGIN SELECT RAISE(ABORT, 'test manifest failure'); END",
        )
        .execute(&repository.pool)
        .await
        .unwrap();
        assert!(matches!(
            repository
                .finish_succeeded(
                    conversion.id,
                    conversion.active_attempt.id,
                    successful_artifacts(conversion.id, conversion.active_attempt.id),
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
        let directory = TempDir::new().unwrap();
        let repository = SqliteRepository::open(directory.path(), 2, BUSY_TIMEOUT)
            .await
            .unwrap();
        let conversion = created(
            repository
                .create_or_replay(new_conversion("transition-atomic", "a"))
                .await
                .unwrap(),
        );
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

    fn new_conversion(key: &str, fingerprint_character: &str) -> NewConversion {
        new_conversion_with_ids(key, fingerprint_character, Uuid::new_v4(), Uuid::new_v4())
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
            profile: Profile::Standard,
            source: NewSource {
                relative_path: format!("jobs/{id}/source/input"),
                media_type: "application/pdf".to_owned(),
                byte_length: 128,
                sha256: "f".repeat(64),
            },
            origin_request_id: Uuid::new_v4().to_string(),
        }
    }

    fn local_start() -> LocalStart {
        LocalStart {
            engine: EngineRecord {
                name: "pdf-inspector".to_owned(),
                version: "1.15.0".to_owned(),
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

    fn failed_result(stage: FailureStage, code: &str) -> FailedResult {
        FailedResult {
            stage,
            failure: StoredFailure {
                code: code.to_owned(),
                message: "The conversion could not be completed.".to_owned(),
            },
        }
    }

    fn successful_artifacts(conversion_id: Uuid, attempt_id: Uuid) -> SuccessfulArtifacts {
        let base = format!("jobs/{conversion_id}/attempts/{attempt_id}/artifacts");
        SuccessfulArtifacts {
            markdown: NewArtifact {
                relative_path: format!("{base}/result.md"),
                media_type: "text/markdown; charset=utf-8".to_owned(),
                byte_length: 32,
                sha256: "a".repeat(64),
            },
            manifest: NewArtifact {
                relative_path: format!("{base}/manifest.json"),
                media_type: "application/json".to_owned(),
                byte_length: 64,
                sha256: "b".repeat(64),
            },
        }
    }
}
