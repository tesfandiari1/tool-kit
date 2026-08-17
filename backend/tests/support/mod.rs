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
use tempfile::TempDir;
use tokio_util::io::ReaderStream;
use tower::ServiceExt;
use uuid::Uuid;

use tool_kit_converter::{
    config::{Limits, Settings},
    router, AppState,
};

pub(crate) const TOKEN: &str = "0123456789abcdef0123456789abcdef";

#[derive(Clone, Debug)]
struct TestOptions {
    worker_timeout: Duration,
    max_upload_bytes: u64,
    max_output_bytes: u64,
    max_jobs: usize,
    max_concurrent_uploads: usize,
}

impl Default for TestOptions {
    fn default() -> Self {
        Self {
            worker_timeout: Duration::from_secs(10),
            max_upload_bytes: 1024 * 1024,
            max_output_bytes: 2 * 1024 * 1024,
            max_jobs: 8,
            max_concurrent_uploads: 2,
        }
    }
}

/// Owns a reusable filesystem root independently from any one `AppState`.
///
/// M1 initializes a fresh ephemeral artifact session beneath this root for each
/// app. M2 can change only `settings()` to point repeated app initializations at
/// the same durable database and artifact directory.
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

    #[cfg(unix)]
    pub(crate) fn with_worker_script(script: &str, worker_timeout: Duration) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let mut harness = Self::new();
        let worker_path = harness.workspace.path().join("fake-worker");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'tool-kit-pdf-worker protocol=1 pdf-inspector=1.15.0'\n  exit 0\nfi\n{}",
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

    pub(crate) async fn app(&self) -> TestApp {
        let state = AppState::initialize(&self.settings()).await.unwrap();
        TestApp {
            router: router(state),
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
            limits: Limits {
                max_upload_bytes: self.options.max_upload_bytes,
                max_output_bytes: self.options.max_output_bytes,
                max_jobs: self.options.max_jobs,
                max_concurrent_uploads: self.options.max_concurrent_uploads,
                upload_timeout: Duration::from_secs(5),
                pdf_timeout: self.options.worker_timeout,
            },
            pdf_threads: 2,
            database_busy_timeout: Duration::from_secs(5),
            worker_poll_interval: Duration::from_secs(1),
            recovery_limit: 3,
            shutdown_grace: Duration::from_secs(30),
        }
    }
}

pub(crate) struct TestApp {
    router: Router,
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
    format!(
        "--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"clientRunId\"\r\n\r\n{client_run_id}\r\n--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\nstandard\r\n--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"source\"; filename=\"slow.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.4\n"
    )
    .into_bytes()
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
