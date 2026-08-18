#![allow(clippy::unwrap_used)]

mod support;

use std::{
    net::SocketAddr,
    path::Path,
    time::{Duration, Instant},
};

use axum::http::{Method, StatusCode};
use http_body_util::BodyExt;
use sqlx::{sqlite::SqliteConnectOptions, Connection, Executor, SqliteConnection};
use uuid::Uuid;

use tool_kit_converter::{
    config::{Limits, Settings},
    persistence::{RepositoryError, SqliteRepository, DATABASE_FILENAME},
    AppState,
};

use support::{
    clean_pdf, count_job_directories, count_named_files, json_body, multipart_body,
    test_app_with_worker_script, TestHarness, TOKEN,
};

/// Counts the pre-acceptance quarantine entries. A clean rejection never
/// quarantines, so a non-zero count is a leak the job-directory count misses.
fn count_quarantined_sources(data_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(data_dir.join("quarantine").join("pre-acceptance")) else {
        return 0;
    };
    entries.filter_map(Result::ok).count()
}

fn database_options(data_dir: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(data_dir.join(DATABASE_FILENAME))
        .foreign_keys(true)
}

/// A conflicting key is answered before the capacity check, so its cleanup runs
/// on a branch no other capacity test reaches.
#[tokio::test]
async fn conflicting_key_at_active_capacity_leaves_no_staged_source() {
    const KEY: &str = "conflict-at-capacity";

    let harness = TestHarness::with_max_jobs(1);
    let app = harness.app().await;
    let pdf = clean_pdf();

    let accepted = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &pdf, "first.pdf"),
            KEY,
            TOKEN,
        )
        .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let first_id = json_body(accepted).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let completed = app.wait_for_terminal(&first_id).await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");

    // Stop the runner before seeding, otherwise it can claim the seeded row
    // between the insert and the converting_local transition.
    app.stop_job_claiming();
    assert!(!app.wait_for_job_runner_exit().await);
    harness.insert_converting_job(&pdf, 0).await;

    // The single active slot now belongs to the seeded converting_local row.
    let capabilities = app.request(Method::GET, "/api/v1/capabilities", None).await;
    assert_eq!(
        json_body(capabilities).await["data"]["conversion"]["acceptingJobs"],
        false
    );

    let jobs_before = count_job_directories(app.data_dir());
    let sources_before = count_named_files(app.data_dir(), "input");
    assert_eq!(jobs_before, 2);
    assert_eq!(sources_before, 2);

    // Same key, different client run id, so the fingerprint differs and the
    // conflict arm returns before the active-count query runs.
    let conflict = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &pdf, "second.pdf"),
            KEY,
            TOKEN,
        )
        .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(conflict).await["error"]["code"],
        "idempotency_conflict"
    );

    assert_eq!(count_job_directories(app.data_dir()), jobs_before);
    assert_eq!(count_named_files(app.data_dir(), "input"), sources_before);
    assert_eq!(count_named_files(app.data_dir(), "input.staging"), 0);
    assert_eq!(count_quarantined_sources(app.data_dir()), 0);
}

#[cfg(unix)]
const INSPECTION: &str = r#""inspection":{"pdfType":"text_based","confidence":1.0,"pageCount":1,"pagesNeedingOcr":[],"ocrReasonsByPage":[],"hasEncodingIssues":false,"isComplex":false,"pagesWithTables":[],"pagesWithColumns":[],"processingTimeMs":1}"#;

/// A worker that takes two seconds and then publishes one valid byte, so the
/// conversion is provably still running when shutdown starts.
#[cfg(unix)]
fn slow_successful_worker() -> String {
    let digest = "4b68ab3847feda7d6c62c1fbcbeebfa35eab7351ed5e78f4ddadea5df64b8015";
    format!(
        "/bin/sleep 2\nprintf X > \"$1/result.md\"\nprintf '%s' '{{\"protocolVersion\":2,\"engine\":{{\"name\":\"pdf-inspector\",\"version\":\"1.15.0\",\"features\":[]}},\"outcome\":{{\"kind\":\"converted\",{INSPECTION},\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":1,\"sha256\":\"{digest}\"}}}}}}' > \"$1/worker-report.json\"\n"
    )
}

/// The forced-cancellation test never lets the drain branch of `shutdown_until`
/// run, so this one proves an in-flight conversion finishes inside the grace.
#[cfg(unix)]
#[tokio::test]
async fn graceful_shutdown_drains_an_active_conversion() {
    const GRACE: Duration = Duration::from_secs(10);

    let app = test_app_with_worker_script(&slow_successful_worker(), Duration::from_secs(10)).await;
    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "draining.pdf"),
            "graceful-drain",
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id = json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut converting = false;
    for _ in 0..400 {
        let status = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}"))
            .await;
        if json_body(status).await["data"]["status"] == "converting_local" {
            converting = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(converting, "the runner never started the drain fixture");

    let started = Instant::now();
    tokio::time::timeout(GRACE * 3, app.shutdown(GRACE))
        .await
        .expect("graceful shutdown must not outlive its own grace");
    let elapsed = started.elapsed();
    assert!(
        elapsed < GRACE,
        "shutdown drained in {elapsed:?}, which is not inside the {GRACE:?} grace"
    );
    assert!(app.job_runner_stopped());
    assert!(!app.job_runner_failed());

    let status = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}"))
        .await;
    let status = json_body(status).await;
    assert_eq!(status["data"]["status"], "succeeded", "{status:#}");

    let markdown = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(markdown.status(), StatusCode::OK);
    let body = markdown.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), b"X");
}

