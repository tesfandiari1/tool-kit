//! M4 routing regression suite (CVR-045).
//!
//! Every case drives the real HTTP surface and the real worker: submit, wait
//! for terminal, then read back what a client would actually see (status, route,
//! warnings, the artifact list, and the Markdown bytes themselves).
//!
//! The policy has unit tests over synthetic `QualitySignals`. Those cannot catch
//! the live defect, because the defect was that a 90%-native PDF reaches the
//! policy looking clean. Only running the engine over real bytes proves the
//! measurement the policy reads is the measurement the engine took.

#![allow(clippy::unwrap_used)]

mod support;

use axum::http::StatusCode;
use http_body_util::BodyExt;
use serde_json::Value;
use uuid::Uuid;

use support::{corpus, count_named_files, json_body, multipart_body, test_app, TestApp, TOKEN};

/// Everything a client can observe about one finished conversion.
struct Outcome {
    status: String,
    route_kind: String,
    reason_codes: Vec<String>,
    warnings: Vec<String>,
    failure_code: Option<String>,
    artifact_kinds: Vec<String>,
    /// `None` when the Markdown route answers 404, which is the only other
    /// answer this suite tolerates.
    markdown: Option<String>,
    manifest: Option<Value>,
}

async fn convert(app: &TestApp, profile: &str, key: &str, source: &[u8]) -> Outcome {
    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), profile, source, "corpus.pdf"),
            key,
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED, "{key}");
    let job_id = json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let completed = app.wait_for_terminal(&job_id).await;
    let data = &completed["data"];

    let artifacts = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
        .await;
    assert_eq!(artifacts.status(), StatusCode::OK, "{key}");
    let artifact_kinds = strings(&json_body(artifacts).await["data"], "kind");

    let response = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    let markdown = match response.status() {
        StatusCode::OK => Some(read_text(response).await),
        StatusCode::NOT_FOUND => None,
        other => panic!("{key}: markdown answered {other}"),
    };

    let response = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/manifest"))
        .await;
    let manifest = match response.status() {
        StatusCode::OK => Some(serde_json::from_str(&read_text(response).await).unwrap()),
        StatusCode::NOT_FOUND => None,
        other => panic!("{key}: manifest answered {other}"),
    };

    Outcome {
        status: data["status"].as_str().unwrap().to_owned(),
        route_kind: data["route"]["kind"].as_str().unwrap().to_owned(),
        reason_codes: strings(&data["route"]["reasonCodes"], ""),
        warnings: strings(&data["warnings"], ""),
        failure_code: data["failure"]["code"].as_str().map(str::to_owned),
        artifact_kinds,
        markdown,
        manifest,
    }
}

/// Reads a JSON array as strings. A non-empty `field` picks one key out of each
/// object instead.
fn strings(value: &Value, field: &str) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let leaf = if field.is_empty() {
                entry
            } else {
                &entry[field]
            };
            leaf.as_str().unwrap().to_owned()
        })
        .collect()
}

async fn read_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The published-clean shape: both artifacts, downloadable Markdown, no caveat.
fn assert_published(outcome: &Outcome, key: &str, warnings: &[&str]) {
    assert_eq!(outcome.status, "succeeded", "{key}");
    assert_eq!(outcome.route_kind, "local_pdf", "{key}");
    assert_eq!(outcome.failure_code, None, "{key}");
    assert_eq!(outcome.warnings, warnings, "{key}");
    assert_eq!(outcome.artifact_kinds, ["markdown", "manifest"], "{key}");
    let markdown = outcome.markdown.as_deref().unwrap_or_else(|| {
        panic!("{key}: published without downloadable Markdown");
    });
    assert!(!markdown.trim().is_empty(), "{key}");
    // The durable record has to carry the same caveat the API reports; a
    // warning that survives only in the response is a warning a re-read loses.
    let manifest = outcome.manifest.as_ref().unwrap();
    assert_eq!(strings(&manifest["warnings"], ""), warnings, "{key}");
}

