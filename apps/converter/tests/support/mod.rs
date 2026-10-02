// Several test binaries include this module and each one uses a subset of it.
#![allow(dead_code)]

use std::{
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    body::Body,
    http::{header::AUTHORIZATION, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteConnectOptions, Connection, Row, SqliteConnection};
use tempfile::TempDir;
use tokio_util::io::ReaderStream;
use tower::ServiceExt;
use uuid::Uuid;

use tool_kit_converter::{
    config::{Limits, Settings},
    faults::FaultBarrier,
    persistence::{
        hash_idempotency_key, CreateOutcome, NewConversion, NewSource, Profile, SqliteRepository,
        DATABASE_FILENAME,
    },
    router, AppState,
};

pub(crate) mod corpus;

pub(crate) const TOKEN: &str = "0123456789abcdef0123456789abcdef";

#[derive(Clone, Debug)]
struct TestOptions {
    worker_timeout: Duration,
    max_upload_bytes: u64,
    max_audio_upload_bytes: u64,
    max_output_bytes: u64,
    max_jobs: usize,
    max_concurrent_uploads: usize,
    worker_poll_interval: Duration,
    recovery_limit: usize,
    vision_worker_path: Option<PathBuf>,
    audio_worker_path: Option<PathBuf>,
    audio_diarizer_dir: Option<PathBuf>,
}

impl Default for TestOptions {
    fn default() -> Self {
        Self {
            worker_timeout: Duration::from_secs(10),
            max_upload_bytes: 1024 * 1024,
            max_audio_upload_bytes: 1024 * 1024,
            max_output_bytes: 2 * 1024 * 1024,
            max_jobs: 8,
            max_concurrent_uploads: 2,
            worker_poll_interval: Duration::from_secs(1),
            recovery_limit: 3,
            vision_worker_path: None,
            audio_worker_path: None,
            audio_diarizer_dir: None,
        }
    }
}

/// The Swift audio worker and its staged diarizer models, on the
/// `audio_worker.rs` shape: both are built by `workers/audio/build.sh` into
/// gitignored directories, so a fresh checkout and Linux CI find neither and
/// the caller skips.
pub(crate) fn audio_tools() -> Option<(PathBuf, PathBuf)> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../workers/audio");
    let worker = root.join("bin/tool-kit-audio-worker");
    let diarizer = root.join("models/speaker-diarization-coreml");
    (worker.is_file() && diarizer.is_dir()).then_some((worker, diarizer))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SeededJob {
    pub(crate) job_id: Uuid,
    pub(crate) attempt_id: Uuid,
}

/// Owns one reusable durable data root independently from any `AppState`.
/// Repeated app initializations therefore exercise the same SQLite database,
/// immutable sources, and published artifact trees.
pub(crate) struct TestHarness {
    workspace: Arc<TempDir>,
    data_dir: PathBuf,
    worker_path: PathBuf,
    options: TestOptions,
}

impl TestHarness {
    pub(crate) fn new() -> Self {
        let workspace = Arc::new(tempfile::tempdir().unwrap());
        let data_dir = workspace.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        std::fs::write(
            workspace.path().join("bootstrap-token"),
            format!("{TOKEN}\n"),
        )
        .unwrap();

        Self {
            workspace,
            data_dir,
            worker_path: PathBuf::from(env!("CARGO_BIN_EXE_tool-kit-pdf-worker")),
            options: TestOptions::default(),
        }
    }

    pub(crate) fn with_output_limit(max_output_bytes: u64) -> Self {
        let mut harness = Self::new();
        harness.options.max_output_bytes = max_output_bytes;
        harness
    }

    pub(crate) fn with_max_jobs(max_jobs: usize) -> Self {
        let mut harness = Self::new();
        harness.options.max_jobs = max_jobs;
        harness
    }

    pub(crate) fn with_upload_limits(max_upload_bytes: u64, max_concurrent_uploads: usize) -> Self {
        let mut harness = Self::new();
        harness.options.max_upload_bytes = max_upload_bytes;
        harness.options.max_concurrent_uploads = max_concurrent_uploads;
        harness
    }

    pub(crate) fn with_poll_interval(worker_poll_interval: Duration) -> Self {
        let mut harness = Self::new();
        harness.options.worker_poll_interval = worker_poll_interval;
        harness
    }

    /// `None` where this machine has no audio worker to run.
    pub(crate) fn with_audio_worker() -> Option<Self> {
        let (worker, diarizer) = audio_tools()?;
        let mut harness = Self::new();
        harness.options.audio_worker_path = Some(worker);
        harness.options.audio_diarizer_dir = Some(diarizer);
        Some(harness)
    }

    /// `None` where this machine has no Vision worker built. The timeout
    /// covers Vision's cold model load, which runs past the default.
    pub(crate) fn with_vision_worker() -> Option<Self> {
        let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../workers/vision/bin/tool-kit-vision-worker");
        if !cfg!(target_os = "macos") || !worker.is_file() {
            return None;
        }
        let mut harness = Self::new();
        harness.options.vision_worker_path = Some(worker);
        harness.options.worker_timeout = Duration::from_secs(120);
        Some(harness)
    }

    /// Chains onto `with_audio_worker`: the audio ceiling only means anything
    /// on a host that advertises audio at all.
    pub(crate) fn with_upload_ceilings(
        mut self,
        max_upload_bytes: u64,
        max_audio_upload_bytes: u64,
    ) -> Self {
        self.options.max_upload_bytes = max_upload_bytes;
        self.options.max_audio_upload_bytes = max_audio_upload_bytes;
        self
    }

    pub(crate) fn with_recovery_limit(recovery_limit: usize) -> Self {
        let mut harness = Self::new();
        harness.options.recovery_limit = recovery_limit;
        harness
    }

    #[cfg(unix)]
    pub(crate) fn with_worker_script(script: &str, worker_timeout: Duration) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let mut harness = Self::new();
        let worker_path = harness.workspace.path().join("fake-worker");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'tool-kit-pdf-worker protocol=2 pdf-inspector=1.25.2'\n  exit 0\nfi\ncat >/dev/null\n{}",
            script.strip_prefix("#!/bin/sh\n").unwrap_or(script)
        );
        std::fs::write(&worker_path, script).unwrap();
        std::fs::set_permissions(&worker_path, std::fs::Permissions::from_mode(0o700)).unwrap();
        harness.worker_path = worker_path;
        harness.options.worker_timeout = worker_timeout;
        harness
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub(crate) async fn insert_queued_job(&self, source: &[u8]) -> SeededJob {
        let job_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let source_directory = self
            .data_dir
            .join("jobs")
            .join(job_id.to_string())
            .join("source");
        let attempt_directory = self
            .data_dir
            .join("jobs")
            .join(job_id.to_string())
            .join("attempts")
            .join(attempt_id.to_string());
        std::fs::create_dir_all(&source_directory).unwrap();
        std::fs::create_dir_all(&attempt_directory).unwrap();
        std::fs::write(source_directory.join("input"), source).unwrap();

        let repository = SqliteRepository::open(
            &self.data_dir,
            self.options.max_jobs as u32,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        let outcome = repository
            .create_or_replay(NewConversion {
                id: job_id,
                initial_attempt_id: attempt_id,
                client_run_id: Uuid::new_v4(),
                idempotency_key_sha256: hash_idempotency_key(&format!("missed-notify-{job_id}")),
                request_fingerprint: hex::encode(Sha256::digest(job_id.as_bytes())),
                profile: Profile::Standard,
                source: NewSource {
                    relative_path: format!("jobs/{job_id}/source/input"),
                    media_type: "application/pdf".to_owned(),
                    byte_length: source.len() as u64,
                    sha256: hex::encode(Sha256::digest(source)),
                },
                origin_request_id: Uuid::new_v4().to_string(),
                ocr_language_correction: true,
                ocr_custom_words: String::new(),
                speaker_count: None,
            })
            .await
            .unwrap();
        assert!(matches!(outcome, CreateOutcome::Created(_)));
        SeededJob { job_id, attempt_id }
    }

    pub(crate) async fn insert_converting_job(
        &self,
        source: &[u8],
        recovery_count: u32,
    ) -> SeededJob {
        use tool_kit_converter::persistence::{EngineRecord, LocalStart};

        let seeded = self.insert_queued_job(source).await;
        let repository = SqliteRepository::open(
            &self.data_dir,
            self.options.max_jobs as u32,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        repository
            .start_local(
                seeded.job_id,
                seeded.attempt_id,
                LocalStart {
                    engine: EngineRecord {
                        name: "pdf-inspector".to_owned(),
                        version: "1.25.2".to_owned(),
                    },
                    route: "local_pdf".to_owned(),
                },
            )
            .await
            .unwrap();
        drop(repository);

        if recovery_count > 0 {
            let mut connection = self.database_connection().await;
            let updated = sqlx::query(
                "UPDATE attempts
                 SET recovery_count = ?1
                 WHERE conversion_id = ?2 AND id = ?3",
            )
            .bind(i64::from(recovery_count))
            .bind(seeded.job_id.hyphenated().to_string())
            .bind(seeded.attempt_id.hyphenated().to_string())
            .execute(&mut connection)
            .await
            .unwrap();
            assert_eq!(updated.rows_affected(), 1);
        }

        seeded
    }

    pub(crate) async fn demote_succeeded_to_finalizing(&self, seeded: SeededJob) {
        let mut connection = self.database_connection().await;
        let mut transaction = connection.begin().await.unwrap();
        let deleted = sqlx::query("DELETE FROM artifacts WHERE attempt_id = ?1")
            .bind(seeded.attempt_id.hyphenated().to_string())
            .execute(&mut *transaction)
            .await
            .unwrap();
        assert_eq!(deleted.rows_affected(), 2);
        let attempt = sqlx::query(
            "UPDATE attempts
             SET state = 'finalizing', finished_at = NULL
             WHERE conversion_id = ?1 AND id = ?2 AND state = 'succeeded'",
        )
        .bind(seeded.job_id.hyphenated().to_string())
        .bind(seeded.attempt_id.hyphenated().to_string())
        .execute(&mut *transaction)
        .await
        .unwrap();
        assert_eq!(attempt.rows_affected(), 1);
        let conversion = sqlx::query(
            "UPDATE conversions
             SET status = 'finalizing', failure_code = NULL, failure_message = NULL
             WHERE id = ?1 AND active_attempt_id = ?2 AND status = 'succeeded'",
        )
        .bind(seeded.job_id.hyphenated().to_string())
        .bind(seeded.attempt_id.hyphenated().to_string())
        .execute(&mut *transaction)
        .await
        .unwrap();
        assert_eq!(conversion.rows_affected(), 1);
        transaction.commit().await.unwrap();
        // A crash before the success commit also comes before the source is
        // removed, so the finalizing job a real crash leaves still has it.
        std::fs::write(
            self.data_dir
                .join("jobs")
                .join(seeded.job_id.to_string())
                .join("source/input"),
            clean_pdf(),
        )
        .unwrap();
    }

    /// Break one succeeded attempt's metadata invariant without breaking a
    /// CHECK: startup recovery requires a classification it recognises.
    pub(crate) async fn clear_stored_classification(&self, seeded: SeededJob) {
        let mut connection = self.database_connection().await;
        let updated = sqlx::query(
            "UPDATE attempts
             SET classification = NULL
             WHERE conversion_id = ?1 AND id = ?2 AND state = 'succeeded'",
        )
        .bind(seeded.job_id.hyphenated().to_string())
        .bind(seeded.attempt_id.hyphenated().to_string())
        .execute(&mut connection)
        .await
        .unwrap();
        assert_eq!(updated.rows_affected(), 1);
    }

    /// Make one succeeded row undecodable without breaking a CHECK. The reason
    /// codes column only has to be a JSON array, so an array of numbers is a
    /// legal row that will not decode as `Vec<String>`. That is a row-level
    /// decode failure, which is a different bug from a row that decodes and
    /// then fails invariant validation.
    pub(crate) async fn corrupt_stored_reason_codes(&self, seeded: SeededJob) {
        let mut connection = self.database_connection().await;
        let updated = sqlx::query(
            "UPDATE attempts
             SET reason_codes_json = json_array(1, 2)
             WHERE conversion_id = ?1 AND id = ?2 AND state = 'succeeded'",
        )
        .bind(seeded.job_id.hyphenated().to_string())
        .bind(seeded.attempt_id.hyphenated().to_string())
        .execute(&mut connection)
        .await
        .unwrap();
        assert_eq!(updated.rows_affected(), 1);
    }

    pub(crate) async fn attempt_count(&self, job_id: Uuid) -> i64 {
        let mut connection = self.database_connection().await;
        sqlx::query("SELECT COUNT(*) AS count FROM attempts WHERE conversion_id = ?1")
            .bind(job_id.hyphenated().to_string())
            .fetch_one(&mut connection)
            .await
            .unwrap()
            .try_get("count")
            .unwrap()
    }

    pub(crate) async fn artifact_row_count(&self, attempt_id: Uuid) -> i64 {
        let mut connection = self.database_connection().await;
        sqlx::query("SELECT COUNT(*) AS count FROM artifacts WHERE attempt_id = ?1")
            .bind(attempt_id.hyphenated().to_string())
            .fetch_one(&mut connection)
            .await
            .unwrap()
            .try_get("count")
            .unwrap()
    }

    /// Reads the durable status and active attempt straight from SQLite. A parked
    /// worker still serves HTTP, but a test that is asserting on the state a crash
    /// left behind must not read it through the code path it is testing.
    pub(crate) async fn stored_status(&self, job_id: Uuid) -> (String, Uuid) {
        let mut connection = self.database_connection().await;
        let row = sqlx::query("SELECT status, active_attempt_id FROM conversions WHERE id = ?1")
            .bind(job_id.hyphenated().to_string())
            .fetch_one(&mut connection)
            .await
            .unwrap();
        let status: String = row.try_get("status").unwrap();
        let active_attempt_id: String = row.try_get("active_attempt_id").unwrap();
        (status, Uuid::parse_str(&active_attempt_id).unwrap())
    }

    /// Every attempt's state in attempt order, so a test can prove the interrupted
    /// attempt survived instead of being overwritten by the recovery attempt.
    pub(crate) async fn attempt_states(&self, job_id: Uuid) -> Vec<String> {
        let mut connection = self.database_connection().await;
        sqlx::query(
            "SELECT state FROM attempts WHERE conversion_id = ?1 ORDER BY attempt_number ASC",
        )
        .bind(job_id.hyphenated().to_string())
        .fetch_all(&mut connection)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get("state").unwrap())
        .collect()
    }

    pub(crate) async fn inject_active_attempt_state(&self, job_id: Uuid, state: &str) {
        let options = SqliteConnectOptions::new()
            .filename(self.data_dir.join(DATABASE_FILENAME))
            .foreign_keys(true);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        let updated = sqlx::query(
            "UPDATE attempts
             SET state = ?1
             WHERE conversion_id = ?2
               AND id = (SELECT active_attempt_id FROM conversions WHERE id = ?2)",
        )
        .bind(state)
        .bind(job_id.hyphenated().to_string())
        .execute(&mut connection)
        .await
        .unwrap();
        assert_eq!(updated.rows_affected(), 1);
    }

    /// Runs one app against the empty data root so the durable layout and the
    /// schema exist before a test writes rows behind the service's back.
    pub(crate) async fn initialize_empty(&self) {
        self.app().await.shutdown(Duration::from_secs(1)).await;
    }

    pub(crate) async fn app(&self) -> TestApp {
        let state = AppState::initialize(&self.settings()).await.unwrap();
        TestApp {
            router: router(state.clone()),
            state,
            _workspace: Arc::clone(&self.workspace),
            data_dir: self.data_dir.clone(),
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            bind_address: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            log_filter: "tool_kit_converter=info".to_owned(),
            token_file: self.workspace.path().join("bootstrap-token"),
            data_dir: self.data_dir.clone(),
            scratch_parent: self.data_dir.clone(),
            pdf_worker_path: self.worker_path.clone(),
            pdf_bcmaps_dir: None,
            vision_worker_path: self.options.vision_worker_path.clone(),
            audio_worker_path: self.options.audio_worker_path.clone(),
            audio_diarizer_dir: self.options.audio_diarizer_dir.clone(),
            limits: Limits {
                max_upload_bytes: self.options.max_upload_bytes,
                max_audio_upload_bytes: self.options.max_audio_upload_bytes,
                max_output_bytes: self.options.max_output_bytes,
                max_jobs: self.options.max_jobs,
                max_concurrent_uploads: self.options.max_concurrent_uploads,
                upload_timeout: Duration::from_secs(5),
                pdf_timeout: self.options.worker_timeout,
                audio_timeout: self.options.worker_timeout,
            },
            pdf_threads: 2,
            database_busy_timeout: Duration::from_secs(5),
            worker_poll_interval: self.options.worker_poll_interval,
            recovery_limit: self.options.recovery_limit,
            shutdown_grace: Duration::from_secs(30),
            shutdown_on_stdin_eof: false,
        }
    }

    async fn database_connection(&self) -> SqliteConnection {
        let options = SqliteConnectOptions::new()
            .filename(self.data_dir.join(DATABASE_FILENAME))
            .foreign_keys(true);
        SqliteConnection::connect_with(&options).await.unwrap()
    }
}

pub(crate) struct TestApp {
    router: Router,
    state: AppState,
    _workspace: Arc<TempDir>,
    data_dir: PathBuf,
}

impl TestApp {
    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub(crate) fn router(&self) -> Router {
        self.router.clone()
    }

    pub(crate) fn fault_barrier(&self) -> Arc<FaultBarrier> {
        self.state.fault_barrier()
    }

    pub(crate) fn stop_job_claiming(&self) {
        self.state.stop_job_claiming();
    }

    pub(crate) fn force_cancel_jobs(&self) {
        self.state.force_cancel_jobs();
    }

    pub(crate) async fn shutdown(&self, grace: Duration) {
        self.state.shutdown_jobs(grace).await;
    }

    pub(crate) fn job_runner_failed(&self) -> bool {
        self.state.job_runner_failed()
    }

    pub(crate) fn job_runner_stopped(&self) -> bool {
        self.state.job_runner_stopped()
    }

    pub(crate) async fn wait_for_job_runner_idle(&self) {
        self.state.wait_for_job_runner_idle().await;
    }

    pub(crate) async fn wait_for_job_runner_exit(&self) -> bool {
        self.state.wait_for_job_runner_exit().await
    }

    pub(crate) async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Body>,
    ) -> axum::response::Response {
        self.request_with_headers(method, uri, body.unwrap_or_else(Body::empty), &[])
            .await
    }

    pub(crate) async fn request_with_headers(
        &self,
        method: Method,
        uri: &str,
        body: Body,
        headers: &[(&str, &str)],
    ) -> axum::response::Response {
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        self.router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }

    pub(crate) async fn submit(
        &self,
        body: Vec<u8>,
        idempotency_key: &str,
        token: &str,
    ) -> axum::response::Response {
        self.request_with_headers(
            Method::POST,
            "/api/v1/conversions",
            Body::from(body),
            &[
                (AUTHORIZATION.as_str(), &format!("Bearer {token}")),
                ("idempotency-key", idempotency_key),
                (
                    "content-type",
                    "multipart/form-data; boundary=tool-kit-boundary",
                ),
            ],
        )
        .await
    }

    pub(crate) async fn authorized_get(&self, uri: &str) -> axum::response::Response {
        self.request_with_headers(
            Method::GET,
            uri,
            Body::empty(),
            &[(AUTHORIZATION.as_str(), &format!("Bearer {TOKEN}"))],
        )
        .await
    }

    pub(crate) async fn wait_for_terminal(&self, job_id: &str) -> Value {
        for _ in 0..100 {
            let response = self
                .authorized_get(&format!("/api/v1/conversions/{job_id}"))
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            let payload = json_body(response).await;
            if matches!(
                payload["data"]["status"].as_str(),
                Some("succeeded" | "failed" | "needs_remote")
            ) {
                return payload;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("conversion did not reach a terminal state");
    }

    /// Submits one clean PDF under `standard` and returns the accepted ids.
    pub(crate) async fn submit_clean(&self, idempotency_key: &str) -> SeededJob {
        let response = self
            .submit(
                multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "fixture.pdf"),
                idempotency_key,
                TOKEN,
            )
            .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let accepted = json_body(response).await;
        SeededJob {
            job_id: Uuid::parse_str(accepted["data"]["id"].as_str().unwrap()).unwrap(),
            attempt_id: Uuid::parse_str(accepted["data"]["activeAttemptId"].as_str().unwrap())
                .unwrap(),
        }
    }

    /// Submits one clean PDF and returns once it has succeeded.
    pub(crate) async fn submit_succeeded_job(&self, idempotency_key: &str) -> SeededJob {
        let seeded = self.submit_clean(idempotency_key).await;
        let completed = self.wait_for_terminal(&seeded.job_id.to_string()).await;
        assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
        seeded
    }
}

pub(crate) async fn test_app() -> TestApp {
    TestHarness::new().app().await
}

pub(crate) async fn test_app_with_output_limit(max_output_bytes: u64) -> TestApp {
    TestHarness::with_output_limit(max_output_bytes).app().await
}

pub(crate) async fn test_app_with_max_jobs(max_jobs: usize) -> TestApp {
    TestHarness::with_max_jobs(max_jobs).app().await
}

pub(crate) async fn test_app_with_upload_limits(
    max_upload_bytes: u64,
    max_concurrent_uploads: usize,
) -> TestApp {
    TestHarness::with_upload_limits(max_upload_bytes, max_concurrent_uploads)
        .app()
        .await
}

pub(crate) async fn test_app_with_poll_interval(worker_poll_interval: Duration) -> TestApp {
    TestHarness::with_poll_interval(worker_poll_interval)
        .app()
        .await
}

#[cfg(unix)]
pub(crate) async fn test_app_with_worker_script(script: &str, worker_timeout: Duration) -> TestApp {
    TestHarness::with_worker_script(script, worker_timeout)
        .app()
        .await
}

pub(crate) async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

pub(crate) fn assert_server_request_id(response: &axum::response::Response) {
    let request_id = response.headers()["x-request-id"].to_str().unwrap();
    assert!(Uuid::parse_str(request_id).is_ok());
}

pub(crate) fn multipart_body(
    client_run_id: Uuid,
    profile: &str,
    source: &[u8],
    filename: &str,
) -> Vec<u8> {
    multipart_body_with_media_type(client_run_id, profile, source, filename, "application/pdf")
}

pub(crate) fn multipart_body_with_media_type(
    client_run_id: Uuid,
    profile: &str,
    source: &[u8],
    filename: &str,
    media_type: &str,
) -> Vec<u8> {
    let boundary = "tool-kit-boundary";
    let mut body = Vec::new();
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"clientRunId\"\r\n\r\n{client_run_id}\r\n"
    )
    .unwrap();
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\n{profile}\r\n"
    )
    .unwrap();
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"source\"; filename=\"{filename}\"\r\nContent-Type: {media_type}\r\n\r\n"
    )
    .unwrap();
    body.extend_from_slice(source);
    write!(body, "\r\n--{boundary}--\r\n").unwrap();
    body
}

