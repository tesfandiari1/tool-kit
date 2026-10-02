#![allow(clippy::unwrap_used)]

mod support;

use std::{fs, path::PathBuf, time::Duration};

use axum::http::StatusCode;
use http_body_util::BodyExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use tool_kit_converter::faults::FaultPoint;

use support::{clean_pdf, json_body, SeededJob, TestApp, TestHarness};

/// A parked worker never resumes, so an unreached barrier would hang the test
/// run forever without a ceiling on the wait.
const BARRIER_TIMEOUT: Duration = Duration::from_secs(30);

/// Submits one clean PDF and returns once the worker has parked on `point`.
/// The barrier is armed before the submit, because a worker that reaches the
/// point first runs straight past it.
async fn park_at(app: &TestApp, point: FaultPoint, idempotency_key: &str) -> SeededJob {
    let barrier = app.fault_barrier();
    barrier.arm(point);
    let seeded = app.submit_clean(idempotency_key).await;

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
    let job_id = harness.insert_queued_job(&clean_pdf()).await.job_id;
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

/// One unreadable row used to abort startup for the whole service, which
/// `restart: unless-stopped` turns into a crash loop.
#[tokio::test]
async fn an_unreadable_conversion_is_quarantined_instead_of_stopping_the_boot() {
    let harness = TestHarness::new();
    let app = harness.app().await;
    let healthy = app.submit_succeeded_job("quarantine-healthy").await;
    let poisoned = app.submit_succeeded_job("quarantine-poisoned").await;
    app.shutdown(Duration::from_secs(5)).await;
    drop(app);

    harness.clear_stored_classification(poisoned).await;

    // This call used to panic: initialize returned the bad row's error.
    let restarted = harness.app().await;

    let quarantined = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", poisoned.job_id))
        .await;
    assert_eq!(quarantined.status(), StatusCode::OK);
    let quarantined = json_body(quarantined).await;
    assert_eq!(quarantined["data"]["status"], "failed", "{quarantined:#}");
    assert_eq!(
        quarantined["data"]["failure"]["code"], "recovery_state_unrecoverable",
        "{quarantined:#}"
    );

    // Audit rows survive, and the healthy neighbour is untouched.
    assert_eq!(harness.artifact_row_count(poisoned.attempt_id).await, 2);
    let survivor = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", healthy.job_id))
        .await;
    assert_eq!(json_body(survivor).await["data"]["status"], "succeeded");
    assert_artifacts_download(&restarted, healthy.job_id).await;
    assert!(!restarted.job_runner_failed());

    // The quarantined row is terminal, so a second boot never revisits it.
    restarted.shutdown(Duration::from_secs(5)).await;
    drop(restarted);
    let third = harness.app().await;
    assert!(!third.job_runner_failed());
    assert_artifacts_download(&third, healthy.job_id).await;
}

/// The other half of the same class. A row that will not decode at all used to
/// abort `list_recovery_candidates` before any per-job containment ran, so
/// containing reconciliation errors alone did not stop the boot loop.
#[tokio::test]
async fn a_row_that_cannot_be_decoded_is_quarantined_instead_of_stopping_the_boot() {
    let harness = TestHarness::new();
    let app = harness.app().await;
    let healthy = app.submit_succeeded_job("decode-healthy").await;
    let corrupt = app.submit_succeeded_job("decode-corrupt").await;
    app.shutdown(Duration::from_secs(5)).await;
    drop(app);

    harness.corrupt_stored_reason_codes(corrupt).await;

    // Used to panic here: the listing decoded rows with `?`, so this row took
    // the whole boot down before any per-job containment ran.
    let restarted = harness.app().await;

    // The row is terminal, so nothing will claim it or revisit it. Read it from
    // SQL, because the row is genuinely unreadable: its own GET answers with a
    // bounded error rather than a status, which is the truth about it.
    let (status, _) = harness.stored_status(corrupt.job_id).await;
    assert_eq!(status, "failed");
    let unreadable = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", corrupt.job_id))
        .await;
    assert_eq!(unreadable.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let survivor = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", healthy.job_id))
        .await;
    assert_eq!(json_body(survivor).await["data"]["status"], "succeeded");
    assert_artifacts_download(&restarted, healthy.job_id).await;
    assert!(!restarted.job_runner_failed());

    // And the next boot is clean too, rather than re-quarantining forever.
    restarted.shutdown(Duration::from_secs(5)).await;
    drop(restarted);
    let third = harness.app().await;
    assert!(!third.job_runner_failed());
    assert_artifacts_download(&third, healthy.job_id).await;
}

/// Orphan storage is the same class. One job directory that could not move to
/// quarantine used to abort startup the way an unreadable row did.
#[cfg(unix)]
#[tokio::test]
async fn an_orphan_that_cannot_be_quarantined_does_not_stop_the_boot() {
    use std::os::unix::fs::PermissionsExt;

    let harness = TestHarness::new();
    let first = harness.app().await;
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    // Moving a directory to a new parent rewrites its `..` entry, so a
    // read-only directory cannot be renamed into quarantine.
    let orphan = harness
        .data_dir()
        .join("jobs")
        .join(Uuid::new_v4().to_string());
    fs::create_dir(&orphan).unwrap();
    fs::set_permissions(&orphan, fs::Permissions::from_mode(0o500)).unwrap();

    let restarted = harness.app().await;
    assert!(orphan.is_dir(), "the orphan stays where it was");
    assert!(!restarted.job_runner_failed());

    fs::set_permissions(&orphan, fs::Permissions::from_mode(0o700)).unwrap();
}
