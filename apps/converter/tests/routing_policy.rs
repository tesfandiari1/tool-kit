//! M4 routing regression suite (CVR-045).
//!
//! Every case drives the real HTTP surface and the real worker: submit, wait
//! for terminal, then read back what a client would actually see (status, route,
//! warnings, and the Markdown bytes themselves).
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
    /// `None` when the Markdown route answers 404, which is the only other
    /// answer this suite tolerates.
    markdown: Option<String>,
}

/// Sends the retired `local_only` profile, as the desktop does.
async fn convert(app: &TestApp, key: &str, source: &[u8]) -> Outcome {
    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "local_only", source, "corpus.pdf"),
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

    let response = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    let markdown = match response.status() {
        StatusCode::OK => Some(read_text(response).await),
        StatusCode::NOT_FOUND => None,
        other => panic!("{key}: markdown answered {other}"),
    };

    Outcome {
        status: data["status"].as_str().unwrap().to_owned(),
        route_kind: data["route"]["kind"].as_str().unwrap().to_owned(),
        reason_codes: strings(&data["route"]["reasonCodes"]),
        warnings: strings(&data["warnings"]),
        failure_code: data["failure"]["code"].as_str().map(str::to_owned),
        markdown,
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.as_str().unwrap().to_owned())
        .collect()
}

async fn read_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The published-clean shape: downloadable Markdown and the expected caveats.
fn assert_published(outcome: &Outcome, key: &str, warnings: &[&str]) {
    assert_eq!(outcome.status, "succeeded", "{key}");
    assert_eq!(outcome.route_kind, "local_pdf", "{key}");
    assert_eq!(outcome.failure_code, None, "{key}");
    assert_eq!(outcome.warnings, warnings, "{key}");
    let markdown = outcome.markdown.as_deref().unwrap_or_else(|| {
        panic!("{key}: published without downloadable Markdown");
    });
    assert!(!markdown.trim().is_empty(), "{key}");
}

/// The routed-away shape: nothing published, nothing on disk to publish later.
fn assert_nothing_published(outcome: &Outcome, key: &str) {
    assert!(outcome.markdown.is_none(), "{key}");
}

#[tokio::test]
async fn a_fully_native_pdf_publishes_clean() {
    let app = test_app().await;
    for pages in [1, 4, 10] {
        let key = format!("native-{pages}");
        let outcome = convert(&app, &key, &corpus::native_pdf(pages)).await;
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
async fn a_partly_scanned_pdf_publishes_with_a_warning() {
    let app = test_app().await;
    for text_pages in [2, 3, 4, 9] {
        let key = format!("partly-{text_pages}");
        let outcome = convert(&app, &key, &corpus::partly_scanned_pdf(text_pages)).await;
        assert_eq!(outcome.reason_codes, ["native_text_pdf"], "{key}");
        // Downloadable, not merely reported: a warning over zero bytes is its
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
    let outcome = convert(&app, "cover", &corpus::sparse_cover_pdf(9)).await;
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
/// engine gives up before producing Markdown, and the job fails.
#[tokio::test]
async fn a_half_scanned_pdf_fails_with_its_reason() {
    let app = test_app().await;
    let outcome = convert(&app, "mixed", &corpus::partly_scanned_pdf(1)).await;
    assert_eq!(outcome.status, "failed", "mixed");
    assert_eq!(outcome.failure_code.as_deref(), Some("mixed_pdf"), "mixed");
    assert_eq!(outcome.reason_codes, ["mixed_pdf"], "mixed");
    assert_eq!(outcome.warnings, [] as [&str; 0], "mixed");
    assert_nothing_published(&outcome, "mixed");
}

#[tokio::test]
async fn complex_layouts_warn_without_changing_the_route() {
    let app = test_app().await;

    let outcome = convert(&app, "table", &corpus::dense_table_pdf()).await;
    assert_eq!(outcome.reason_codes, ["native_text_pdf"], "table");
    // Ruled cells read as columns as well as tables: aligned cell text is
    // literally a multi-column layout.
    assert_published(&outcome, "table", &["dense_tables", "multi_column_layout"]);
    assert!(outcome.markdown.unwrap().contains("R0 C0"), "table");

    let outcome = convert(&app, "columns", &corpus::two_column_pdf()).await;
    assert_eq!(outcome.reason_codes, ["native_text_pdf"], "columns");
    assert_published(&outcome, "columns", &["multi_column_layout"]);
    assert!(
        outcome.markdown.unwrap().contains("Column 1 line 0"),
        "columns"
    );
}

/// When the engine gives up, the job fails and its own reason reaches the
/// client verbatim, as the failure code and as the route's reason.
#[tokio::test]
async fn a_pdf_the_engine_gave_up_on_fails_and_keeps_its_reason() {
    let app = test_app().await;
    for (name, source, reason) in [
        ("image-only-1", corpus::image_only_pdf(1), "scanned_pdf"),
        ("image-only-3", corpus::image_only_pdf(3), "scanned_pdf"),
        ("blank-content", corpus::blank_content_pdf(), "scanned_pdf"),
        ("garbled-font", corpus::garbled_font_pdf(), "ocr_required"),
        ("acroform", corpus::form_pdf(), "ocr_required"),
    ] {
        let outcome = convert(&app, name, &source).await;
        assert_eq!(outcome.status, "failed", "{name}");
        assert_eq!(outcome.route_kind, "local_pdf", "{name}");
        assert_eq!(outcome.reason_codes, [reason], "{name}");
        assert_eq!(outcome.warnings, [] as [&str; 0], "{name}");
        assert_eq!(outcome.failure_code.as_deref(), Some(reason), "{name}");
        assert_nothing_published(&outcome, name);
    }
    assert_eq!(count_named_files(app.data_dir(), "result.md"), 0);
}

/// A rejection fails with the worker's code and no engine reason: the file is
/// encrypted or truncated, which no engine would fix.
#[tokio::test]
async fn a_rejected_pdf_fails_without_publishing() {
    let app = test_app().await;
    for (name, source, code) in [
        ("encrypted", corpus::encrypted_pdf(), "encrypted_pdf"),
        (
            "truncated",
            corpus::truncated_pdf(),
            "invalid_pdf_structure",
        ),
    ] {
        let outcome = convert(&app, name, &source).await;
        assert_eq!(outcome.status, "failed", "{name}");
        assert_eq!(outcome.failure_code.as_deref(), Some(code), "{name}");
        assert_eq!(outcome.reason_codes, [] as [&str; 0], "{name}");
        assert_eq!(outcome.warnings, [] as [&str; 0], "{name}");
        assert_nothing_published(&outcome, name);
    }
}
