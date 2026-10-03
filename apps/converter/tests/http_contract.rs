#![allow(clippy::unwrap_used)]

mod support;

use std::{fs, time::Duration};

use axum::{
    body::Body,
    http::{header::AUTHORIZATION, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tower::ServiceExt;
use uuid::Uuid;

use support::{
    assert_server_request_id, clean_pdf, count_job_directories, count_named_files, json_body,
    multipart, multipart_body, pdf_with_content, slow_multipart_prefix, streaming_body, test_app,
    test_app_with_max_jobs, test_app_with_output_limit, test_app_with_poll_interval,
    test_app_with_upload_limits, test_app_with_worker_script, TestHarness, TOKEN,
};

#[tokio::test]
async fn public_health_and_capabilities_are_truthful() {
    let app = test_app().await;
    let response = app.request(Method::GET, "/health/live", None).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_server_request_id(&response);
    let payload = json_body(response).await;
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["service"], "tool-kit-converter");

    let response = app.request(Method::GET, "/api/v1/capabilities", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let payload = json_body(response).await;
    let conversion = &payload["data"]["conversion"];
    assert_eq!(conversion["acceptingJobs"], true);
    assert_eq!(
        conversion["inputFormats"],
        serde_json::json!([
            "application/pdf",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/msword",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.ms-powerpoint",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.ms-excel",
            "application/epub+zip",
            "application/vnd.oasis.opendocument.text",
            "application/vnd.oasis.opendocument.spreadsheet",
            "application/vnd.oasis.opendocument.presentation",
            "application/rtf",
            "text/csv",
            "application/vnd.ms-word.document.macroEnabled.12",
            "application/vnd.ms-excel.sheet.macroEnabled.12",
            "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
            "application/vnd.ms-powerpoint.slideshow.macroEnabled.12"
        ]),
        "the test app runs no Vision worker, so the image types are contract only"
    );
    let keys = |value: &Value| {
        value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&payload["data"]), ["conversion"]);
    assert_eq!(keys(conversion), ["acceptingJobs", "inputFormats"]);
}

#[tokio::test]
async fn harness_reinitializes_app_state_against_one_data_root_without_a_listener() {
    let harness = TestHarness::new();
    let data_dir = harness.data_dir().to_owned();

    let first = harness.app().await;
    assert_eq!(first.data_dir(), data_dir);
    assert_eq!(
        first
            .request(Method::GET, "/health/ready", None)
            .await
            .status(),
        StatusCode::OK
    );
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    let second = harness.app().await;
    assert_eq!(second.data_dir(), data_dir);
    assert_eq!(
        second
            .request(Method::GET, "/health/ready", None)
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn readiness_reports_every_dependency_check_on_a_healthy_app() {
    let app = test_app().await;
    let response = app.request(Method::GET, "/health/ready", None).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_server_request_id(&response);
    // Compared whole, so a leaked path, SQL string, or error detail on this
    // unauthenticated route fails the test instead of riding along.
    let payload = json_body(response).await;
    assert_eq!(
        payload,
        serde_json::json!({
            "status": "ready",
            "service": "tool-kit-converter",
            "serviceVersion": env!("CARGO_PKG_VERSION"),
            "checks": {
                "database": "ok",
                "dataRoot": "ok",
                "worker": "ok",
            },
        }),
        "{payload:#}"
    );
}

#[tokio::test]
async fn readiness_fails_only_the_worker_check_after_the_runner_stops() {
    let app = test_app().await;
    app.stop_job_claiming();
    assert!(
        !app.wait_for_job_runner_exit().await,
        "the runner must stop cleanly rather than fail"
    );
    assert!(app.job_runner_stopped());

    let response = app.request(Method::GET, "/health/ready", None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_server_request_id(&response);
    let payload = json_body(response).await;
    assert_eq!(payload["status"], "not_ready", "{payload:#}");
    assert_eq!(payload["checks"]["worker"], "failed", "{payload:#}");
    // SQLite and the data root outlive the runner, so a stopped worker must
    // not drag their answers down with it.
    assert_eq!(payload["checks"]["database"], "ok", "{payload:#}");
    assert_eq!(payload["checks"]["dataRoot"], "ok", "{payload:#}");
}

#[tokio::test]
async fn readiness_probes_the_data_root_without_touching_stored_conversions() {
    let app = test_app().await;
    let seeded = app.submit_succeeded_job("readiness-artifact-safety").await;
    let jobs = count_job_directories(app.data_dir());
    let sources = count_named_files(app.data_dir(), "input");
    let markdown = count_named_files(app.data_dir(), "result.md");
    let health = app.data_dir().join(".health");
    assert!(health.is_dir(), "readiness owns a probe directory on disk");
    // Removing the probe directory under a running service is the operator
    // case: readiness has to rebuild it, not latch as permanently unready.
    fs::remove_dir_all(&health).unwrap();

    for round in 0..2 {
        let response = app.request(Method::GET, "/health/ready", None).await;
        assert_eq!(response.status(), StatusCode::OK, "round {round}");
        assert_eq!(json_body(response).await["checks"]["dataRoot"], "ok");
        assert!(health.is_dir(), "round {round}: no probe directory");
        assert_eq!(
            fs::read_dir(&health).unwrap().count(),
            0,
            "round {round}: the probe left a file behind"
        );
    }

    assert_eq!(count_job_directories(app.data_dir()), jobs);
    assert_eq!(count_named_files(app.data_dir(), "input"), sources);
    assert_eq!(count_named_files(app.data_dir(), "result.md"), markdown);
    let status = app
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    assert_eq!(json_body(status).await["data"]["status"], "succeeded");
}

#[tokio::test]
async fn startup_reconciliation_ignores_the_readiness_probe_directory() {
    let harness = TestHarness::new();
    let first = harness.app().await;
    let seeded = first.submit_succeeded_job("readiness-reconciliation").await;
    assert_eq!(
        first
            .request(Method::GET, "/health/ready", None)
            .await
            .status(),
        StatusCode::OK
    );
    let health = harness.data_dir().join(".health");
    assert!(health.is_dir(), "readiness owns a probe directory on disk");
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    // A crash between creating and removing a probe leaves a UUID-named file
    // behind, which is what a storage scan reaching past `jobs/` would read as
    // a conversion and quarantine.
    fs::write(health.join(Uuid::new_v4().to_string()), b"ready").unwrap();

    let restarted = harness.app().await;
    assert!(health.is_dir(), "restart moved the probe directory");
    assert_eq!(
        fs::read_dir(harness.data_dir().join("quarantine/pre-acceptance"))
            .unwrap()
            .count(),
        0,
        "startup reconciliation quarantined the probe directory"
    );
    assert_eq!(count_job_directories(harness.data_dir()), 1);
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    assert_eq!(json_body(status).await["data"]["status"], "succeeded");
    assert_eq!(
        restarted
            .request(Method::GET, "/health/ready", None)
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn conversion_routes_require_one_valid_bearer_token() {
    let app = test_app().await;
    let response = app
        .request(Method::GET, "/api/v1/conversions/not-a-uuid", None)
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()["www-authenticate"].to_str().unwrap(),
        "Bearer"
    );
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let payload = json_body(response).await;
    assert_eq!(payload["error"]["code"], "unauthorized");
    assert_eq!(payload["error"]["requestId"], request_id);

    let response = app
        .request_with_headers(
            Method::GET,
            "/api/v1/conversions/not-a-uuid",
            Body::empty(),
            &[(
                AUTHORIZATION.as_str(),
                "Bearer wrong-wrong-wrong-wrong-wrong-wrong",
            )],
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let (_writer, reader) = tokio::io::duplex(64);
    let response = tokio::time::timeout(
        Duration::from_millis(250),
        app.request_with_headers(
            Method::POST,
            "/api/v1/conversions",
            streaming_body(reader),
            &[
                ("idempotency-key", "unauthenticated-stream"),
                (
                    "content-type",
                    "multipart/form-data; boundary=tool-kit-boundary",
                ),
            ],
        ),
    )
    .await
    .expect("authentication must reject before reading a streaming body");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(count_job_directories(app.data_dir()), 0);
}

#[tokio::test]
async fn clean_pdf_completes_and_idempotency_replays_the_job() {
    let app = test_app().await;
    let pdf = clean_pdf();
    let client_run_id = Uuid::new_v4();
    let body = multipart_body(client_run_id, "standard", &pdf, "fixture.pdf");

    let response = app.submit(body.clone(), "clean-pdf-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(response.headers()["idempotency-replayed"], "false");
    let location = response.headers()["location"].to_str().unwrap().to_owned();
    let submitted = json_body(response).await;
    let job_id = submitted["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(submitted["data"]["profile"], "standard");
    assert_eq!(submitted["data"]["status"], "queued");
    assert_eq!(location, format!("/api/v1/conversions/{job_id}"));

    let completed = app.wait_for_terminal(&job_id).await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    assert_eq!(completed["data"]["route"]["kind"], "local_pdf");
    assert_eq!(
        completed["data"]["route"]["reasonCodes"],
        serde_json::json!(["native_text_pdf"])
    );

    let artifacts = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
        .await;
    assert_eq!(artifacts.status(), StatusCode::OK);
    let artifacts = json_body(artifacts).await;
    assert_eq!(artifacts["data"].as_array().unwrap().len(), 2);

    let markdown = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(markdown.status(), StatusCode::OK);
    assert_eq!(markdown.headers()["cache-control"], "private, no-store");
    let markdown = markdown.into_body().collect().await.unwrap().to_bytes();
    assert!(!markdown.is_empty());

    let manifest = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/manifest"))
        .await;
    assert_eq!(manifest.status(), StatusCode::OK);
    let manifest_bytes = manifest.into_body().collect().await.unwrap().to_bytes();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["schemaVersion"], 1);
    assert_eq!(manifest["engine"]["version"], "1.25.2");
    assert_eq!(manifest["document"]["pdfType"], "text_based");
    assert_eq!(
        manifest["output"]["sha256"],
        hex::encode(Sha256::digest(&markdown))
    );
    assert!(!String::from_utf8_lossy(&manifest_bytes).contains("fixture.pdf"));

    let replay = app.submit(body, "clean-pdf-1", TOKEN).await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(replay.headers()["idempotency-replayed"], "true");
    let replay = json_body(replay).await;
    assert_eq!(replay["data"]["id"], job_id);

    let conflict_body = multipart_body(Uuid::new_v4(), "standard", &pdf, "fixture.pdf");
    let conflict = app.submit(conflict_body, "clean-pdf-1", TOKEN).await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(conflict).await["error"]["code"],
        "idempotency_conflict"
    );
}

#[tokio::test]
async fn docx_completes_through_anydoc_and_replays() {
    let app = test_app().await;
    let docx = include_bytes!("fixtures/anydoc/text.docx").as_slice();
    let body = multipart(
        &[
            ("clientRunId", &Uuid::new_v4().to_string()),
            ("profile", "standard"),
        ],
        Some((
            docx,
            "notes.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )),
    );

    let response = app.submit(body.clone(), "anydoc-docx-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let submitted = json_body(response).await;
    let job_id = submitted["data"]["id"].as_str().unwrap().to_owned();

    let completed = app.wait_for_terminal(&job_id).await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    assert_eq!(completed["data"]["route"]["kind"], "local_anydoc");
    assert_eq!(
        completed["data"]["route"]["reasonCodes"],
        serde_json::json!(["structured_document"])
    );

    let markdown = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(markdown.status(), StatusCode::OK);
    let markdown = markdown.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&markdown).contains("Fixture Document"));

    let manifest = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/manifest"))
        .await;
    assert_eq!(manifest.status(), StatusCode::OK);
    let manifest_bytes = manifest.into_body().collect().await.unwrap().to_bytes();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["engine"]["name"], "anydoc");
    assert_eq!(manifest["engine"]["version"], "0.2.4");
    assert_eq!(manifest["document"]["format"], "docx");
    assert_eq!(
        manifest["source"]["mediaType"],
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    );
    assert_eq!(
        manifest["output"]["sha256"],
        hex::encode(Sha256::digest(&markdown))
    );
    assert!(!String::from_utf8_lossy(&manifest_bytes).contains("notes.docx"));

    let replay = app.submit(body, "anydoc-docx-1", TOKEN).await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(replay.headers()["idempotency-replayed"], "true");
    let replay = json_body(replay).await;
    assert_eq!(replay["data"]["id"], job_id);
}

#[tokio::test]
async fn every_advertised_anydoc_family_converts() {
    let app = test_app().await;
    for (fixture, filename, media_type, label) in [
        (
            include_bytes!("fixtures/anydoc/sheet.xlsx").as_slice(),
            "sheet.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "excel",
        ),
        (
            include_bytes!("fixtures/anydoc/sheet.xls").as_slice(),
            "sheet.xls",
            "application/vnd.ms-excel",
            "excel",
        ),
        (
            include_bytes!("fixtures/anydoc/pres.pptx").as_slice(),
            "deck.pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "pptx",
        ),
        (
            include_bytes!("fixtures/anydoc/handmade-multimaster.ppt").as_slice(),
            "deck.ppt",
            "application/vnd.ms-powerpoint",
            "ppt",
        ),
        (
            include_bytes!("fixtures/anydoc/text.doc").as_slice(),
            "notes.doc",
            "application/msword",
            "doc",
        ),
        (
            include_bytes!("fixtures/anydoc/book.epub").as_slice(),
            "book.epub",
            "application/epub+zip",
            "epub",
        ),
        (
            include_bytes!("fixtures/anydoc/text.odt").as_slice(),
            "notes.odt",
            "application/vnd.oasis.opendocument.text",
            "odt",
        ),
        (
            include_bytes!("fixtures/anydoc/sheet.ods").as_slice(),
            "sheet.ods",
            "application/vnd.oasis.opendocument.spreadsheet",
            "ods",
        ),
        (
            include_bytes!("fixtures/anydoc/pres.odp").as_slice(),
            "deck.odp",
            "application/vnd.oasis.opendocument.presentation",
            "odp",
        ),
        (
            include_bytes!("fixtures/anydoc/text.rtf").as_slice(),
            "notes.rtf",
            "application/rtf",
            "rtf",
        ),
        (
            include_bytes!("fixtures/anydoc/sheet.csv").as_slice(),
            "rows.csv",
            "text/csv",
            "csv",
        ),
        // Extension variants: same parser, so the reported family is the base
        // format's label, not the extension.
        (
            include_bytes!("fixtures/anydoc/text.docm").as_slice(),
            "macro.docm",
            "application/vnd.ms-word.document.macroEnabled.12",
            "docx",
        ),
        (
            include_bytes!("fixtures/anydoc/sheet.xlsm").as_slice(),
            "macro.xlsm",
            "application/vnd.ms-excel.sheet.macroEnabled.12",
            "excel",
        ),
        (
            include_bytes!("fixtures/anydoc/pres.pptm").as_slice(),
            "macro.pptm",
            "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
            "pptx",
        ),
        (
            include_bytes!("fixtures/anydoc/pres.ppsx").as_slice(),
            "show.ppsx",
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
            "pptx",
        ),
        (
            include_bytes!("fixtures/anydoc/pres.ppsm").as_slice(),
            "show.ppsm",
            "application/vnd.ms-powerpoint.slideshow.macroEnabled.12",
            "pptx",
        ),
        (
            include_bytes!("fixtures/anydoc/deck.pps").as_slice(),
            "show.pps",
            "application/vnd.ms-powerpoint",
            "ppt",
        ),
        (
            include_bytes!("fixtures/anydoc/deck.pot").as_slice(),
            "template.pot",
            "application/vnd.ms-powerpoint",
            "ppt",
        ),
    ] {
        let body = multipart(
            &[
                ("clientRunId", &Uuid::new_v4().to_string()),
                ("profile", "standard"),
            ],
            Some((fixture, filename, media_type)),
        );
        let response = app
            .submit(body, &format!("anydoc-family-{filename}"), TOKEN)
            .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED, "{label}");
        let submitted = json_body(response).await;
        let job_id = submitted["data"]["id"].as_str().unwrap().to_owned();
        let completed = app.wait_for_terminal(&job_id).await;
        assert_eq!(
            completed["data"]["status"], "succeeded",
            "{label}: {completed:#}"
        );
        assert_eq!(
            completed["data"]["route"]["kind"], "local_anydoc",
            "{label}"
        );

        let manifest = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/manifest"))
            .await;
        assert_eq!(manifest.status(), StatusCode::OK, "{label}");
        let manifest: Value =
            serde_json::from_slice(&manifest.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(manifest["engine"]["name"], "anydoc", "{label}");
        assert_eq!(manifest["document"]["format"], label, "{label}");
        assert!(manifest["document"]["processingTimeMs"].is_u64(), "{label}");
    }
}

#[tokio::test]
async fn broken_and_hostile_anydoc_inputs_fail_closed_without_artifacts() {
    let app = test_app().await;
    for (fixture, filename, media_type, code) in [
        (
            include_bytes!("fixtures/anydoc/truncated.docx").as_slice(),
            "broken.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "invalid_document",
        ),
        (
            include_bytes!("fixtures/anydoc/truncated.doc").as_slice(),
            "broken.doc",
            "application/msword",
            "invalid_document",
        ),
        (
            include_bytes!("fixtures/anydoc/truncated.xls").as_slice(),
            "broken.xls",
            "application/vnd.ms-excel",
            "invalid_document",
        ),
        (
            include_bytes!("fixtures/anydoc/truncated.xlsx").as_slice(),
            "broken.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "invalid_document",
        ),
        (
            include_bytes!("fixtures/anydoc/truncated.epub").as_slice(),
            "broken.epub",
            "application/epub+zip",
            "invalid_document",
        ),
        (
            include_bytes!("fixtures/anydoc/deepnest.ppt").as_slice(),
            "deep.ppt",
            "application/vnd.ms-powerpoint",
            "document_exceeds_limits",
        ),
        (
            include_bytes!("fixtures/anydoc/hugespan.pptx").as_slice(),
            "huge.pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "document_exceeds_limits",
        ),
        (
            include_bytes!("fixtures/anydoc/encrypted.odt").as_slice(),
            "locked.odt",
            "application/vnd.oasis.opendocument.text",
            "encrypted_document",
        ),
        (
            include_bytes!("fixtures/anydoc/hugerepeat.ods").as_slice(),
            "huge.ods",
            "application/vnd.oasis.opendocument.spreadsheet",
            "document_exceeds_limits",
        ),
        (
            include_bytes!("fixtures/anydoc/truncated.odp").as_slice(),
            "broken.odp",
            "application/vnd.oasis.opendocument.presentation",
            "invalid_document",
        ),
        (
            include_bytes!("fixtures/anydoc/empty.rtf").as_slice(),
            "empty.rtf",
            "application/rtf",
            "invalid_document",
        ),
        (
            include_bytes!("fixtures/anydoc/empty.csv").as_slice(),
            "empty.csv",
            "text/csv",
            "invalid_document",
        ),
    ] {
        let body = multipart(
            &[
                ("clientRunId", &Uuid::new_v4().to_string()),
                ("profile", "standard"),
            ],
            Some((fixture, filename, media_type)),
        );
        let response = app
            .submit(body, &format!("anydoc-broken-{filename}"), TOKEN)
            .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED, "{code}");
        let submitted = json_body(response).await;
        let job_id = submitted["data"]["id"].as_str().unwrap().to_owned();
        let completed = app.wait_for_terminal(&job_id).await;
        assert_eq!(
            completed["data"]["status"], "failed",
            "{code}: {completed:#}"
        );
        assert_eq!(completed["data"]["failure"]["code"], code, "{code}");

        let artifacts = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
            .await;
        assert!(
            json_body(artifacts).await["data"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{code}"
        );
        let markdown = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
            .await;
        assert_eq!(markdown.status(), StatusCode::NOT_FOUND, "{code}");
    }
    assert_eq!(count_named_files(app.data_dir(), "result.md"), 0);
}

#[tokio::test]
async fn a_cross_family_mislabelled_container_fails_before_conversion() {
    // docx/xlsx/pptx/epub all carry the same ZIP magic, so upload admission
    // cannot separate them. An xlsx sent as .docx used to convert fine and
    // then trip the manifest check as `artifact_integrity_failed` - a
    // corruption code for a merely misnamed file. It must fail as an invalid
    // document instead, and before any parsing happens.
    let app = test_app().await;
    let body = multipart(
        &[
            ("clientRunId", &Uuid::new_v4().to_string()),
            ("profile", "standard"),
        ],
        Some((
            include_bytes!("fixtures/anydoc/sheet.xlsx").as_slice(),
            "actually-a-sheet.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )),
    );
    let response = app.submit(body, "anydoc-mislabelled-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id = json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let completed = app.wait_for_terminal(&job_id).await;
    assert_eq!(completed["data"]["status"], "failed", "{completed:#}");
    assert_eq!(
        completed["data"]["failure"]["code"], "invalid_document",
        "{completed:#}"
    );
    let artifacts = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
        .await;
    assert!(json_body(artifacts).await["data"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn anydoc_output_ceiling_fails_closed() {
    let app = test_app_with_output_limit(64).await;
    let docx = include_bytes!("fixtures/anydoc/text.docx").as_slice();
    let body = multipart(
        &[
            ("clientRunId", &Uuid::new_v4().to_string()),
            ("profile", "standard"),
        ],
        Some((
            docx,
            "notes.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )),
    );
    let response = app.submit(body, "anydoc-ceiling-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let submitted = json_body(response).await;
    let job_id = submitted["data"]["id"].as_str().unwrap().to_owned();

    let completed = app.wait_for_terminal(&job_id).await;
    assert_eq!(completed["data"]["status"], "failed", "{completed:#}");
    assert_eq!(
        completed["data"]["failure"]["code"],
        "document_exceeds_limits"
    );
    app.wait_for_job_runner_idle().await;
    assert_eq!(count_named_files(app.data_dir(), "input"), 0);
}

#[tokio::test]
async fn anydoc_upload_boundaries_enforce_extension_media_type_and_magic() {
    let app = test_app().await;
    let docx = include_bytes!("fixtures/anydoc/text.docx").as_slice();
    let docx_mime = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    for (body, key, status, code) in [
        (
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((&clean_pdf(), "notes.docx", docx_mime)),
            ),
            "pdf-content-docx-extension",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_source_signature",
        ),
        (
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((docx, "notes.docx", "application/msword")),
            ),
            "mismatched-media-type",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_source_media_type",
        ),
        (
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((docx, "notes.md", "text/markdown")),
            ),
            "unsupported-extension",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_source_extension",
        ),
        (
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((docx, "notes.docx", docx_mime)),
            ),
            "docx-accepted",
            StatusCode::ACCEPTED,
            "",
        ),
    ] {
        let response = app.submit(body, key, TOKEN).await;
        assert_eq!(response.status(), status, "{key}");
        if status != StatusCode::ACCEPTED {
            assert_eq!(json_body(response).await["error"]["code"], code, "{key}");
        }
    }
}

#[tokio::test]
async fn a_succeeded_anydoc_job_survives_restart_without_wedging_startup() {
    // Startup reconciliation revalidates every stored success. It once knew
    // only the four PDF classifications, so a succeeded AnyDoc attempt
    // (`structured_document`) failed the metadata invariant and aborted
    // startup - the whole service refused to boot, not just that one job.
    let harness = TestHarness::new();
    let docx = include_bytes!("fixtures/anydoc/text.docx").as_slice();
    let body = multipart(
        &[
            ("clientRunId", &Uuid::new_v4().to_string()),
            ("profile", "standard"),
        ],
        Some((
            docx,
            "notes.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )),
    );
    let first = harness.app().await;

    let response = first.submit(body, "anydoc-restart-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id = json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let completed = first.wait_for_terminal(&job_id).await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    let markdown_before = first
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    // Booting at all is half the assertion: recovery runs before this returns.
    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{job_id}"))
        .await;
    assert_eq!(status.status(), StatusCode::OK);
    let after = json_body(status).await;
    assert_eq!(after["data"]["status"], "succeeded", "{after:#}");
    let markdown_after = restarted
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(markdown_after.status(), StatusCode::OK);
    assert_eq!(
        markdown_after
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        markdown_before,
        "anydoc markdown changed across a restart"
    );
}

#[tokio::test]
async fn completed_job_idempotency_and_downloads_survive_app_restart() {
    let harness = TestHarness::new();
    let pdf = clean_pdf();
    let client_run_id = Uuid::new_v4();
    let body = multipart_body(client_run_id, "standard", &pdf, "fixture.pdf");
    let first = harness.app().await;

    let response = first.submit(body.clone(), "durable-restart-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let submitted = json_body(response).await;
    let job_id = submitted["data"]["id"].as_str().unwrap().to_owned();
    let completed = first.wait_for_terminal(&job_id).await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");

    let markdown_before = first
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let manifest_before = first
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/manifest"))
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{job_id}"))
        .await;
    assert_eq!(status.status(), StatusCode::OK);
    assert_eq!(json_body(status).await["data"]["status"], "succeeded");

    let markdown_after = restarted
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(markdown_after.status(), StatusCode::OK);
    assert_eq!(
        markdown_after
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        markdown_before
    );
    let manifest_after = restarted
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/manifest"))
        .await;
    assert_eq!(manifest_after.status(), StatusCode::OK);
    assert_eq!(
        manifest_after
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        manifest_before
    );

    let replay = restarted.submit(body, "durable-restart-1", TOKEN).await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(replay.headers()["idempotency-replayed"], "true");
    assert_eq!(json_body(replay).await["data"]["id"], job_id);

    let conflict = restarted
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &pdf, "fixture.pdf"),
            "durable-restart-1",
            TOKEN,
        )
        .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(conflict).await["error"]["code"],
        "idempotency_conflict"
    );
    assert_eq!(count_job_directories(restarted.data_dir()), 1);
    // Neither request stored a source, and the succeeded job's own is gone.
    assert_eq!(count_named_files(restarted.data_dir(), "input"), 0);
    assert_eq!(count_named_files(restarted.data_dir(), "input.staging"), 0);
}

#[tokio::test]
async fn corrupted_published_artifact_fails_closed_and_preserves_audit_files() {
    let harness = TestHarness::new();
    let app = harness.app().await;
    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "fixture.pdf"),
            "artifact-integrity-1",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let submitted = json_body(response).await;
    let job_id = submitted["data"]["id"].as_str().unwrap().to_owned();
    let attempt_id = submitted["data"]["activeAttemptId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        app.wait_for_terminal(&job_id).await["data"]["status"],
        "succeeded"
    );

    let markdown_path = app
        .data_dir()
        .join("jobs")
        .join(&job_id)
        .join("attempts")
        .join(&attempt_id)
        .join("artifacts")
        .join("result.md");
    std::fs::write(&markdown_path, b"corrupted").unwrap();

    let status = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}"))
        .await;
    assert_eq!(status.status(), StatusCode::OK);
    let status = json_body(status).await;
    assert_eq!(status["data"]["status"], "failed", "{status:#}");
    assert_eq!(
        status["data"]["failure"]["code"],
        "artifact_integrity_failed"
    );
    let artifacts = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
        .await;
    assert!(json_body(artifacts).await["data"]
        .as_array()
        .unwrap()
        .is_empty());
    let download = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(download.status(), StatusCode::NOT_FOUND);
    assert_eq!(std::fs::read(&markdown_path).unwrap(), b"corrupted");

    app.shutdown(Duration::from_secs(1)).await;
    drop(app);
    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{job_id}"))
        .await;
    assert_eq!(json_body(status).await["data"]["status"], "failed");
    assert!(markdown_path.exists());
}

#[tokio::test]
async fn converting_job_requeues_with_a_fresh_attempt_and_completes_after_restart() {
    let harness = TestHarness::new();
    harness.initialize_empty().await;
    let seeded = harness.insert_converting_job(&clean_pdf(), 0).await;
    let interrupted_attempt = harness
        .data_dir()
        .join("jobs")
        .join(seeded.job_id.to_string())
        .join("attempts")
        .join(seeded.attempt_id.to_string());

    let restarted = harness.app().await;
    let completed = restarted
        .wait_for_terminal(&seeded.job_id.to_string())
        .await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    assert_ne!(
        completed["data"]["activeAttemptId"],
        seeded.attempt_id.to_string()
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 2);
    assert!(interrupted_attempt.exists());
    assert!(!restarted.job_runner_failed());
}

#[tokio::test]
async fn recovery_limit_produces_a_stable_failure_without_another_attempt() {
    let harness = TestHarness::with_recovery_limit(1);
    harness.initialize_empty().await;
    let seeded = harness.insert_converting_job(&clean_pdf(), 1).await;

    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    assert_eq!(status.status(), StatusCode::OK);
    let status = json_body(status).await;
    assert_eq!(status["data"]["status"], "failed", "{status:#}");
    assert_eq!(status["data"]["failure"]["code"], "recovery_limit_exceeded");
    assert_eq!(
        status["data"]["activeAttemptId"],
        seeded.attempt_id.to_string()
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert_eq!(count_named_files(harness.data_dir(), "input"), 0);
}

#[tokio::test]
async fn corrupt_active_source_fails_recovery_without_creating_a_retry() {
    let harness = TestHarness::new();
    harness.initialize_empty().await;
    let seeded = harness.insert_converting_job(&clean_pdf(), 0).await;
    let source = harness
        .data_dir()
        .join("jobs")
        .join(seeded.job_id.to_string())
        .join("source/input");
    fs::write(&source, b"%PDF-1.4\nmutated after durable acceptance\n").unwrap();

    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    let status = json_body(status).await;
    assert_eq!(status["data"]["status"], "failed", "{status:#}");
    assert_eq!(status["data"]["failure"]["code"], "source_integrity_failed");
    assert_eq!(
        status["data"]["activeAttemptId"],
        seeded.attempt_id.to_string()
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert!(!source.exists());
}

/// A build before this one kept every succeeded job's source, which for audio
/// is a hidden copy of each recording. The next boot removes them.
#[tokio::test]
async fn restart_removes_the_source_a_succeeded_job_kept() {
    let harness = TestHarness::new();
    let first = harness.app().await;
    let seeded = first.submit_succeeded_job("sweep-succeeded-source").await;
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);
    let source = harness
        .data_dir()
        .join("jobs")
        .join(seeded.job_id.to_string())
        .join("source/input");
    assert!(!source.exists());
    fs::write(&source, clean_pdf()).unwrap();

    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    assert_eq!(json_body(status).await["data"]["status"], "succeeded");
    assert!(!source.exists());
}

#[tokio::test]
async fn finalizing_job_with_a_valid_published_bundle_completes_during_restart() {
    let harness = TestHarness::new();
    let first = harness.app().await;
    let seeded = first.submit_succeeded_job("recover-valid-finalizing").await;
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);
    harness.demote_succeeded_to_finalizing(seeded).await;

    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    let status = json_body(status).await;
    assert_eq!(status["data"]["status"], "succeeded", "{status:#}");
    assert_eq!(
        status["data"]["activeAttemptId"],
        seeded.attempt_id.to_string()
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert_eq!(harness.artifact_row_count(seeded.attempt_id).await, 2);
    for name in ["markdown", "manifest"] {
        let download = restarted
            .authorized_get(&format!(
                "/api/v1/conversions/{}/artifacts/{name}",
                seeded.job_id
            ))
            .await;
        assert_eq!(download.status(), StatusCode::OK, "{name}");
    }
}

#[tokio::test]
async fn corrupted_succeeded_bundle_is_persisted_failed_on_restart_and_retained_for_audit() {
    let harness = TestHarness::new();
    let first = harness.app().await;
    let seeded = first
        .submit_succeeded_job("recover-corrupt-succeeded")
        .await;
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    let artifacts = harness
        .data_dir()
        .join("jobs")
        .join(seeded.job_id.to_string())
        .join("attempts")
        .join(seeded.attempt_id.to_string())
        .join("artifacts");
    let markdown = artifacts.join("result.md");
    let manifest = artifacts.join("manifest.json");
    fs::write(&markdown, b"corrupted while the service was stopped").unwrap();

    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    let status = json_body(status).await;
    assert_eq!(status["data"]["status"], "failed", "{status:#}");
    assert_eq!(
        status["data"]["failure"]["code"],
        "artifact_integrity_failed"
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert_eq!(harness.artifact_row_count(seeded.attempt_id).await, 2);
    assert!(markdown.exists());
    assert!(manifest.exists());
    for name in ["markdown", "manifest"] {
        let download = restarted
            .authorized_get(&format!(
                "/api/v1/conversions/{}/artifacts/{name}",
                seeded.job_id
            ))
            .await;
        assert_eq!(download.status(), StatusCode::NOT_FOUND, "{name}");
    }

    restarted.shutdown(Duration::from_secs(1)).await;
    drop(restarted);
    let second_restart = harness.app().await;
    let status = second_restart
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    let status = json_body(status).await;
    assert_eq!(status["data"]["status"], "failed", "{status:#}");
    assert_eq!(
        status["data"]["failure"]["code"],
        "artifact_integrity_failed"
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert!(markdown.exists());
    assert!(manifest.exists());
}

#[tokio::test]
async fn database_less_canonical_job_is_quarantined_without_touching_an_owned_sibling() {
    let harness = TestHarness::new();
    let first = harness.app().await;
    let owned = first.submit_succeeded_job("recover-owned-sibling").await;
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    let orphan_id = Uuid::new_v4();
    let orphan = harness.data_dir().join("jobs").join(orphan_id.to_string());
    fs::create_dir_all(orphan.join("source")).unwrap();
    fs::write(orphan.join("source/orphan-marker"), b"orphan audit").unwrap();
    let owned_directory = harness
        .data_dir()
        .join("jobs")
        .join(owned.job_id.to_string());

    let restarted = harness.app().await;
    assert!(!orphan.exists());
    assert!(owned_directory.exists());
    let preacceptance = harness.data_dir().join("quarantine/pre-acceptance");
    let reservation_prefix = format!("{orphan_id}-");
    let quarantine = fs::read_dir(preacceptance)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&reservation_prefix))
        })
        .expect("orphan quarantine reservation was not created")
        .path()
        .join("job");
    assert_eq!(
        fs::read(quarantine.join("source/orphan-marker")).unwrap(),
        b"orphan audit"
    );
    let owned_status = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", owned.job_id))
        .await;
    assert_eq!(json_body(owned_status).await["data"]["status"], "succeeded");
}

#[tokio::test]
async fn invalid_finalizing_bundles_requeue_to_a_fresh_attempt() {
    for case in ["unknown-manifest-field", "extra-publication-file"] {
        let harness = TestHarness::new();
        let first = harness.app().await;
        let seeded = first.submit_succeeded_job(case).await;
        first.shutdown(Duration::from_secs(1)).await;
        drop(first);
        harness.demote_succeeded_to_finalizing(seeded).await;

        let published = harness
            .data_dir()
            .join("jobs")
            .join(seeded.job_id.to_string())
            .join("attempts")
            .join(seeded.attempt_id.to_string())
            .join("artifacts");
        match case {
            "unknown-manifest-field" => {
                let path = published.join("manifest.json");
                let mut manifest: Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                manifest
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".to_owned(), Value::Bool(true));
                fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
            "extra-publication-file" => {
                fs::write(published.join("unexpected.bin"), b"unexpected").unwrap();
            }
            _ => unreachable!(),
        }

        let restarted = harness.app().await;
        let completed = restarted
            .wait_for_terminal(&seeded.job_id.to_string())
            .await;
        assert_eq!(
            completed["data"]["status"], "succeeded",
            "{case}: {completed:#}"
        );
        assert_ne!(
            completed["data"]["activeAttemptId"],
            seeded.attempt_id.to_string(),
            "{case}"
        );
        assert_eq!(harness.attempt_count(seeded.job_id).await, 2, "{case}");
        assert!(published.exists(), "{case}: old audit bundle was removed");
    }
}

const AUDIO_MEDIA_TYPES: [&str; 6] = [
    "audio/wav",
    "audio/mp4",
    "video/mp4",
    "video/quicktime",
    "audio/mpeg",
    "audio/flac",
];
const TWO_SPEAKERS_WAV: &[u8] = include_bytes!("fixtures/audio/two-speakers.wav");
const SILENCE_WAV: &[u8] = include_bytes!("fixtures/audio/silence.wav");

/// Runs everywhere, because the default harness stages no audio worker. An
/// advertised format with no engine behind it is a job that is accepted and
/// never claimed, so the whole audio surface has to be absent instead.
#[tokio::test]
async fn audio_is_unadvertised_and_refused_without_the_worker() {
    let app = test_app().await;
    let payload = json_body(app.request(Method::GET, "/api/v1/capabilities", None).await).await;
    let conversion = &payload["data"]["conversion"];
    let formats = conversion["inputFormats"].as_array().unwrap();
    for media_type in AUDIO_MEDIA_TYPES {
        assert!(
            !formats.iter().any(|format| format == media_type),
            "{media_type} is advertised with no engine"
        );
    }

    let response = app
        .submit(
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((TWO_SPEAKERS_WAV, "two-speakers.wav", "audio/wav")),
            ),
            "audio-absent-1",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "source_format_unavailable"
    );
}

#[tokio::test]
async fn a_two_speaker_recording_transcribes_end_to_end() {
    let Some(harness) = TestHarness::with_audio_worker() else {
        eprintln!(
            "SKIPPED a_two_speaker_recording_transcribes_end_to_end: workers/audio/bin and \
             workers/audio/models are unbuilt on this machine"
        );
        return;
    };
    let app = harness.app().await;

    let payload = json_body(app.request(Method::GET, "/api/v1/capabilities", None).await).await;
    let conversion = &payload["data"]["conversion"];
    let formats = conversion["inputFormats"].as_array().unwrap();
    for media_type in AUDIO_MEDIA_TYPES {
        assert!(
            formats.iter().any(|format| format == media_type),
            "{media_type} is missing from a host that can transcribe"
        );
    }

    let response = app
        .submit(
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                    ("speakerCount", "2"),
                ],
                Some((TWO_SPEAKERS_WAV, "two-speakers.wav", "audio/wav")),
            ),
            "audio-transcript-1",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id = json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let completed = app.wait_for_terminal(&job_id).await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    assert_eq!(completed["data"]["route"]["kind"], "local_audio");
    assert_eq!(
        completed["data"]["route"]["reasonCodes"],
        serde_json::json!(["transcribed_audio"])
    );

    let markdown = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(markdown.status(), StatusCode::OK);
    let markdown = markdown.into_body().collect().await.unwrap().to_bytes();
    let transcript = String::from_utf8_lossy(&markdown);
    assert!(
        transcript.contains("Speaker 1") && transcript.contains("Speaker 2"),
        "a pinned count of two should name two speakers:\n{transcript}"
    );
    assert!(
        !transcript.contains("Speakers guessed"),
        "a pinned count is not a guess:\n{transcript}"
    );

    // Serving the manifest at all is the classification assertion: the read
    // path refuses one whose stored classification disagrees with the engine
    // that produced it, and "audio" is the only value local-audio may carry.
    let manifest = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/manifest"))
        .await;
    assert_eq!(manifest.status(), StatusCode::OK);
    let manifest: Value =
        serde_json::from_slice(&manifest.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(manifest["engine"]["name"], "local-audio");
    assert_eq!(manifest["route"]["kind"], "local_audio");
    assert_eq!(
        manifest["route"]["reasonCodes"],
        serde_json::json!(["transcribed_audio"])
    );
    assert_eq!(manifest["document"]["speakersFound"], 2);
    assert_eq!(manifest["document"]["speakerCountGuessed"], false);
    assert_eq!(
        manifest["output"]["sha256"],
        hex::encode(Sha256::digest(&markdown))
    );
}

/// A rejection ends the job. v1 has no remote leg for a local transcription,
/// so `needs_remote` here would strand it.
#[tokio::test]
async fn a_recording_with_no_speech_fails_rather_than_asking_for_a_remote() {
    let Some(harness) = TestHarness::with_audio_worker() else {
        eprintln!(
            "SKIPPED a_recording_with_no_speech_fails_rather_than_asking_for_a_remote: \
             workers/audio/bin and workers/audio/models are unbuilt on this machine"
        );
        return;
    };
    let app = harness.app().await;

    let response = app
        .submit(
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((SILENCE_WAV, "silence.wav", "audio/wav")),
            ),
            "audio-silence-1",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id = json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let completed = app.wait_for_terminal(&job_id).await;
    assert_eq!(completed["data"]["status"], "failed", "{completed:#}");
    assert_eq!(completed["data"]["failure"]["code"], "no_speech_found");
}

#[tokio::test]
async fn speaker_count_is_validated_and_part_of_the_request_identity() {
    let app = test_app().await;
    let pdf = clean_pdf();
    for (counts, key, code) in [
        (["0"].as_slice(), "speakers-zero", "invalid_speaker_count"),
        (["21"].as_slice(), "speakers-over", "invalid_speaker_count"),
        (["two"].as_slice(), "speakers-word", "invalid_speaker_count"),
        (
            ["2", "3"].as_slice(),
            "speakers-twice",
            "duplicate_multipart_field",
        ),
    ] {
        let client_run_id = Uuid::new_v4().to_string();
        let mut fields = vec![
            ("clientRunId", client_run_id.as_str()),
            ("profile", "standard"),
        ];
        fields.extend(counts.iter().map(|count| ("speakerCount", *count)));
        let body = multipart(&fields, Some((&pdf, "fixture.pdf", "application/pdf")));
        let response = app.submit(body, key, TOKEN).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{key}");
        assert_eq!(json_body(response).await["error"]["code"], code, "{key}");
    }
    assert_eq!(count_job_directories(app.data_dir()), 0);

    // Same key, same bytes, a different count is a different request, so it
    // must conflict rather than replay the first job's transcript settings.
    let client_run_id = Uuid::new_v4();
    let two = multipart(
        &[
            ("clientRunId", &client_run_id.to_string()),
            ("profile", "standard"),
            ("speakerCount", "2"),
        ],
        Some((&pdf, "fixture.pdf", "application/pdf")),
    );
    let response = app.submit(two.clone(), "speakers-identity", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let replay = app.submit(two, "speakers-identity", TOKEN).await;
    assert_eq!(replay.headers()["idempotency-replayed"], "true");

    let three = multipart(
        &[
            ("clientRunId", &client_run_id.to_string()),
            ("profile", "standard"),
            ("speakerCount", "3"),
        ],
        Some((&pdf, "fixture.pdf", "application/pdf")),
    );
    let conflict = app.submit(three, "speakers-identity", TOKEN).await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(conflict).await["error"]["code"],
        "idempotency_conflict"
    );
}

/// An hour of audio clears the document ceiling by an order of magnitude, so
/// the two limits are separate and the format's engine picks which one binds.
#[tokio::test]
async fn the_audio_ceiling_bounds_recordings_without_touching_documents() {
    let Some(harness) = TestHarness::with_audio_worker() else {
        eprintln!(
            "SKIPPED the_audio_ceiling_bounds_recordings_without_touching_documents: \
             workers/audio/bin and workers/audio/models are unbuilt on this machine"
        );
        return;
    };
    let app = harness.with_upload_ceilings(1024 * 1024, 1024).app().await;

    let response = app
        .submit(
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((TWO_SPEAKERS_WAV, "two-speakers.wav", "audio/wav")),
            ),
            "audio-ceiling-1",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "upload_too_large"
    );

    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "fixture.pdf"),
            "audio-ceiling-2",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn invalid_submission_fields_are_rejected_without_creating_jobs() {
    let app = test_app().await;
    let pdf = clean_pdf();

    let wrong_profile = multipart_body(Uuid::new_v4(), "best_quality", &pdf, "fixture.pdf");
    let response = app.submit(wrong_profile, "profile-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "profile_unavailable"
    );

    // A real PNG, well formed and advertised by the contract. Only the missing
    // engine refuses it, and it must refuse before anything is staged.
    let no_engine = multipart(
        &[
            ("clientRunId", &Uuid::new_v4().to_string()),
            ("profile", "standard"),
        ],
        Some((
            b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR",
            "fixture.png",
            "image/png",
        )),
    );
    let response = app.submit(no_engine, "vision-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "source_format_unavailable"
    );

    let wrong_signature = multipart_body(
        Uuid::new_v4(),
        "standard",
        b"PK\x03\x04not-a-pdf",
        "fixture.pdf",
    );
    let response = app.submit(wrong_signature, "signature-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "invalid_source_signature"
    );
    assert_eq!(count_job_directories(app.data_dir()), 0);
    assert_eq!(count_named_files(app.data_dir(), "input"), 0);
    assert_eq!(count_named_files(app.data_dir(), "input.staging"), 0);
}

#[tokio::test]
async fn multipart_boundaries_enforce_fields_media_type_extension_and_size() {
    let app = test_app().await;
    let pdf = clean_pdf();
    // The second profile follows the source, so it is refused after staging.
    let mut duplicate_profile = multipart_body(Uuid::new_v4(), "standard", &pdf, "fixture.pdf");
    duplicate_profile.truncate(duplicate_profile.len() - b"--tool-kit-boundary--\r\n".len());
    duplicate_profile.extend(multipart(&[("profile", "local_only")], None));
    for (body, key, status, code) in [
        (
            multipart_body(Uuid::new_v4(), "standard", &pdf, "fixture.txt"),
            "wrong-extension",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_source_extension",
        ),
        (
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((&pdf, "fixture.pdf", "application/octet-stream")),
            ),
            "wrong-media-type",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_source_media_type",
        ),
        (
            duplicate_profile,
            "duplicate-profile",
            StatusCode::UNPROCESSABLE_ENTITY,
            "duplicate_multipart_field",
        ),
        (
            multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                None,
            ),
            "missing-source",
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_source",
        ),
    ] {
        let response = app.submit(body, key, TOKEN).await;
        assert_eq!(response.status(), status, "{key}");
        assert_eq!(json_body(response).await["error"]["code"], code, "{key}");
    }

    let bounded = test_app_with_upload_limits(64, 1).await;
    let response = bounded
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &pdf, "too-large.pdf"),
            "too-large",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "upload_too_large"
    );
    assert_eq!(count_job_directories(bounded.data_dir()), 0);
}

#[tokio::test]
async fn a_body_with_no_container_signature_is_refused_before_the_upload_ends() {
    let app = test_app().await;
    let (mut writer, reader) = tokio::io::duplex(16 * 1024);
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/conversions")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header("idempotency-key", "unadmittable-upload")
        .header(
            "content-type",
            "multipart/form-data; boundary=tool-kit-boundary",
        )
        .body(streaming_body(reader))
        .unwrap();
    let router = app.router();
    let response = tokio::spawn(async move { router.oneshot(request).await.unwrap() });

    // Enough bytes to fill the signature window, and no end to the body: the
    // service must answer off the prefix rather than stream a gigabyte first.
    writer
        .write_all(&slow_multipart_prefix(
            Uuid::new_v4(),
            b"PK\x03\x04not-a-pdf",
        ))
        .await
        .unwrap();
    writer.write_all(&vec![b'x'; 4096]).await.unwrap();

    let response = tokio::time::timeout(Duration::from_secs(5), response)
        .await
        .expect("the signature verdict must not wait for the last chunk")
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "invalid_source_signature"
    );
    drop(writer);
}

#[tokio::test]
async fn concurrent_slow_uploads_receive_immediate_backpressure_and_cleanup() {
    let app = test_app_with_upload_limits(1024 * 1024, 1).await;
    let (mut writer, reader) = tokio::io::duplex(4096);
    writer
        .write_all(&slow_multipart_prefix(Uuid::new_v4(), b"%PDF-1.4\n"))
        .await
        .unwrap();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/conversions")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header("idempotency-key", "slow-upload")
        .header(
            "content-type",
            "multipart/form-data; boundary=tool-kit-boundary",
        )
        .body(streaming_body(reader))
        .unwrap();
    let router = app.router();
    let first = tokio::spawn(async move { router.oneshot(request).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let second = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "second.pdf"),
            "second-during-slow-upload",
            TOKEN,
        )
        .await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        json_body(second).await["error"]["code"],
        "upload_capacity_reached"
    );

    drop(writer);
    let first = first.await.unwrap();
    assert_eq!(first.status(), StatusCode::BAD_REQUEST);
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(count_job_directories(app.data_dir()), 0);
}

#[tokio::test]
async fn non_text_pdf_never_publishes_partial_markdown() {
    let app = test_app().await;
    let pdf = pdf_with_content(b"q\nQ\n");
    let body = multipart_body(Uuid::new_v4(), "local_only", &pdf, "blank.pdf");
    let response = app.submit(body, "blank-pdf-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let submitted = json_body(response).await;
    let job_id = submitted["data"]["id"].as_str().unwrap();
    assert_eq!(submitted["data"]["profile"], "local_only");
    assert_eq!(submitted["data"]["status"], "queued");

    let completed = app.wait_for_terminal(job_id).await;
    assert_eq!(completed["data"]["status"], "needs_remote", "{completed:#}");
    assert_ne!(
        completed["data"]["route"]["reasonCodes"],
        serde_json::json!(["native_text_pdf"])
    );
    let artifacts = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
        .await;
    assert!(json_body(artifacts).await["data"]
        .as_array()
        .unwrap()
        .is_empty());
    let markdown = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(markdown.status(), StatusCode::NOT_FOUND);
    assert_eq!(count_job_directories(app.data_dir()), 1);
    // The status commits before the source is removed, so wait for the runner.
    app.wait_for_job_runner_idle().await;
    assert_eq!(count_named_files(app.data_dir(), "input"), 0);
    assert_eq!(count_named_files(app.data_dir(), "result.md"), 0);
}

#[tokio::test]
async fn worker_output_limit_routes_without_writing_an_artifact() {
    let app = test_app_with_output_limit(16).await;
    let body = multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "bounded.pdf");
    let response = app.submit(body, "bounded-output-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let submitted = json_body(response).await;
    let job_id = submitted["data"]["id"].as_str().unwrap();

    let completed = app.wait_for_terminal(job_id).await;
    assert_eq!(completed["data"]["status"], "needs_remote", "{completed:#}");
    assert_eq!(
        completed["data"]["route"]["reasonCodes"],
        serde_json::json!(["output_too_large"])
    );
    let artifacts = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
        .await;
    assert!(json_body(artifacts).await["data"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn concurrent_idempotent_submissions_create_one_job() {
    let app = test_app().await;
    let body = multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "same.pdf");
    let (first, second) = tokio::join!(
        app.submit(body.clone(), "concurrent-same-1", TOKEN),
        app.submit(body, "concurrent-same-1", TOKEN)
    );
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let replay_headers = [
        first.headers()["idempotency-replayed"]
            .to_str()
            .unwrap()
            .to_owned(),
        second.headers()["idempotency-replayed"]
            .to_str()
            .unwrap()
            .to_owned(),
    ];
    assert!(replay_headers.contains(&"false".to_owned()));
    assert!(replay_headers.contains(&"true".to_owned()));
    let first = json_body(first).await;
    let second = json_body(second).await;
    assert_eq!(first["data"]["id"], second["data"]["id"]);
}

#[tokio::test]
async fn capabilities_stop_accepting_new_jobs_at_active_capacity() {
    let app = test_app_with_max_jobs(1).await;
    let pdf = clean_pdf();
    let client_run_id = Uuid::new_v4();
    let body = multipart_body(client_run_id, "standard", &pdf, "first.pdf");
    let first = app.submit(body.clone(), "capacity-first", TOKEN).await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first_id = json_body(first).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let capabilities = app.request(Method::GET, "/api/v1/capabilities", None).await;
    assert_eq!(
        json_body(capabilities).await["data"]["conversion"]["acceptingJobs"],
        false
    );

    let second = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &pdf, "second.pdf"),
            "capacity-second",
            TOKEN,
        )
        .await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        json_body(second).await["error"]["code"],
        "job_capacity_reached"
    );

    let replay = app.submit(body, "capacity-first", TOKEN).await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(replay.headers()["idempotency-replayed"], "true");
    assert_eq!(json_body(replay).await["data"]["id"], first_id);
    assert_eq!(count_job_directories(app.data_dir()), 1);
    assert_eq!(count_named_files(app.data_dir(), "input.staging"), 0);
    // The replay stored no source, and the first job's own goes when it
    // succeeds.
    let completed = app.wait_for_terminal(&first_id).await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    // The status commits before the source is removed, so wait for the runner.
    app.wait_for_job_runner_idle().await;
    assert_eq!(count_named_files(app.data_dir(), "input"), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn single_runner_keeps_waiting_jobs_queued_and_execution_serial() {
    let app =
        test_app_with_worker_script("#!/bin/sh\n/bin/sleep 1\nexit 42\n", Duration::from_secs(3))
            .await;
    let first = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "first.pdf"),
            "engine-queue-first",
            TOKEN,
        )
        .await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first_id = json_body(first).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut first_started = false;
    for _ in 0..100 {
        let status = app
            .authorized_get(&format!("/api/v1/conversions/{first_id}"))
            .await;
        let status = json_body(status).await;
        if status["data"]["status"] == "converting_local" {
            first_started = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(first_started, "first conversion never acquired the engine");

    let second = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "second.pdf"),
            "engine-queue-second",
            TOKEN,
        )
        .await;
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let second_id = json_body(second).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let waiting = app
        .authorized_get(&format!("/api/v1/conversions/{second_id}"))
        .await;
    assert_eq!(json_body(waiting).await["data"]["status"], "queued");
    assert_eq!(
        app.wait_for_terminal(&first_id).await["data"]["status"],
        "failed"
    );
    assert_eq!(
        app.wait_for_terminal(&second_id).await["data"]["status"],
        "failed"
    );
}

#[tokio::test]
async fn submission_notification_wakes_runner_before_a_long_poll() {
    let app = test_app_with_poll_interval(Duration::from_secs(30)).await;
    app.wait_for_job_runner_idle().await;
    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "notify.pdf"),
            "runner-notify-wake",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id = json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let completed = tokio::time::timeout(Duration::from_secs(3), app.wait_for_terminal(&job_id))
        .await
        .expect("Notify should wake the runner before its 30-second poll");
    assert_eq!(completed["data"]["status"], "succeeded");
    assert!(!app.job_runner_failed());
}

#[tokio::test]
async fn polling_claims_a_job_when_its_notification_was_missed() {
    let harness = TestHarness::with_poll_interval(Duration::from_millis(100));
    let app = harness.app().await;
    app.wait_for_job_runner_idle().await;
    let job_id = harness
        .insert_queued_job(&clean_pdf())
        .await
        .job_id
        .to_string();

    let completed = tokio::time::timeout(Duration::from_secs(3), app.wait_for_terminal(&job_id))
        .await
        .expect("periodic polling should recover a missed notification");
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    assert!(!app.job_runner_failed());
}

#[tokio::test]
async fn idle_runner_shutdown_is_clean_and_bounded() {
    let app = test_app().await;
    app.wait_for_job_runner_idle().await;
    tokio::time::timeout(
        Duration::from_secs(1),
        app.shutdown(Duration::from_millis(250)),
    )
    .await
    .expect("idle shutdown should not consume the outer bound");
    assert!(app.job_runner_stopped());
    assert!(!app.job_runner_failed());
}

#[cfg(unix)]
#[tokio::test]
async fn active_forced_cancellation_leaves_the_claim_recoverable() {
    let app =
        test_app_with_worker_script("#!/bin/sh\nexec /bin/sleep 5\n", Duration::from_secs(10))
            .await;
    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "active.pdf"),
            "runner-active-cancel",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let submitted = json_body(response).await;
    let job_id = submitted["data"]["id"].as_str().unwrap().to_owned();
    let attempt_id = submitted["data"]["activeAttemptId"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut claimed = false;
    for _ in 0..100 {
        let status = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}"))
            .await;
        if json_body(status).await["data"]["status"] == "converting_local" {
            claimed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        claimed,
        "runner never claimed the active cancellation fixture"
    );

    app.stop_job_claiming();
    app.force_cancel_jobs();
    app.shutdown(Duration::from_secs(1)).await;
    assert!(app.job_runner_stopped());
    assert!(!app.job_runner_failed());

    let status = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}"))
        .await;
    assert_eq!(
        json_body(status).await["data"]["status"],
        "converting_local"
    );
    assert!(app
        .data_dir()
        .join("jobs")
        .join(job_id)
        .join("attempts")
        .join(attempt_id)
        .exists());
    assert_eq!(count_named_files(app.data_dir(), "input"), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn persistence_invariant_failure_stops_the_single_runner() {
    let harness = TestHarness::with_worker_script(
        "#!/bin/sh\n/bin/sleep 1\nexit 42\n",
        Duration::from_secs(3),
    );
    let app = harness.app().await;
    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "invariant.pdf"),
            "runner-persistence-invariant",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id =
        Uuid::parse_str(json_body(response).await["data"]["id"].as_str().unwrap()).unwrap();

    let mut claimed = false;
    for _ in 0..100 {
        let status = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}"))
            .await;
        if json_body(status).await["data"]["status"] == "converting_local" {
            claimed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(claimed, "runner never claimed the invariant fixture");
    harness.inject_active_attempt_state(job_id, "failed").await;

    let runner_failed =
        tokio::time::timeout(Duration::from_secs(3), app.wait_for_job_runner_exit())
            .await
            .expect("runner did not expose the fatal persistence invariant");
    assert!(runner_failed);
    assert!(app.job_runner_failed());
    app.shutdown(Duration::from_secs(1)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn worker_timeout_and_crash_fail_only_the_job() {
    for (script, worker_timeout, expected_code) in [
        (
            "#!/bin/sh\nexec /bin/sleep 5\n",
            Duration::from_millis(50),
            "worker_timeout",
        ),
        (
            "#!/bin/sh\nexit 42\n",
            Duration::from_secs(2),
            "worker_crash",
        ),
    ] {
        let app = test_app_with_worker_script(script, worker_timeout).await;
        let body = multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "fixture.pdf");
        let response = app
            .submit(body, &format!("failure-{expected_code}"), TOKEN)
            .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let submitted = json_body(response).await;
        let job_id = submitted["data"]["id"].as_str().unwrap();
        let completed = app.wait_for_terminal(job_id).await;
        assert_eq!(completed["data"]["status"], "failed");
        assert_eq!(completed["data"]["failure"]["code"], expected_code);
        assert_eq!(count_job_directories(app.data_dir()), 1);
        app.wait_for_job_runner_idle().await;
        assert_eq!(count_named_files(app.data_dir(), "input"), 0);
        assert_eq!(count_named_files(app.data_dir(), "result.md"), 0);
        assert!(!app.job_runner_failed());

        let health = app.request(Method::GET, "/health/live", None).await;
        assert_eq!(health.status(), StatusCode::OK);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_or_untrusted_worker_outputs_never_publish() {
    const INSPECTION: &str = r#""inspection":{"pdfType":"text_based","confidence":1.0,"pageCount":1,"pagesNeedingOcr":[],"ocrReasonsByPage":[],"hasEncodingIssues":false,"isComplex":false,"pagesWithTables":[],"pagesWithColumns":[],"processingTimeMs":1}"#;
    let wrong_identity = r#"printf '%s' '{"protocolVersion":3,"engine":{"name":"pdf-inspector","version":"0.0.0","features":[]},"outcome":{"kind":"rejected","code":"invalid_pdf"}}' > "$1/worker-report.json"
"#;
    let bad_hash = format!(
        "printf X > \"$1/result.md\"\nprintf '%s' '{{\"protocolVersion\":3,\"engine\":{{\"name\":\"pdf-inspector\",\"version\":\"1.25.2\",\"features\":[]}},\"outcome\":{{\"kind\":\"converted\",{INSPECTION},\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":1,\"sha256\":\"{}\"}}}}}}' > \"$1/worker-report.json\"\n",
        "0".repeat(64)
    );
    let symlink = format!(
        "ln -s /etc/passwd \"$1/result.md\"\nprintf '%s' '{{\"protocolVersion\":3,\"engine\":{{\"name\":\"pdf-inspector\",\"version\":\"1.25.2\",\"features\":[]}},\"outcome\":{{\"kind\":\"converted\",{INSPECTION},\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":1,\"sha256\":\"{}\"}}}}}}' > \"$1/worker-report.json\"\n",
        "0".repeat(64)
    );
    let extra_file = format!(
        "printf X > \"$1/result.md\"\nprintf extra > \"$1/unexpected.bin\"\nprintf '%s' '{{\"protocolVersion\":3,\"engine\":{{\"name\":\"pdf-inspector\",\"version\":\"1.25.2\",\"features\":[]}},\"outcome\":{{\"kind\":\"converted\",{INSPECTION},\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":1,\"sha256\":\"4b68ab3847feda7d6c62c1fbcbeebfa35eab7351ed5e78f4ddadea5df64b8015\"}}}}}}' > \"$1/worker-report.json\"\n"
    );
    let cases = [
        (
            "malformed-report",
            "printf not-json > \"$1/worker-report.json\"\n".to_owned(),
            "worker_protocol_error",
        ),
        (
            "wrong-identity",
            wrong_identity.to_owned(),
            "worker_protocol_error",
        ),
        ("hash-mismatch", bad_hash, "worker_protocol_error"),
        ("symlink-output", symlink, "worker_protocol_error"),
        (
            "unexpected-publication-file",
            extra_file,
            "artifact_publication_failed",
        ),
    ];

    for (name, script, expected_code) in cases {
        let app = test_app_with_worker_script(&script, Duration::from_secs(2)).await;
        let response = app
            .submit(
                multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "fixture.pdf"),
                name,
                TOKEN,
            )
            .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED, "{name}");
        let submitted = json_body(response).await;
        let job_id = submitted["data"]["id"].as_str().unwrap();
        let completed = app.wait_for_terminal(job_id).await;
        assert_eq!(
            completed["data"]["status"], "failed",
            "{name}: {completed:#}"
        );
        assert_eq!(
            completed["data"]["failure"]["code"], expected_code,
            "{name}: {completed:#}"
        );
        let artifacts = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
            .await;
        assert!(json_body(artifacts).await["data"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            app.request(Method::GET, "/health/live", None)
                .await
                .status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn server_replaces_spoofed_request_ids_on_errors_and_success() {
    let app = test_app().await;
    for uri in ["/health/live", "/missing"] {
        let response = app
            .request_with_headers(
                Method::GET,
                uri,
                Body::empty(),
                &[("x-request-id", "client-controlled")],
            )
            .await;
        let request_id = response.headers()["x-request-id"].to_str().unwrap();
        assert_ne!(request_id, "client-controlled");
        assert!(Uuid::parse_str(request_id).is_ok());
    }
}

#[tokio::test]
async fn persistence_dependent_routes_remain_absent() {
    let app = test_app().await;
    for (method, path, expected) in [
        (
            Method::GET,
            "/api/v1/conversions?clientRunId=x",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            Method::POST,
            "/api/v1/conversions/00000000-0000-0000-0000-000000000000/cancel",
            StatusCode::NOT_FOUND,
        ),
        (
            Method::POST,
            "/api/v1/conversions/00000000-0000-0000-0000-000000000000/retries",
            StatusCode::NOT_FOUND,
        ),
        (
            Method::DELETE,
            "/api/v1/conversions/00000000-0000-0000-0000-000000000000",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
    ] {
        let response = app
            .request_with_headers(
                method,
                path,
                Body::empty(),
                &[(AUTHORIZATION.as_str(), &format!("Bearer {TOKEN}"))],
            )
            .await;
        assert_eq!(response.status(), expected, "{path}");
    }
}