/// The routed-away shape: nothing published, nothing on disk to publish later.
fn assert_nothing_published(outcome: &Outcome, key: &str) {
    assert!(outcome.artifact_kinds.is_empty(), "{key}");
    assert!(outcome.markdown.is_none(), "{key}");
    assert!(outcome.manifest.is_none(), "{key}");
}

#[tokio::test]
async fn a_fully_native_pdf_publishes_clean_under_both_local_profiles() {
    for profile in ["standard", "local_only"] {
        let app = test_app().await;
        for pages in [1, 4, 10] {
            let key = format!("native-{profile}-{pages}");
            let outcome = convert(&app, profile, &key, &corpus::native_pdf(pages)).await;
            assert_eq!(outcome.reason_codes, ["native_text_pdf"], "{key}");
            assert_published(&outcome, &key, &[]);
            let markdown = outcome.markdown.unwrap();
            // Every page's text has to survive, not just the first one.
            for page in 0..pages {
                assert!(
                    markdown.contains(&format!("Sheet {page} ")),
                    "{key}: page {page} is missing"
                );
            }
        }
    }
}

/// The live defect, end to end, and the correction the review forced.
///
/// A PDF that is 90% native text converted, published, and reported
/// `succeeded` with no warnings, with the image page's content simply absent.
/// It must never publish silently again.
///
/// It must also never be routed remote on this signal. `native_text_ratio` is
/// the share of pages carrying extractable text, not a completeness measure,
/// and `a_sparse_cover_page_is_not_missing_content` is the proof: an ordinary
/// report with a one-line cover page and no images anywhere reports the same
/// 0.9 and loses nothing. Routing on it would bill Datalab for that document.
#[tokio::test]
async fn a_partly_scanned_pdf_publishes_with_a_warning_under_every_profile() {
    let app = test_app().await;
    for profile in ["standard", "local_only"] {
        for text_pages in [2, 3, 4, 9] {
            let key = format!("partly-{profile}-{text_pages}");
            let outcome =
                convert(&app, profile, &key, &corpus::partly_scanned_pdf(text_pages)).await;
            assert_eq!(outcome.reason_codes, ["native_text_pdf"], "{key}");
            // Downloadable, not merely listed: a warning over zero bytes is its
            // own bug.
            assert_published(&outcome, &key, &["pages_without_extractable_text"]);
            let markdown = outcome.markdown.clone().unwrap();
            for page in 0..text_pages {
                assert!(
                    markdown.contains(&format!("Sheet {page} ")),
                    "{key}: page {page} is missing"
                );
            }
        }
    }
}

/// Why the warning above is a warning and not a route.
///
/// Ten pages, no images anywhere, and the only unusual thing is a cover page
/// holding one title line. The engine reports the same 0.9 it reports for a
/// document with a scanned page, and the Markdown contains every word,
/// including the cover. A policy that routed on this signal would send this
/// document to a paid provider for nothing.
#[tokio::test]
async fn a_sparse_cover_page_is_not_missing_content() {
    let app = test_app().await;
    let outcome = convert(&app, "standard", "cover", &corpus::sparse_cover_pdf(9)).await;
    assert_eq!(outcome.reason_codes, ["native_text_pdf"], "cover");
    assert_published(&outcome, "cover", &["pages_without_extractable_text"]);
    let markdown = outcome.markdown.unwrap();
    assert!(
        markdown.contains("Annual Report 2026"),
        "the cover page's text is in the Markdown, so nothing was lost"
    );
    for page in 0..9 {
        assert!(
            markdown.contains(&format!("Sheet {page} ")),
            "page {page} missing"
        );
    }
}