/// `speaker_counts` are raw part values written verbatim, so a test can send
/// one the parser must refuse, or send the part twice.
pub(crate) fn multipart_body_with_speaker_counts(
    client_run_id: Uuid,
    source: &[u8],
    filename: &str,
    media_type: &str,
    speaker_counts: &[&str],
) -> Vec<u8> {
    let boundary = "tool-kit-boundary";
    let mut body = Vec::new();
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"clientRunId\"\r\n\r\n{client_run_id}\r\n"
    )
    .unwrap();
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\nstandard\r\n"
    )
    .unwrap();
    for count in speaker_counts {
        write!(
            body,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"speakerCount\"\r\n\r\n{count}\r\n"
        )
        .unwrap();
    }
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"source\"; filename=\"{filename}\"\r\nContent-Type: {media_type}\r\n\r\n"
    )
    .unwrap();
    body.extend_from_slice(source);
    write!(body, "\r\n--{boundary}--\r\n").unwrap();
    body
}

pub(crate) fn multipart_body_with_duplicate_profile(client_run_id: Uuid, source: &[u8]) -> Vec<u8> {
    let mut body = multipart_body(client_run_id, "standard", source, "fixture.pdf");
    let closing = b"--tool-kit-boundary--\r\n";
    body.truncate(body.len() - closing.len());
    body.extend_from_slice(
        b"--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\nlocal_only\r\n--tool-kit-boundary--\r\n",
    );
    body
}

