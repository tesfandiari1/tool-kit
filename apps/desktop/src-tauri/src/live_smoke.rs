//! Ignored live smoke tests against real conversion providers and services.
//!
//! These hit the network and spend real API credits, so they are `#[ignore]`d.
//! Run explicitly with keys in the environment:
//!
//! ```sh
//! DATALAB_API_KEY=... REVAI_API_KEY=... \
//!   cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib live_smoke -- --ignored --nocapture
//! ```
//!
//! The native conversion-service compatibility smoke is deliberately explicit:
//!
//! ```sh
//! CONVERSION_SERVICE_BASE_URL=http://127.0.0.1:8080 \
//! CONVERSION_SERVICE_TOKEN=... \
//! CONVERSION_SERVICE_SOURCE=apps/converter/tests/fixtures/anydoc/text.docx \
//!   cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib \
//!     live_smoke::conversion_service_compatibility_live -- --ignored --nocapture
//! ```
//!
//! `TEST_PDF` / `TEST_AUDIO` override the direct-provider input paths.

use crate::conversion_service;
use crate::providers::{self, PollResult};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

struct TempOutput(PathBuf);

impl TempOutput {
    fn create() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tool-kit-conversion-service-live-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&path).expect("create isolated conversion-service output directory");
        Self(path)
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        // The UUID-named directory is the only path this test creates or removes.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn conversion_details(job: &conversion_service::ConversionJob) -> String {
    let route = job
        .route
        .as_ref()
        .map_or("none", |route| route.kind.as_str());
    let reasons = job
        .route
        .as_ref()
        .map(|route| route.reason_codes.join(","))
        .unwrap_or_default();
    let failure = job.failure.as_ref().map_or_else(
        || "none".to_string(),
        |failure| format!("{}: {}", failure.code, failure.message),
    );
    format!(
        "status={} route={route} reasonCodes=[{reasons}] warnings=[{}] failure={failure}",
        job.status,
        job.warnings.join(",")
    )
}

#[tokio::test]
#[ignore = "hits a live conversion service; set CONVERSION_SERVICE_BASE_URL, CONVERSION_SERVICE_TOKEN, and CONVERSION_SERVICE_SOURCE"]
async fn conversion_service_compatibility_live() {
    let base_url = std::env::var("CONVERSION_SERVICE_BASE_URL")
        .expect("set CONVERSION_SERVICE_BASE_URL to an explicit HTTP(S) origin");
    let token = std::env::var("CONVERSION_SERVICE_TOKEN")
        .expect("set CONVERSION_SERVICE_TOKEN for the target service");
    let source_path = std::env::var("CONVERSION_SERVICE_SOURCE")
        .expect("set CONVERSION_SERVICE_SOURCE to a readable local document");
    let source = Path::new(&source_path);
    assert!(
        source.is_file(),
        "conversion source does not exist: {source_path}"
    );
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .expect("conversion source must have a UTF-8 filename");
    let media_type = mime_guess::from_path(source)
        .first_or_octet_stream()
        .essence_str()
        .to_string();

    let capabilities = conversion_service::fetch_capabilities(&base_url)
        .await
        .expect("fetch unauthenticated conversion-service capabilities");
    assert!(
        capabilities
            .input_formats
            .iter()
            .any(|advertised| advertised.eq_ignore_ascii_case(&media_type)),
        "source MIME {media_type} is not advertised by the conversion service; advertised={:?}",
        capabilities.input_formats
    );
    assert!(
        capabilities.accepting_jobs,
        "conversion service advertises the source MIME but is not accepting jobs"
    );

    let client_run_id = uuid::Uuid::new_v4().to_string();
    let idempotency_key = uuid::Uuid::new_v4().to_string();
    let mut job = conversion_service::submit_conversion(
        &base_url,
        &token,
        &source_path,
        &client_run_id,
        "standard",
        &conversion_service::OcrOptions {
            language_correction: true,
            custom_words: Vec::new(),
            speaker_count: None,
        },
        &idempotency_key,
    )
    .await
    .expect("submit conversion through the native multipart helper");

    for _ in 0..240 {
        match job.status.as_str() {
            "succeeded" => {
                let output = TempOutput::create();
                let path = conversion_service::download_markdown(
                    &base_url,
                    &token,
                    &job.id,
                    &output.0.to_string_lossy(),
                    file_name,
                )
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "download Markdown through the native streaming helper failed: {error}; {}",
                        conversion_details(&job)
                    )
                });

                // The native helper exposes only a filesystem path, never the
                // artifact body. Verify the streamed file rather than reading it
                // into the test process as a response payload.
                let artifact = Path::new(&path);
                assert_eq!(artifact.parent(), Some(output.0.as_path()));
                let metadata = std::fs::metadata(artifact)
                    .expect("download helper returned a path that does not exist");
                assert!(metadata.is_file(), "downloaded artifact is not a file");
                assert!(metadata.len() > 0, "downloaded Markdown is empty");
                return;
            }
            "failed" => panic!(
                "conversion service returned a terminal failure: {}",
                conversion_details(&job)
            ),
            "needs_remote" => panic!(
                "conversion requires remote fallback in the local compatibility smoke: {}",
                conversion_details(&job)
            ),
            // Known pending states and future statuses are both pending. Only
            // the explicit allowlist above is terminal.
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        job = conversion_service::poll_conversion(&base_url, &token, &job.id)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "poll conversion through the native helper failed: {error}; {}",
                    conversion_details(&job)
                )
            });
    }

    panic!(
        "conversion service did not reach a terminal state within 60 seconds: {}",
        conversion_details(&job)
    );
}

