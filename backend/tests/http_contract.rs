mod support;

use std::time::Duration;

use axum::{
    body::Body,
    http::{header::AUTHORIZATION, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use serde_yaml_ng::Value as YamlValue;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tower::ServiceExt;
use uuid::Uuid;

use support::{
    assert_server_request_id, clean_pdf, count_named_files, json_body, multipart_body,
    multipart_body_with_duplicate_profile, multipart_body_with_media_type,
    multipart_body_without_source, pdf_with_content, slow_multipart_prefix, streaming_body,
    test_app, test_app_with_max_jobs, test_app_with_output_limit, test_app_with_upload_limits,
    test_app_with_worker_script, TestHarness, TOKEN,
};

#[tokio::test]
async fn public_health_and_capabilities_are_truthful() {
    let app = test_app();
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
    assert_eq!(conversion["durability"], "ephemeral");
    assert_eq!(
        conversion["inputFormats"],
        serde_json::json!(["application/pdf"])
    );
    assert_eq!(conversion["engine"]["version"], "1.15.0");
    assert_eq!(payload["data"]["remoteFallback"]["available"], false);
}

#[tokio::test]
async fn harness_reinitializes_app_state_against_one_data_root_without_a_listener() {
    let harness = TestHarness::new();
    let data_dir = harness.data_dir().to_owned();

    let first = harness.app();
    assert_eq!(first.data_dir(), data_dir);
    assert_eq!(
        first
            .request(Method::GET, "/health/ready", None)
            .await
            .status(),
        StatusCode::OK
    );
    drop(first);

    let second = harness.app();
    assert_eq!(second.data_dir(), data_dir);
    assert_eq!(
        second
            .request(Method::GET, "/health/ready", None)
            .await
            .status(),
        StatusCode::OK
    );
}

#[test]
fn conversion_profile_and_job_status_json_values_are_stable() {
    let document: YamlValue = serde_yaml_ng::from_str(include_str!("../openapi/openapi.yaml"))
        .expect("OpenAPI must be valid YAML");
    let profile_values = document["components"]["schemas"]["ConversionProfile"]["enum"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    let status_values = document["components"]["schemas"]["ConversionJob"]["properties"]["status"]
        ["enum"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(profile_values, ["standard", "local_only", "best_quality"]);
    assert_eq!(
        status_values,
        [
            "queued",
            "converting_local",
            "finalizing",
            "succeeded",
            "failed",
            "needs_remote",
        ]
    );
}

#[tokio::test]
async fn conversion_routes_require_one_valid_bearer_token() {
    let app = test_app();
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
    assert_eq!(count_named_files(app.data_dir(), "source.pdf"), 0);
}

#[tokio::test]
async fn clean_pdf_completes_and_idempotency_replays_the_job() {
    let app = test_app();
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
    assert_eq!(manifest["engine"]["version"], "1.15.0");
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
async fn invalid_submission_fields_are_rejected_without_creating_jobs() {
    let app = test_app();
    let pdf = clean_pdf();

    let wrong_profile = multipart_body(Uuid::new_v4(), "best_quality", &pdf, "fixture.pdf");
    let response = app.submit(wrong_profile, "profile-1", TOKEN).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "profile_unavailable"
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
        "invalid_pdf_signature"
    );
}

#[tokio::test]
async fn multipart_boundaries_enforce_fields_media_type_extension_and_size() {
    let app = test_app();
    let pdf = clean_pdf();
    for (body, key, status, code) in [
        (
            multipart_body(Uuid::new_v4(), "standard", &pdf, "fixture.txt"),
            "wrong-extension",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_pdf_filename",
        ),
        (
            multipart_body_with_media_type(
                Uuid::new_v4(),
                "standard",
                &pdf,
                "fixture.pdf",
                "application/octet-stream",
            ),
            "wrong-media-type",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_pdf_media_type",
        ),
        (
            multipart_body_with_duplicate_profile(Uuid::new_v4(), &pdf),
            "duplicate-profile",
            StatusCode::UNPROCESSABLE_ENTITY,
            "duplicate_multipart_field",
        ),
        (
            multipart_body_without_source(Uuid::new_v4(), "standard"),
            "missing-source",
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_source",
        ),
    ] {
        let response = app.submit(body, key, TOKEN).await;
        assert_eq!(response.status(), status, "{key}");
        assert_eq!(json_body(response).await["error"]["code"], code, "{key}");
    }

    let bounded = test_app_with_upload_limits(64, 1);
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
    assert_eq!(count_named_files(bounded.data_dir(), "source.pdf"), 0);
}

#[tokio::test]
async fn concurrent_slow_uploads_receive_immediate_backpressure_and_cleanup() {
    let app = test_app_with_upload_limits(1024 * 1024, 1);
    let (mut writer, reader) = tokio::io::duplex(4096);
    writer
        .write_all(&slow_multipart_prefix(Uuid::new_v4()))
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
    assert_eq!(count_named_files(app.data_dir(), "source.pdf"), 0);
}

#[tokio::test]
async fn non_text_pdf_never_publishes_partial_markdown() {
    let app = test_app();
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
}

#[tokio::test]
async fn worker_output_limit_routes_without_writing_an_artifact() {
    let app = test_app_with_output_limit(16);
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
    let app = test_app();
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
async fn capabilities_stop_accepting_new_jobs_at_ephemeral_capacity() {
    let app = test_app_with_max_jobs(1);
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
}

#[cfg(unix)]
#[tokio::test]
async fn worker_timeout_and_crash_fail_only_the_job() {
    for (script, worker_timeout, expected_code) in [
        (
            "#!/bin/sh\n/bin/sleep 5\n",
            Duration::from_millis(50),
            "worker_timeout",
        ),
        (
            "#!/bin/sh\nexit 42\n",
            Duration::from_secs(2),
            "worker_crash",
        ),
    ] {
        let app = test_app_with_worker_script(script, worker_timeout);
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

        let health = app.request(Method::GET, "/health/live", None).await;
        assert_eq!(health.status(), StatusCode::OK);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_or_untrusted_worker_outputs_never_publish() {
    const INSPECTION: &str = r#""inspection":{"pdfType":"text_based","confidence":1.0,"pageCount":1,"pagesNeedingOcr":[],"ocrReasonsByPage":[],"hasEncodingIssues":false,"isComplex":false,"pagesWithTables":[],"pagesWithColumns":[],"processingTimeMs":1}"#;
    let wrong_identity = r#"printf '%s' '{"protocolVersion":1,"engine":{"name":"pdf-inspector","version":"0.0.0","features":[]},"outcome":{"kind":"rejected","code":"invalid_pdf"}}' > "$2/worker-report.json"
"#;
    let bad_hash = format!(
        "printf X > \"$2/result.md\"\nprintf '%s' '{{\"protocolVersion\":1,\"engine\":{{\"name\":\"pdf-inspector\",\"version\":\"1.15.0\",\"features\":[]}},\"outcome\":{{\"kind\":\"converted\",{INSPECTION},\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":1,\"sha256\":\"{}\"}}}}}}' > \"$2/worker-report.json\"\n",
        "0".repeat(64)
    );
    let symlink = format!(
        "ln -s /etc/passwd \"$2/result.md\"\nprintf '%s' '{{\"protocolVersion\":1,\"engine\":{{\"name\":\"pdf-inspector\",\"version\":\"1.15.0\",\"features\":[]}},\"outcome\":{{\"kind\":\"converted\",{INSPECTION},\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":1,\"sha256\":\"{}\"}}}}}}' > \"$2/worker-report.json\"\n",
        "0".repeat(64)
    );
    let extra_file = format!(
        "printf X > \"$2/result.md\"\nprintf extra > \"$2/unexpected.bin\"\nprintf '%s' '{{\"protocolVersion\":1,\"engine\":{{\"name\":\"pdf-inspector\",\"version\":\"1.15.0\",\"features\":[]}},\"outcome\":{{\"kind\":\"converted\",{INSPECTION},\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":1,\"sha256\":\"4b68ab3847feda7d6c62c1fbcbeebfa35eab7351ed5e78f4ddadea5df64b8015\"}}}}}}' > \"$2/worker-report.json\"\n"
    );
    let cases = [
        (
            "malformed-report",
            "printf not-json > \"$2/worker-report.json\"\n".to_owned(),
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
        let app = test_app_with_worker_script(&script, Duration::from_secs(2));
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
    let app = test_app();
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
    let app = test_app();
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

#[test]
fn openapi_parses_and_documents_only_the_live_routes() {
    let spec = include_str!("../openapi/openapi.yaml");
    let document: YamlValue = serde_yaml_ng::from_str(spec).expect("OpenAPI must be valid YAML");
    let paths = document["paths"]
        .as_mapping()
        .expect("OpenAPI paths must be a mapping");

    assert_eq!(document["openapi"].as_str(), Some("3.1.0"));
    for (path, method) in [
        ("/health/live", "get"),
        ("/health/ready", "get"),
        ("/api/v1/capabilities", "get"),
        ("/api/v1/conversions", "post"),
        ("/api/v1/conversions/{id}", "get"),
        ("/api/v1/conversions/{id}/artifacts", "get"),
        ("/api/v1/conversions/{id}/artifacts/markdown", "get"),
        ("/api/v1/conversions/{id}/artifacts/manifest", "get"),
    ] {
        assert!(
            !document["paths"][path][method].is_null(),
            "missing {method} {path}"
        );
    }
    assert_eq!(paths.len(), 8);
    assert!(document["paths"]["/api/v1/conversions"]["get"].is_null());
    assert!(!document["components"]["securitySchemes"]["bootstrapBearer"].is_null());
    assert_eq!(
        document["paths"]["/api/v1/conversions"]["post"]["security"][0]["bootstrapBearer"]
            .as_sequence()
            .unwrap()
            .len(),
        0
    );
    for status in [
        "202", "400", "401", "408", "409", "413", "415", "422", "429", "500",
    ] {
        assert!(
            !document["paths"]["/api/v1/conversions"]["post"]["responses"][status].is_null(),
            "POST response {status} is undocumented"
        );
    }
    for path in [
        "/api/v1/conversions/{id}",
        "/api/v1/conversions/{id}/artifacts",
        "/api/v1/conversions/{id}/artifacts/markdown",
        "/api/v1/conversions/{id}/artifacts/manifest",
    ] {
        assert!(
            !document["paths"][path]["get"]["security"][0]["bootstrapBearer"].is_null(),
            "GET {path} must require bearer authentication"
        );
    }
    let submission = &document["components"]["schemas"]["ConversionSubmission"];
    assert_eq!(submission["additionalProperties"].as_bool(), Some(false));
    assert_eq!(submission["required"].as_sequence().unwrap().len(), 3);
    assert_eq!(
        document["paths"]["/api/v1/conversions"]["post"]["requestBody"]["content"]
            ["multipart/form-data"]["schema"]["$ref"]
            .as_str(),
        Some("#/components/schemas/ConversionSubmission")
    );
    for header in [
        "Location",
        "Retry-After",
        "Idempotency-Replayed",
        "X-Request-Id",
    ] {
        assert!(
            !document["paths"]["/api/v1/conversions"]["post"]["responses"]["202"]["headers"]
                [header]
                .is_null(),
            "202 header {header} is undocumented"
        );
    }
    assert!(
        document["components"]["schemas"]["ConversionCapabilities"]["properties"]["acceptingJobs"]
            ["const"]
            .is_null(),
        "acceptingJobs is dynamic"
    );
}
