use std::{path::Path, time::Duration};

use sha2::{Digest, Sha256};
use sqlx::{
    migrate::MigrateError,
    sqlite::{
        SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions, SqliteRow,
        SqliteSynchronous,
    },
    Row, SqlitePool,
};
use thiserror::Error;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

use super::model::{
    ArtifactKind, AttemptState, ConversionState, CreateOutcome, EngineRecord, NewConversion,
    Profile, StoredArtifact, StoredAttempt, StoredConversion, StoredFailure, StoredSource,
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
            .map_err(RepositoryError::CommitOutcomeUncertain)?;
        Ok(CreateOutcome::Created(created))
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
    #[error("a committed conversion could not be read back")]
    MissingCommittedConversion,
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    /// SQLite may report an I/O error while committing without giving the
    /// caller enough information to decide whether durable ownership changed.
    /// Callers must reconcile by idempotency key and must not blindly delete
    /// the submitted source when this variant is returned.
    #[error("database commit outcome is uncertain; reconcile before cleanup")]
    CommitOutcomeUncertain(#[source] sqlx::Error),
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
        hash_idempotency_key, SqliteRepository, AUTH_SCOPE, DATABASE_FILENAME, MAX_POOL_CONNECTIONS,
    };
    use crate::persistence::{
        AttemptState, ConversionState, CreateOutcome, NewConversion, NewSource, Profile,
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
}
