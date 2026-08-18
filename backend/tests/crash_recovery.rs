mod support;

use std::{fs, path::PathBuf, time::Duration};

use axum::http::StatusCode;
use http_body_util::BodyExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use tool_kit_converter::faults::FaultPoint;

use support::{clean_pdf, json_body, multipart_body, SeededJob, TestApp, TestHarness, TOKEN};

/// A parked worker never resumes, so an unreached barrier would hang the test
/// run forever without a ceiling on the wait.
const BARRIER_TIMEOUT: Duration = Duration::from_secs(30);

/// Submits one clean PDF and returns once the worker has parked on `point`.
/// The barrier is armed before the submit, because a worker that reaches the
/// point first runs straight past it.
async fn park_at(app: &TestApp, point: FaultPoint, idempotency_key: &str) -> SeededJob {
    let barrier = app.fault_barrier();
    barrier.arm(point);

    let response = app
        .submit(
            multipart_body(Uuid::new_v4(), "standard", &clean_pdf(), "fixture.pdf"),
            idempotency_key,
            TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = json_body(response).await;
    let seeded = SeededJob {
        job_id: Uuid::parse_str(accepted["data"]["id"].as_str().unwrap()).unwrap(),
        attempt_id: Uuid::parse_str(accepted["data"]["activeAttemptId"].as_str().unwrap()).unwrap(),
    };

    tokio::time::timeout(BARRIER_TIMEOUT, barrier.wait_reached())
        .await
        .expect("the worker never reached the armed fault point");
    seeded
}

/// One attempt's durable directories, read from the data root instead of from
/// the service that wrote them.
struct AttemptFiles {
    attempt: PathBuf,
    staging: PathBuf,
    published: PathBuf,
}

impl AttemptFiles {
    fn of(harness: &TestHarness, seeded: SeededJob) -> Self {
        let attempt = harness
            .data_dir()
            .join("jobs")
            .join(seeded.job_id.to_string())
            .join("attempts")
            .join(seeded.attempt_id.to_string());
        Self {
            staging: attempt.join("publication.staging"),
            published: attempt.join("artifacts"),
            attempt,
        }
    }

    fn markdown(&self) -> PathBuf {
        self.published.join("result.md")
    }

    fn manifest(&self) -> PathBuf {
        self.published.join("manifest.json")
    }
}

/// The durable state every post-publication barrier must show: the rename is on
/// disk, the two published files agree with each other, and SQLite still says
/// `finalizing` with no artifact rows.
async fn assert_published_but_uncommitted(harness: &TestHarness, seeded: SeededJob) {
    let (status, active_attempt) = harness.stored_status(seeded.job_id).await;
    assert_eq!(status, "finalizing");
    assert_eq!(active_attempt, seeded.attempt_id);
    assert_eq!(harness.attempt_states(seeded.job_id).await, ["finalizing"]);
    assert_eq!(harness.artifact_row_count(seeded.attempt_id).await, 0);

    let files = AttemptFiles::of(harness, seeded);
    assert!(
        !files.staging.exists(),
        "publication renames the staging directory away"
    );
    assert!(files.published.is_dir());

    let markdown = fs::read(files.markdown()).unwrap();
    assert!(!markdown.is_empty());
    let manifest: Value = serde_json::from_slice(&fs::read(files.manifest()).unwrap()).unwrap();
    assert_eq!(manifest["attemptId"], seeded.attempt_id.to_string());
    assert_eq!(
        manifest["output"]["sha256"],
        hex::encode(Sha256::digest(&markdown)),
        "the published pair must be complete, not half-written"
    );
}

/// Downloads both artifacts and checks each against the hash the success commit
/// recorded, so "succeeded" is only accepted when a client can fetch the bytes.
async fn assert_artifacts_download(app: &TestApp, job_id: Uuid) {
    let listing = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts"))
        .await;
    assert_eq!(listing.status(), StatusCode::OK);
    let listing = json_body(listing).await;
    let artifacts = listing["data"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2, "{listing:#}");

    for artifact in artifacts {
        let kind = artifact["kind"].as_str().unwrap();
        let download = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/{kind}"))
            .await;
        assert_eq!(download.status(), StatusCode::OK, "{kind}");
        let bytes = download.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            artifact["byteLength"].as_u64().unwrap(),
            bytes.len() as u64,
            "{kind}"
        );
        assert_eq!(
            artifact["sha256"],
            hex::encode(Sha256::digest(&bytes)),
            "{kind}"
        );
    }
}

