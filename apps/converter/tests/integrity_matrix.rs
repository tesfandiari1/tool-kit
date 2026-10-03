#![allow(clippy::unwrap_used)]

mod support;

use std::{fs, path::Path, time::Duration};

use axum::http::StatusCode;

use support::{clean_pdf, count_named_files, json_body, TestHarness};

/// The five ways stored bytes stop matching what the database recorded. Each one
/// trips a different guard, and an equal-length rewrite is the only one a byte
/// count cannot catch, so dropping a case leaves that guard unproven.
#[derive(Clone, Copy, Debug)]
enum Corruption {
    Missing,
    Truncated,
    RewrittenAtEqualLength,
    NonRegular,
    Symlink,
}

impl Corruption {
    /// `decoy` takes a byte-perfect copy of the original for the symlink case, so
    /// that case proves containment rather than content.
    fn apply(self, path: &Path, decoy: &Path) {
        let original = fs::read(path).unwrap();
        assert!(original.len() > 1, "the fixture is too short to truncate");
        fs::remove_file(path).unwrap();
        match self {
            Self::Missing => {}
            Self::Truncated => fs::write(path, &original[..original.len() / 2]).unwrap(),
            Self::RewrittenAtEqualLength => {
                let mut rewritten = original.clone();
                *rewritten.last_mut().unwrap() ^= 0xff;
                fs::write(path, &rewritten).unwrap();
            }
            Self::NonRegular => fs::create_dir(path).unwrap(),
            Self::Symlink => {
                fs::write(decoy, &original).unwrap();
                #[cfg(unix)]
                std::os::unix::fs::symlink(decoy, path).unwrap();
                #[cfg(not(unix))]
                unreachable!("the symlink cases only run on unix");
            }
        }
    }
}

/// Corrupts the immutable source of a queued job, then lets a fresh app claim it.
/// The claim path owns this guard because startup recovery hands a queued job
/// straight to the runner without reading its bytes.
async fn corrupt_source_fails_the_claim(corruption: Corruption) {
    let harness = TestHarness::with_poll_interval(Duration::from_millis(50));
    harness.initialize_empty().await;
    let seeded = harness.insert_queued_job(&clean_pdf()).await;
    let source = harness
        .data_dir()
        .join("jobs")
        .join(seeded.job_id.to_string())
        .join("source/input");
    // The decoy sits outside the data root so it cannot be mistaken for owned storage.
    let decoy = harness.data_dir().parent().unwrap().join("source-decoy");
    corruption.apply(&source, &decoy);

    let app = harness.app().await;
    let status = app.wait_for_terminal(&seeded.job_id.to_string()).await;
    assert_eq!(status["data"]["status"], "failed", "{status:#}");
    assert_eq!(
        status["data"]["failure"]["code"], "source_integrity_failed",
        "{status:#}"
    );
    assert_eq!(
        status["data"]["activeAttemptId"],
        seeded.attempt_id.to_string()
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert_eq!(harness.artifact_row_count(seeded.attempt_id).await, 0);
    assert_eq!(count_named_files(harness.data_dir(), "result.md"), 0);
    app.shutdown(Duration::from_secs(1)).await;
}

/// Corrupts a published bundle while the service is stopped, then restarts. A
/// paid-for result that no longer matches its recorded bytes must read as failed
/// and stay on disk, because the rows are the only audit trail left.
async fn corrupt_published_markdown_fails_the_restart(
    corruption: Corruption,
    idempotency_key: &str,
) {
    let harness = TestHarness::new();
    let first = harness.app().await;
    let seeded = first.submit_succeeded_job(idempotency_key).await;
    first.shutdown(Duration::from_secs(1)).await;
    drop(first);

    let published = harness
        .data_dir()
        .join("jobs")
        .join(seeded.job_id.to_string())
        .join("attempts")
        .join(seeded.attempt_id.to_string())
        .join("artifacts");
    let markdown = published.join("result.md");
    // Keeping the decoy out of the published directory leaves the symlink case as
    // the only reason the bundle fails, not an unexpected extra entry.
    let decoy = harness.data_dir().parent().unwrap().join("artifact-decoy");
    corruption.apply(&markdown, &decoy);

    let restarted = harness.app().await;
    let status = restarted
        .authorized_get(&format!("/api/v1/conversions/{}", seeded.job_id))
        .await;
    assert_eq!(status.status(), StatusCode::OK);
    let status = json_body(status).await;
    assert_eq!(status["data"]["status"], "failed", "{status:#}");
    assert_eq!(
        status["data"]["failure"]["code"], "artifact_integrity_failed",
        "{status:#}"
    );
    assert_eq!(harness.attempt_count(seeded.job_id).await, 1);
    assert_eq!(harness.artifact_row_count(seeded.attempt_id).await, 1);
    let download = restarted
        .authorized_get(&format!(
            "/api/v1/conversions/{}/artifacts/markdown",
            seeded.job_id
        ))
        .await;
    assert_eq!(download.status(), StatusCode::NOT_FOUND);
    restarted.shutdown(Duration::from_secs(1)).await;
}

#[tokio::test]
async fn missing_source_fails_the_claimed_job() {
    corrupt_source_fails_the_claim(Corruption::Missing).await;
}

#[tokio::test]
async fn truncated_source_fails_the_claimed_job() {
    corrupt_source_fails_the_claim(Corruption::Truncated).await;
}

#[tokio::test]
async fn rewritten_source_of_equal_length_fails_the_claimed_job() {
    corrupt_source_fails_the_claim(Corruption::RewrittenAtEqualLength).await;
}

#[tokio::test]
async fn directory_in_place_of_the_source_fails_the_claimed_job() {
    corrupt_source_fails_the_claim(Corruption::NonRegular).await;
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_source_with_correct_bytes_fails_the_claimed_job() {
    corrupt_source_fails_the_claim(Corruption::Symlink).await;
}

#[tokio::test]
async fn missing_published_markdown_fails_the_succeeded_job() {
    corrupt_published_markdown_fails_the_restart(Corruption::Missing, "artifact-matrix-missing")
        .await;
}

#[tokio::test]
async fn truncated_published_markdown_fails_the_succeeded_job() {
    corrupt_published_markdown_fails_the_restart(
        Corruption::Truncated,
        "artifact-matrix-truncated",
    )
    .await;
}

#[tokio::test]
async fn rewritten_published_markdown_of_equal_length_fails_the_succeeded_job() {
    corrupt_published_markdown_fails_the_restart(
        Corruption::RewrittenAtEqualLength,
        "artifact-matrix-rewritten",
    )
    .await;
}

#[tokio::test]
async fn directory_in_place_of_the_published_markdown_fails_the_succeeded_job() {
    corrupt_published_markdown_fails_the_restart(
        Corruption::NonRegular,
        "artifact-matrix-non-regular",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_published_markdown_with_correct_bytes_fails_the_succeeded_job() {
    corrupt_published_markdown_fails_the_restart(Corruption::Symlink, "artifact-matrix-symlink")
        .await;
}