/// A second writer holding the database must surface a bounded API error rather
/// than parking a request forever.
#[tokio::test]
async fn locked_database_fails_cleanly_within_the_busy_timeout() {
    let harness = TestHarness::new();
    let app = harness.app().await;

    // The idle runner also opens write transactions. Stop it so the only
    // contender for the lock is the request under test.
    app.stop_job_claiming();
    assert!(!app.wait_for_job_runner_exit().await);

    let mut blocker = SqliteConnection::connect_with(&database_options(harness.data_dir()))
        .await
        .unwrap();
    (&mut blocker).execute("BEGIN EXCLUSIVE").await.unwrap();

    let started = Instant::now();
    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "locked.pdf"),
            "locked-database",
            TOKEN,
        )
        .await;
    let elapsed = started.elapsed();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "conversion_unavailable"
    );
    // The configured busy timeout is five seconds. The bounds stay wide so a
    // loaded machine cannot turn scheduling noise into a failure.
    assert!(
        elapsed >= Duration::from_secs(2),
        "the request gave up after {elapsed:?}, so it never waited on the busy handler"
    );
    assert!(
        elapsed < Duration::from_secs(60),
        "the request took {elapsed:?}, which is not a bounded failure"
    );

    (&mut blocker).execute("ROLLBACK").await.unwrap();
    blocker.close().await.unwrap();

    assert_eq!(count_job_directories(app.data_dir()), 0);
    assert_eq!(count_named_files(app.data_dir(), "input.staging"), 0);
    assert_eq!(count_quarantined_sources(app.data_dir()), 0);
}

/// Serving from a data root the process cannot write to would accept jobs it can
/// never store, so startup has to refuse.
#[cfg(unix)]
#[tokio::test]
async fn unwritable_data_root_fails_startup_instead_of_serving() {
    use std::{fs::Permissions, os::unix::fs::PermissionsExt};

    let workspace = tempfile::tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let token_file = workspace.path().join("bootstrap-token");
    std::fs::create_dir(&data_dir).unwrap();
    std::fs::write(&token_file, format!("{TOKEN}\n")).unwrap();
    std::fs::set_permissions(&data_dir, Permissions::from_mode(0o500)).unwrap();

    // Root and permission-free filesystems ignore the mode bits, so a probe
    // write that succeeds means this machine cannot host the test at all.
    let probe = data_dir.join("write-probe");
    if std::fs::write(&probe, b"probe").is_ok() {
        std::fs::remove_file(&probe).unwrap();
        std::fs::set_permissions(&data_dir, Permissions::from_mode(0o700)).unwrap();
        eprintln!(
            "SKIPPED unwritable_data_root_fails_startup_instead_of_serving: writing into a \
             0o500 directory succeeded, so this user (root?) or filesystem cannot make a data \
             root unwritable"
        );
        return;
    }

    let settings = Settings {
        bind_address: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        log_filter: "tool_kit_converter=info".to_owned(),
        token_file,
        data_dir: data_dir.clone(),
        scratch_parent: data_dir.clone(),
        pdf_worker_path: env!("CARGO_BIN_EXE_tool-kit-pdf-worker").into(),
        pdf_bcmaps_dir: None,
        limits: Limits {
            max_upload_bytes: 1024 * 1024,
            max_output_bytes: 2 * 1024 * 1024,
            max_jobs: 8,
            max_concurrent_uploads: 2,
            upload_timeout: Duration::from_secs(5),
            pdf_timeout: Duration::from_secs(10),
        },
        pdf_threads: 2,
        database_busy_timeout: Duration::from_secs(5),
        worker_poll_interval: Duration::from_secs(1),
        recovery_limit: 3,
        shutdown_grace: Duration::from_secs(30),
    };

    let error = AppState::initialize(&settings)
        .await
        .expect_err("an unwritable data root must fail startup");
    let message = error.to_string();
    assert!(
        message.contains("cannot initialize artifact storage"),
        "startup failed for an unrelated reason: {message}"
    );

    std::fs::set_permissions(&data_dir, Permissions::from_mode(0o700)).unwrap();
}

/// A modified migration means the schema on disk is not the schema the code was
/// written against, so opening the database has to fail instead of guessing.
#[tokio::test]
async fn migration_failure_prevents_startup() {
    let harness = TestHarness::new();
    let app = harness.app().await;
    app.shutdown(Duration::from_secs(1)).await;
    drop(app);

    let mut connection = SqliteConnection::connect_with(&database_options(harness.data_dir()))
        .await
        .unwrap();
    let corrupted = sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1")
        .execute(&mut connection)
        .await
        .unwrap();
    assert_eq!(corrupted.rows_affected(), 1);
    connection.close().await.unwrap();

    let error = SqliteRepository::open(harness.data_dir(), 8, Duration::from_secs(5))
        .await
        .expect_err("a mismatched migration checksum must fail the open");
    assert!(
        matches!(error, RepositoryError::Migration(_)),
        "expected a migration failure, got {error}"
    );
}