/// A single native page plus a single image page classifies as `mixed`, so the
/// engine gives up before producing Markdown. `local_only` cannot publish what
/// was never written.
#[tokio::test]
async fn a_half_scanned_pdf_needs_remote_under_both_profiles() {
    for profile in ["standard", "local_only"] {
        let app = test_app().await;
        let key = format!("mixed-{profile}");
        let outcome = convert(&app, profile, &key, &corpus::partly_scanned_pdf(1)).await;
        assert_eq!(outcome.status, "needs_remote", "{key}");
        assert_eq!(outcome.reason_codes, ["mixed_pdf"], "{key}");
        assert_eq!(outcome.warnings, [] as [&str; 0], "{key}");
        assert_nothing_published(&outcome, &key);
    }
}

#[tokio::test]
async fn complex_layouts_warn_without_changing_the_route() {
    for profile in ["standard", "local_only"] {
        let app = test_app().await;

        let key = format!("table-{profile}");
        let outcome = convert(&app, profile, &key, &corpus::dense_table_pdf()).await;
        assert_eq!(outcome.reason_codes, ["native_text_pdf"], "{key}");
        // Ruled cells read as columns as well as tables: aligned cell text is
        // literally a multi-column layout.
        assert_published(&outcome, &key, &["dense_tables", "multi_column_layout"]);
        assert!(outcome.markdown.unwrap().contains("R0 C0"), "{key}");

        let key = format!("columns-{profile}");
        let outcome = convert(&app, profile, &key, &corpus::two_column_pdf()).await;
        assert_eq!(outcome.reason_codes, ["native_text_pdf"], "{key}");
        assert_published(&outcome, &key, &["multi_column_layout"]);
        assert!(
            outcome.markdown.unwrap().contains("Column 1 line 0"),
            "{key}"
        );
    }
}

/// When the engine gives up, its own reason reaches the client verbatim and the
/// profile does not soften it: `local_only` has no partial Markdown to publish.
#[tokio::test]
async fn a_pdf_the_engine_gave_up_on_needs_remote_and_keeps_its_reason() {
    for profile in ["standard", "local_only"] {
        let app = test_app().await;
        for (name, source, reason) in [
            ("image-only-1", corpus::image_only_pdf(1), "scanned_pdf"),
            ("image-only-3", corpus::image_only_pdf(3), "scanned_pdf"),
            ("blank-content", corpus::blank_content_pdf(), "scanned_pdf"),
            ("garbled-font", corpus::garbled_font_pdf(), "ocr_required"),
            ("acroform", corpus::form_pdf(), "ocr_required"),
        ] {
            let key = format!("{name}-{profile}");
            let outcome = convert(&app, profile, &key, &source).await;
            assert_eq!(outcome.status, "needs_remote", "{key}");
            assert_eq!(outcome.route_kind, "local_pdf", "{key}");
            assert_eq!(outcome.reason_codes, [reason], "{key}");
            assert_eq!(outcome.warnings, [] as [&str; 0], "{key}");
            assert_eq!(outcome.failure_code, None, "{key}");
            assert_nothing_published(&outcome, &key);
        }
        assert_eq!(count_named_files(app.data_dir(), "result.md"), 0);
    }
}

/// A rejection is a failure, not a route: there is no remote leg that would fix
/// an encrypted or truncated file, so `needs_remote` would be a lie.
#[tokio::test]
async fn a_rejected_pdf_fails_without_publishing() {
    for profile in ["standard", "local_only"] {
        let app = test_app().await;
        for (name, source, code) in [
            ("encrypted", corpus::encrypted_pdf(), "encrypted_pdf"),
            (
                "truncated",
                corpus::truncated_pdf(),
                "invalid_pdf_structure",
            ),
        ] {
            let key = format!("{name}-{profile}");
            let outcome = convert(&app, profile, &key, &source).await;
            assert_eq!(outcome.status, "failed", "{key}");
            assert_eq!(outcome.failure_code.as_deref(), Some(code), "{key}");
            assert_eq!(outcome.reason_codes, [] as [&str; 0], "{key}");
            assert_eq!(outcome.warnings, [] as [&str; 0], "{key}");
            assert_nothing_published(&outcome, &key);
        }
    }
}
