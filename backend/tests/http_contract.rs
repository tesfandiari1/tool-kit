use std::{io::Write as _, net::SocketAddr, path::PathBuf, time::Duration};

use axum::{
    body::Body,
    http::{header::AUTHORIZATION, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::Value;
use serde_yaml_ng::Value as YamlValue;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use tower::ServiceExt;
use uuid::Uuid;

use tool_kit_converter::{
    config::{Limits, Settings},
    router, AppState,
};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

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
            Body::from_stream(ReaderStream::new(reader)),
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
    assert_eq!(count_named_files(app._directory.path(), "source.pdf"), 0);
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
    assert_eq!(
        count_named_files(bounded._directory.path(), "source.pdf"),
        0
    );
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
        .body(Body::from_stream(ReaderStream::new(reader)))
        .unwrap();
    let router = app.router.clone();
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
    assert_eq!(count_named_files(app._directory.path(), "source.pdf"), 0);
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

struct TestApp {
    router: Router,
    _directory: TempDir,
}

impl TestApp {
    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Body>,
    ) -> axum::response::Response {
        self.request_with_headers(method, uri, body.unwrap_or_else(Body::empty), &[])
            .await
    }

    async fn request_with_headers(
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

    async fn submit(
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

    async fn authorized_get(&self, uri: &str) -> axum::response::Response {
        self.request_with_headers(
            Method::GET,
            uri,
            Body::empty(),
            &[(AUTHORIZATION.as_str(), &format!("Bearer {TOKEN}"))],
        )
        .await
    }

    async fn wait_for_terminal(&self, job_id: &str) -> Value {
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

fn test_app() -> TestApp {
    let directory = tempfile::tempdir().unwrap();
    let worker_path = PathBuf::from(env!("CARGO_BIN_EXE_tool-kit-pdf-worker"));
    build_test_app(
        directory,
        worker_path,
        Duration::from_secs(10),
        1024 * 1024,
        2 * 1024 * 1024,
        8,
        2,
    )
}

fn test_app_with_output_limit(max_output_bytes: u64) -> TestApp {
    let directory = tempfile::tempdir().unwrap();
    let worker_path = PathBuf::from(env!("CARGO_BIN_EXE_tool-kit-pdf-worker"));
    build_test_app(
        directory,
        worker_path,
        Duration::from_secs(10),
        1024 * 1024,
        max_output_bytes,
        8,
        2,
    )
}

fn test_app_with_max_jobs(max_jobs: usize) -> TestApp {
    let directory = tempfile::tempdir().unwrap();
    let worker_path = PathBuf::from(env!("CARGO_BIN_EXE_tool-kit-pdf-worker"));
    build_test_app(
        directory,
        worker_path,
        Duration::from_secs(10),
        1024 * 1024,
        2 * 1024 * 1024,
        max_jobs,
        2,
    )
}

fn test_app_with_upload_limits(max_upload_bytes: u64, max_concurrent_uploads: usize) -> TestApp {
    let directory = tempfile::tempdir().unwrap();
    let worker_path = PathBuf::from(env!("CARGO_BIN_EXE_tool-kit-pdf-worker"));
    build_test_app(
        directory,
        worker_path,
        Duration::from_secs(10),
        max_upload_bytes,
        2 * 1024 * 1024,
        8,
        max_concurrent_uploads,
    )
}

#[cfg(unix)]
fn test_app_with_worker_script(script: &str, worker_timeout: Duration) -> TestApp {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let worker_path = directory.path().join("fake-worker");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'tool-kit-pdf-worker protocol=1 pdf-inspector=1.15.0'\n  exit 0\nfi\n{}",
        script.strip_prefix("#!/bin/sh\n").unwrap_or(script)
    );
    std::fs::write(&worker_path, script).unwrap();
    std::fs::set_permissions(&worker_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    build_test_app(
        directory,
        worker_path,
        worker_timeout,
        1024 * 1024,
        2 * 1024 * 1024,
        8,
        2,
    )
}

fn build_test_app(
    directory: TempDir,
    worker_path: PathBuf,
    worker_timeout: Duration,
    max_upload_bytes: u64,
    max_output_bytes: u64,
    max_jobs: usize,
    max_concurrent_uploads: usize,
) -> TestApp {
    let token_file = directory.path().join("bootstrap-token");
    std::fs::write(&token_file, format!("{TOKEN}\n")).unwrap();
    let settings = Settings {
        bind_address: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        log_filter: "tool_kit_converter=info".to_owned(),
        token_file,
        scratch_parent: directory.path().to_owned(),
        pdf_worker_path: worker_path,
        pdf_bcmaps_dir: None,
        limits: Limits {
            max_upload_bytes,
            max_output_bytes,
            max_jobs,
            max_concurrent_uploads,
            upload_timeout: Duration::from_secs(5),
            pdf_timeout: worker_timeout,
        },
        pdf_threads: 2,
    };
    let state = AppState::initialize(&settings).unwrap();
    TestApp {
        router: router(state),
        _directory: directory,
    }
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn assert_server_request_id(response: &axum::response::Response) {
    let request_id = response.headers()["x-request-id"].to_str().unwrap();
    assert!(Uuid::parse_str(request_id).is_ok());
}

fn multipart_body(client_run_id: Uuid, profile: &str, source: &[u8], filename: &str) -> Vec<u8> {
    multipart_body_with_media_type(client_run_id, profile, source, filename, "application/pdf")
}

fn multipart_body_with_media_type(
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

fn multipart_body_with_duplicate_profile(client_run_id: Uuid, source: &[u8]) -> Vec<u8> {
    let mut body = multipart_body(client_run_id, "standard", source, "fixture.pdf");
    let closing = b"--tool-kit-boundary--\r\n";
    body.truncate(body.len() - closing.len());
    body.extend_from_slice(
        b"--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\nlocal_only\r\n--tool-kit-boundary--\r\n",
    );
    body
}

fn multipart_body_without_source(client_run_id: Uuid, profile: &str) -> Vec<u8> {
    format!(
        "--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"clientRunId\"\r\n\r\n{client_run_id}\r\n--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\n{profile}\r\n--tool-kit-boundary--\r\n"
    )
    .into_bytes()
}

fn slow_multipart_prefix(client_run_id: Uuid) -> Vec<u8> {
    format!(
        "--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"clientRunId\"\r\n\r\n{client_run_id}\r\n--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"profile\"\r\n\r\nstandard\r\n--tool-kit-boundary\r\nContent-Disposition: form-data; name=\"source\"; filename=\"slow.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.4\n"
    )
    .into_bytes()
}

fn count_named_files(root: &std::path::Path, expected: &str) -> usize {
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

fn clean_pdf() -> Vec<u8> {
    let content = b"BT\n/F1 18 Tf\n72 720 Td\n(Clean PDF Fixture) Tj\n0 -24 Td\n(Second native text line) Tj\n0 -24 Td\n(Third native text line) Tj\nET\n";
    pdf_with_content(content)
}

fn pdf_with_content(content: &[u8]) -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{}endstream", content.len(), String::from_utf8_lossy(content)).into_bytes(),
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