#[tokio::test]
#[ignore = "hits the live Datalab API; run with DATALAB_API_KEY set"]
async fn datalab_convert_live() {
    let key = std::env::var("DATALAB_API_KEY").expect("set DATALAB_API_KEY");
    let path = env_or("TEST_PDF", "/tmp/toolkit-test/sample.pdf");
    let client = reqwest::Client::new();

    let submitted = providers::datalab_submit(&client, &key, &path, "markdown", true)
        .await
        .expect("datalab submit");
    let check_url = submitted.check_url.clone().unwrap_or_else(|| {
        format!(
            "https://www.datalab.to/api/v1/convert/{}",
            submitted.remote_id
        )
    });

    let mut markdown = None;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        match providers::datalab_poll(&client, &key, &check_url, "markdown")
            .await
            .expect("datalab poll")
        {
            PollResult::Done(t) => {
                markdown = Some(t);
                break;
            }
            PollResult::Failed(e) => panic!("datalab failed: {e}"),
            PollResult::Pending => {}
        }
    }

    let markdown = markdown.expect("datalab timed out");
    let preview: String = markdown.chars().take(900).collect();
    println!("\n===== DATALAB MARKDOWN ({} chars) =====\n{preview}\n=========================================\n", markdown.len());
    assert!(!markdown.trim().is_empty(), "markdown should not be empty");
}

#[tokio::test]
#[ignore = "hits the live Rev.ai API; run with REVAI_API_KEY set"]
async fn revai_transcribe_live() {
    let key = std::env::var("REVAI_API_KEY").expect("set REVAI_API_KEY");
    let path = env_or("TEST_AUDIO", "/tmp/toolkit-test/sample.wav");
    let client = reqwest::Client::new();

    let submitted = providers::revai_submit(&client, &key, &path)
        .await
        .expect("revai submit");

    let mut transcript = None;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        match providers::revai_poll(&client, &key, &submitted.remote_id)
            .await
            .expect("revai poll")
        {
            PollResult::Done(t) => {
                transcript = Some(t);
                break;
            }
            PollResult::Failed(e) => panic!("revai failed: {e}"),
            PollResult::Pending => {}
        }
    }

    let transcript = transcript.expect("revai timed out");
    println!("\n===== REV.AI TRANSCRIPT ({} chars) =====\n{transcript}\n=========================================\n", transcript.len());
    assert!(
        !transcript.trim().is_empty(),
        "transcript should not be empty"
    );
}