pub(crate) fn multipart_body_without_source(client_run_id: Uuid, profile: &str) -> Vec<u8> {
    format!(
        "--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"clientRunId\"\r\n\r\n{client_run_id}\r\n--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\n{profile}\r\n--tool-kit-boundary--\r\n"
    )
    .into_bytes()
}

pub(crate) fn slow_multipart_prefix(client_run_id: Uuid) -> Vec<u8> {
    slow_multipart_prefix_opening(client_run_id, b"%PDF-1.4\n")
}

/// The same prefix with the source part's first bytes chosen by the caller,
/// so a test can open a still-running upload with content that can never be
/// admitted.
pub(crate) fn slow_multipart_prefix_opening(client_run_id: Uuid, opening: &[u8]) -> Vec<u8> {
    let mut prefix = format!(
        "--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"clientRunId\"\r\n\r\n{client_run_id}\r\n--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\nstandard\r\n--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"source\"; filename=\"slow.pdf\"\r\nContent-Type: application/pdf\r\n\r\n"
    )
    .into_bytes();
    prefix.extend_from_slice(opening);
    prefix
}

pub(crate) fn count_named_files(root: &Path, expected: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| {
            if entry.path().is_dir() {
                count_named_files(&entry.path(), expected)
            } else {
                usize::from(entry.file_name() == std::ffi::OsStr::new(expected))
            }
        })
        .sum()
}

pub(crate) fn count_job_directories(data_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(data_dir.join("jobs")) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path().is_dir()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| Uuid::parse_str(name).is_ok())
        })
        .count()
}

pub(crate) fn clean_pdf() -> Vec<u8> {
    let content = b"BT\n/F1 18 Tf\n72 720 Td\n(Clean PDF Fixture) Tj\n0 -24 Td\n(Second native text line) Tj\n0 -24 Td\n(Third native text line) Tj\nET\n";
    pdf_with_content(content)
}

pub(crate) fn pdf_with_content(content: &[u8]) -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}endstream",
            content.len(),
            String::from_utf8_lossy(content)
        )
        .into_bytes(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        writeln!(pdf, "{} 0 obj", index + 1).unwrap();
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    write!(pdf, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
    for offset in offsets {
        writeln!(pdf, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    pdf
}

pub(crate) fn streaming_body(reader: tokio::io::DuplexStream) -> Body {
    Body::from_stream(ReaderStream::new(reader))
}