#[tokio::test]
async fn crash_after_claim_requeues_a_fresh_attempt() {
    let harness = TestHarness::new();
    let crashed = harness.app().await;
    let seeded = park_at(&crashed, FaultPoint::AfterClaim, "crash-after-claim").await;

    let (status, active_attempt) = harness.stored_status(seeded.job_id).await;
    assert_eq!(status, "converting_local");
    assert_eq!(active_attempt, seeded.attempt_id);
    let files = AttemptFiles::of(&harness, seeded);
    assert!(files.attempt.is_dir(), "acceptance created the attempt");
    assert!(
        !files.staging.exists(),
        "the claim commits before anything is staged"
    );
    assert!(!files.published.exists());

    // Dropping the app is the crash. No shutdown, no drain, no cleanup, which is
    // exactly what a killed process gets.
    drop(crashed);

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
    assert_eq!(
        harness.attempt_states(seeded.job_id).await,
        ["interrupted", "succeeded"]
    );
    assert!(
        files.attempt.is_dir(),
        "the interrupted attempt stays on disk for audit"
    );
    assert_artifacts_download(&restarted, seeded.job_id).await;
    assert!(!restarted.job_runner_failed());
}

#[tokio::test]
async fn crash_after_finalizing_discards_staging_and_retries() {
    let harness = TestHarness::new();
    let crashed = harness.app().await;
    let seeded = park_at(
        &crashed,
        FaultPoint::AfterFinalizing,
        "crash-after-finalizing",
    )
    .await;

    let (status, active_attempt) = harness.stored_status(seeded.job_id).await;
    assert_eq!(status, "finalizing");
    assert_eq!(active_attempt, seeded.attempt_id);
    assert_eq!(harness.artifact_row_count(seeded.attempt_id).await, 0);

    let files = AttemptFiles::of(&harness, seeded);
    assert!(files.staging.is_dir(), "the markdown is still only staged");
    assert!(files.staging.join("result.md").is_file());
    // The manifest is written after this commit, so staging holds the worker's
    // markdown alone. The engine already removed its report file.
    assert_eq!(fs::read_dir(&files.staging).unwrap().count(), 1);
    assert!(!files.published.exists(), "nothing is published yet");

    drop(crashed);

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
    assert_eq!(
        harness.attempt_states(seeded.job_id).await,
        ["interrupted", "succeeded"]
    );
    assert!(
        !files.staging.exists(),
        "recovery discards the abandoned staging"
    );
    assert!(
        !files.published.exists(),
        "the interrupted attempt never publishes"
    );
    assert_artifacts_download(&restarted, seeded.job_id).await;
    assert!(!restarted.job_runner_failed());
}

#[tokio::test]
async fn crash_after_publish_commits_the_same_attempt() {
    let harness = TestHarness::new();
    let crashed = harness.app().await;
    let seeded = park_at(&crashed, FaultPoint::AfterPublish, "crash-after-publish").await;

    assert_published_but_uncommitted(&harness, seeded).await;

    drop(crashed);

    let restarted = harness.app().await;
    let completed = restarted
        .wait_for_terminal(&seeded.job_id.to_string())
        .await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    assert_eq!(
        completed["data"]["activeAttemptId"],
        seeded.attempt_id.to_string(),
        "a durable publication is adopted, not converted again"
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert_eq!(harness.artifact_row_count(seeded.attempt_id).await, 2);
    assert_artifacts_download(&restarted, seeded.job_id).await;
    assert!(!restarted.job_runner_failed());
}

#[tokio::test]
async fn crash_before_success_commit_is_indistinguishable_from_after_publish() {
    let harness = TestHarness::new();
    let crashed = harness.app().await;
    let seeded = park_at(
        &crashed,
        FaultPoint::BeforeSuccessCommit,
        "crash-before-success-commit",
    )
    .await;

    // The same assertion the after-publish crash passes, and that is the point:
    // publication reaches disk before the success commit, so no crash window can
    // report success without downloadable artifacts. Carrying a job through that
    // window is the whole reason the `finalizing` state exists.
    assert_published_but_uncommitted(&harness, seeded).await;

    drop(crashed);

    let restarted = harness.app().await;
    let completed = restarted
        .wait_for_terminal(&seeded.job_id.to_string())
        .await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    assert_eq!(
        completed["data"]["activeAttemptId"],
        seeded.attempt_id.to_string()
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert_eq!(harness.artifact_row_count(seeded.attempt_id).await, 2);
    assert_artifacts_download(&restarted, seeded.job_id).await;
    assert!(!restarted.job_runner_failed());
}

#[tokio::test]
async fn queued_job_survives_restart_on_its_original_attempt() {
    let harness = TestHarness::new();
    let first = harness.app().await;
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    // Seeded with no service running, so the row is waiting for startup recovery
    // rather than for a notification.
    let job_id = harness
        .insert_queued_without_notification(&clean_pdf())
        .await;
    let (status, seeded_attempt) = harness.stored_status(job_id).await;
    assert_eq!(status, "queued");

    let restarted = harness.app().await;
    let completed = restarted.wait_for_terminal(&job_id.to_string()).await;
    assert_eq!(completed["data"]["status"], "succeeded", "{completed:#}");
    assert_eq!(
        completed["data"]["activeAttemptId"],
        seeded_attempt.to_string(),
        "queued work was never started, so recovery must not spend an attempt on it"
    );
    assert_eq!(harness.attempt_count(job_id).await, 1);
    assert_eq!(harness.attempt_states(job_id).await, ["succeeded"]);
    assert_artifacts_download(&restarted, job_id).await;
    assert!(!restarted.job_runner_failed());
}
