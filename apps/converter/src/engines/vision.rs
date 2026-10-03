//! Apple Vision engine: OCRs one image, or a scanned PDF page by page, through
//! the Swift worker.
//!
//! Isolated in a child process on the `pdf_inspector.rs` shape, for the same
//! reason: the decoders that read the bytes are not ours. The worker is macOS
//! only and ships nothing on any other host, so its absence is a normal state
//! the service runs without, never a startup error.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{fs, process::Command, sync::watch};

use super::child::{
    self, is_dotted_number, is_lowercase_sha256, wait_for_child, WorkerStartupError,
};
use super::{EngineAnalysis, EngineFailure, EngineOutcome, EngineRejection, QualitySignals};
use crate::{
    artifacts::{AttemptPaths, ValidatedOpenFile},
    persistence::DocumentClassification,
    vision_protocol::{
        VisionOutcome, VisionPages, VisionRejectionCode, VisionReport, VISION_ENGINE_NAME,
        VISION_MARKDOWN_FILE, VISION_WORKER_CUSTOM_WORDS_ENV,
        VISION_WORKER_EXPECTED_SOURCE_BYTES_ENV, VISION_WORKER_EXPECTED_SOURCE_SHA256_ENV,
        VISION_WORKER_IDENTITY_PREFIX, VISION_WORKER_LANGUAGE_CORRECTION_ENV,
        VISION_WORKER_MAX_OUTPUT_BYTES_ENV, VISION_WORKER_NATIVE_PAGES_ENV,
        VISION_WORKER_PROTOCOL_VERSION, VISION_WORKER_REPORT_FILE,
    },
    worker_protocol::{FallbackReason, NATIVE_PAGES_FILE},
};

const WORKER_LABEL: &str = "Vision";
const WORKER_IDENTITY_TIMEOUT: Duration = Duration::from_secs(10);
/// One runner serves every engine, so one scan may hold it 10 minutes at most.
const MAX_SCAN_TIMEOUT: Duration = Duration::from_secs(600);

/// Engine-specific detail persisted as attempt diagnostics and embedded in the
/// manifest. Content-free, and wall time is all of it. The page counts go to
/// the policy through [`analysis`], not here.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct VisionDiagnostics {
    pub processing_time_ms: u64,
}

#[derive(Clone, Debug)]
pub struct VisionEngine {
    worker_path: Arc<PathBuf>,
    /// The macOS product version the worker handshook with. Vision ships with
    /// the OS, so that is the engine's version and it is only knowable at run
    /// time, never as a compile-time pin.
    version: Arc<str>,
    timeout: Duration,
    max_output_bytes: u64,
    /// The PDF worker staged native pages, so Vision OCRs only the others.
    native_pages: bool,
}

impl VisionEngine {
    pub fn initialize(
        worker_path: PathBuf,
        timeout: Duration,
        max_output_bytes: u64,
    ) -> Result<Self, WorkerStartupError> {
        child::validate_worker(&worker_path, WORKER_LABEL)?;
        let version = verify_worker_identity(&worker_path)?;

        Ok(Self {
            worker_path: Arc::new(worker_path),
            version: version.into(),
            timeout,
            max_output_bytes,
            native_pages: false,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// The same engine with one more second per page, for a
    /// scanned PDF the worker reads a page at a time, up to
    /// [`MAX_SCAN_TIMEOUT`]. A scan past it times out into the inspector's
    /// needs_remote.
    pub fn with_page_budget(&self, pages: u32) -> Self {
        Self {
            timeout: (self.timeout + Duration::from_secs(pages.into()))
                .min(MAX_SCAN_TIMEOUT.max(self.timeout)),
            ..self.clone()
        }
    }

    /// The same engine, splicing its OCR into the native pages staged for it.
    pub fn with_native_pages(&self) -> Self {
        Self {
            native_pages: true,
            ..self.clone()
        }
    }

    pub async fn convert(
        &self,
        paths: &AttemptPaths,
        source: ValidatedOpenFile,
        cancellation: watch::Receiver<bool>,
        language_correction: bool,
        custom_words: &str,
    ) -> Result<EngineOutcome, EngineFailure> {
        if *cancellation.borrow() {
            return Err(EngineFailure::Interrupted);
        }
        let ValidatedOpenFile {
            file,
            byte_length,
            sha256,
        } = source;
        if byte_length == 0 || !is_lowercase_sha256(&sha256) {
            return Err(EngineFailure::Protocol);
        }
        let source = file.into_std().await;

        let started = Instant::now();
        let mut command = Command::new(self.worker_path.as_path());
        command
            .arg(&paths.publication_staging)
            .current_dir(&paths.attempt)
            .env_clear()
            .env(
                VISION_WORKER_MAX_OUTPUT_BYTES_ENV,
                self.max_output_bytes.to_string(),
            )
            .env(
                VISION_WORKER_EXPECTED_SOURCE_BYTES_ENV,
                byte_length.to_string(),
            )
            .env(VISION_WORKER_EXPECTED_SOURCE_SHA256_ENV, sha256)
            // Both settings are always sent, so the worker never falls back to
            // its own defaults on a request that asked for something else.
            .env(
                VISION_WORKER_LANGUAGE_CORRECTION_ENV,
                if language_correction { "1" } else { "0" },
            )
            .env(VISION_WORKER_CUSTOM_WORDS_ENV, custom_words)
            .stdin(Stdio::from(source))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if self.native_pages {
            command.env(VISION_WORKER_NATIVE_PAGES_ENV, NATIVE_PAGES_FILE);
        }
        let mut child = command.spawn().map_err(|_| EngineFailure::Unavailable)?;
        wait_for_child(&mut child, self.timeout, cancellation).await?;

        self.read_and_validate_report(paths, started.elapsed())
            .await
    }

    async fn read_and_validate_report(
        &self,
        paths: &AttemptPaths,
        elapsed: Duration,
    ) -> Result<EngineOutcome, EngineFailure> {
        let report_path = paths.publication_staging.join(VISION_WORKER_REPORT_FILE);
        let report: VisionReport = child::read_report(&report_path).await?;
        self.validate_identity(&report)?;

        let outcome = match report.outcome {
            VisionOutcome::Converted { artifact, pages } => {
                let digest = child::validate_staged_markdown(
                    paths,
                    VISION_MARKDOWN_FILE,
                    &artifact.relative_path,
                    artifact.byte_length,
                    &artifact.sha256,
                    self.max_output_bytes,
                )
                .await?;
                EngineOutcome::Converted {
                    analysis: analysis(elapsed, pages)?,
                    byte_length: artifact.byte_length,
                    sha256: digest,
                }
            }
            VisionOutcome::Rejected { code } => {
                child::reject_if_markdown_staged(paths).await?;
                match fallback_reason(code) {
                    Some(reason_code) => EngineOutcome::NeedsRemote {
                        analysis: analysis(elapsed, None)?,
                        reason_code,
                    },
                    None => EngineOutcome::Rejected {
                        rejection: rejection(code),
                    },
                }
            }
        };
        fs::remove_file(report_path)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        // Publication holds the Markdown and the manifest, nothing else.
        if self.native_pages {
            fs::remove_file(paths.publication_staging.join(NATIVE_PAGES_FILE))
                .await
                .map_err(|_| EngineFailure::Protocol)?;
        }
        Ok(outcome)
    }

    /// The report must name the same OS the handshake did. A worker that moved
    /// under us mid-run is not the one startup vetted.
    fn validate_identity(&self, report: &VisionReport) -> Result<(), EngineFailure> {
        if report.protocol_version != VISION_WORKER_PROTOCOL_VERSION
            || report.engine.name != VISION_ENGINE_NAME
            || report.engine.version != *self.version
            || !report.engine.features.is_empty()
        {
            return Err(EngineFailure::Protocol);
        }
        Ok(())
    }
}

/// Returns the macOS product version the worker reported. The handshake line
/// is a fixed prefix plus that version, so the check is prefix equality plus a
/// dotted-number tail rather than the byte equality a pinned engine allows.
fn verify_worker_identity(path: &Path) -> Result<String, WorkerStartupError> {
    let output = child::worker_identity_line(path, WORKER_LABEL, WORKER_IDENTITY_TIMEOUT)?;
    let mismatch = || WorkerStartupError::WorkerIdentityMismatch {
        worker: WORKER_LABEL,
        path: path.to_owned(),
    };
    let line = std::str::from_utf8(&output).map_err(|_| mismatch())?;
    let version = line
        .strip_prefix(VISION_WORKER_IDENTITY_PREFIX)
        .and_then(|tail| tail.strip_suffix('\n'))
        .ok_or_else(mismatch)?;
    if !is_dotted_number(version) {
        return Err(mismatch());
    }
    Ok(version.to_owned())
}

fn analysis(
    elapsed: Duration,
    pages: Option<VisionPages>,
) -> Result<EngineAnalysis, EngineFailure> {
    // The one thing Vision measures is which pages of a scan gave no text, so
    // a partly unreadable scan publishes with `pages_without_extractable_text`
    // rather than as a clean success. A single image has no pages. Table
    // detection is never claimed: the spike caught it wrong in both
    // directions, tables reported over plain paragraphs and real grids missed.
    let native_text_ratio = match pages {
        None => None,
        // Every page empty is `no_text_found`, so a converted report saying so
        // is not one this worker writes.
        Some(VisionPages {
            total,
            without_text,
        }) if without_text < total => Some((total - without_text) as f32 / total as f32),
        Some(_) => return Err(EngineFailure::Protocol),
    };
    Ok(EngineAnalysis {
        classification: DocumentClassification::ImageBased,
        quality: QualitySignals {
            native_text_ratio,
            ..QualitySignals::unmeasured()
        },
        diagnostics: serde_json::to_value(VisionDiagnostics {
            processing_time_ms: u64::try_from(elapsed.as_millis())
                .map_err(|_| EngineFailure::Protocol)?,
        })
        .map_err(|_| EngineFailure::Protocol)?,
    })
}

/// The rejections a remote engine could still convert, and the reason the job
/// carries into `needs_remote`. Everything this engine gives up on is a
/// statement about this engine, not about the file: no text recognized, a
/// frame count it cannot carry whole, output past the ceiling. Each of those
/// spends the remote fallback the standard profile promises. Only
/// `InvalidImage` is terminal, because admission already matched the container
/// signature, so bytes no decoder reads here are bytes no engine reads.
///
/// Multi-frame is the one that has to fall back rather than fail. Images left
/// `PERMANENT_DIRECT_FORMATS` in this same change, so a multi-page TIFF that
/// used to go straight to the remote provider now arrives here instead, and
/// answering it with a terminal failure would drop a file that converted fine
/// the day before.
fn fallback_reason(code: VisionRejectionCode) -> Option<FallbackReason> {
    match code {
        VisionRejectionCode::NoTextFound | VisionRejectionCode::MultiFrameImage => {
            Some(FallbackReason::LocalQualityFailed)
        }
        VisionRejectionCode::OutputTooLarge => Some(FallbackReason::OutputTooLarge),
        VisionRejectionCode::InvalidImage => None,
    }
}

/// Maps the worker's rejection wire codes to the engine-owned rejection.
/// The strings are the public failure code and message; do not reword them
/// without a contract review.
fn rejection(code: VisionRejectionCode) -> EngineRejection {
    match code {
        VisionRejectionCode::InvalidImage => EngineRejection {
            code: "invalid_image",
            message: "The uploaded file is not a readable image.",
        },
        VisionRejectionCode::MultiFrameImage => EngineRejection {
            code: "multi_frame_image",
            message: "Only single-frame images can be converted.",
        },
        VisionRejectionCode::NoTextFound => EngineRejection {
            code: "no_text_found",
            message: "No text was recognized in the image.",
        },
        VisionRejectionCode::OutputTooLarge => EngineRejection {
            code: "output_too_large",
            message: "The recognized text exceeds the conversion limits.",
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use sha2::{Digest, Sha256};
    use tokio::sync::watch;

    use super::{
        verify_worker_identity, EngineOutcome, FallbackReason, VisionEngine, WorkerStartupError,
    };
    use crate::artifacts::{AttemptPaths, ValidatedOpenFile};
    use crate::persistence::DocumentClassification;

    #[cfg(unix)]
    fn write_worker(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    fn worker_script(body: &str) -> String {
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'tool-kit-vision-worker protocol=1 apple-vision=26.6.1\\n'\n  exit 0\nfi\n{body}\n"
        )
    }

    async fn source(path: &Path, bytes: &[u8]) -> ValidatedOpenFile {
        tokio::fs::write(path, bytes).await.unwrap();
        ValidatedOpenFile {
            file: tokio::fs::File::open(path).await.unwrap(),
            byte_length: u64::try_from(bytes.len()).unwrap(),
            sha256: hex::encode(Sha256::digest(bytes)),
        }
    }

    fn paths(root: &Path) -> AttemptPaths {
        let attempt = root.join("attempt");
        let publication_staging = attempt.join("publication.staging");
        std::fs::create_dir_all(&publication_staging).unwrap();
        AttemptPaths {
            source: root.join("input"),
            published: attempt.join("artifacts"),
            publication_staging,
            attempt,
        }
    }

    #[cfg(unix)]
    #[test]
    fn startup_rejects_a_worker_with_the_wrong_identity() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("wrong-worker");
        std::fs::write(&worker, "#!/bin/sh\necho wrong-worker\n").unwrap();
        std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(matches!(
            verify_worker_identity(&worker),
            Err(WorkerStartupError::WorkerIdentityMismatch { .. })
        ));
    }

    /// A second a page, so a 100 000-page scan held the only runner for a day.
    #[cfg(unix)]
    #[test]
    fn the_scan_budget_stops_at_ten_minutes() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(&worker, &worker_script("exit 0"));
        let engine = VisionEngine::initialize(worker, Duration::from_secs(60), 1024).unwrap();

        assert_eq!(engine.with_page_budget(30).timeout, Duration::from_secs(90));
        assert_eq!(
            engine.with_page_budget(100_000).timeout,
            Duration::from_secs(600)
        );
    }

    /// The handshake carries the running OS version, so it cannot be compared
    /// byte for byte. It still has to be a version.
    #[cfg(unix)]
    #[test]
    fn startup_rejects_a_handshake_whose_version_is_not_one() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            "#!/bin/sh\nprintf 'tool-kit-vision-worker protocol=1 apple-vision=whenever\\n'\n",
        );

        assert!(matches!(
            verify_worker_identity(&worker),
            Err(WorkerStartupError::WorkerIdentityMismatch { .. })
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_converted_report_is_validated_against_the_staged_markdown() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &worker_script(
                "set -e\nstaging=\"$1\"\nprintf '# Quarterly Report\\n' > \"$staging/result.md\"\n\
                 sha=$(/usr/bin/shasum -a 256 \"$staging/result.md\" | cut -d' ' -f1)\n\
                 bytes=$(/usr/bin/wc -c < \"$staging/result.md\" | tr -d ' ')\n\
                 printf '{\"protocolVersion\":1,\"engine\":{\"name\":\"apple-vision\",\"version\":\"26.6.1\",\"features\":[]},\"outcome\":{\"kind\":\"converted\",\"artifact\":{\"relativePath\":\"result.md\",\"byteLength\":%s,\"sha256\":\"%s\"}}}' \"$bytes\" \"$sha\" > \"$staging/worker-report.json\"\n",
            ),
        );
        let paths = paths(directory.path());
        let source = source(&paths.source, b"image bytes").await;

        let engine = VisionEngine::initialize(worker, Duration::from_secs(5), 1024).unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let outcome = engine
            .convert(&paths, source, cancellation, true, "")
            .await
            .unwrap();

        let EngineOutcome::Converted { analysis, .. } = outcome else {
            panic!("a converted report must convert");
        };
        assert_eq!(analysis.classification, DocumentClassification::ImageBased);
        assert!(analysis.quality.native_text_ratio.is_none());
        assert!(!paths
            .publication_staging
            .join("worker-report.json")
            .exists());
    }

    /// A scan Vision read only part of must reach the policy as a scan with
    /// textless pages, or it publishes as a clean success.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_scan_with_textless_pages_reports_them_to_the_policy() {
        for (total, without_text, warns) in [(3, 1, Some(true)), (2, 0, Some(false)), (2, 2, None)]
        {
            let directory = tempfile::tempdir().unwrap();
            let worker = directory.path().join("worker");
            write_worker(
                &worker,
                &worker_script(&format!(
                    "set -e\nstaging=\"$1\"\nprintf 'Page one\\n' > \"$staging/result.md\"\n\
                     sha=$(/usr/bin/shasum -a 256 \"$staging/result.md\" | cut -d' ' -f1)\n\
                     bytes=$(/usr/bin/wc -c < \"$staging/result.md\" | tr -d ' ')\n\
                     printf '{{\"protocolVersion\":1,\"engine\":{{\"name\":\"apple-vision\",\"version\":\"26.6.1\",\"features\":[]}},\"outcome\":{{\"kind\":\"converted\",\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":%s,\"sha256\":\"%s\"}},\"pages\":{{\"total\":{total},\"withoutText\":{without_text}}}}}}}' \"$bytes\" \"$sha\" > \"$staging/worker-report.json\"\n",
                )),
            );
            let paths = paths(directory.path());
            let source = source(&paths.source, b"%PDF-1.7 scan").await;

            let engine = VisionEngine::initialize(worker, Duration::from_secs(5), 1024).unwrap();
            let (_cancel, cancellation) = watch::channel(false);
            let result = engine.convert(&paths, source, cancellation, true, "").await;

            match (result, warns) {
                (Ok(EngineOutcome::Converted { analysis, .. }), Some(warns)) => {
                    assert_eq!(analysis.quality.has_pages_without_text(), warns);
                }
                // Every page empty is `no_text_found`, never a conversion.
                (Err(failure), None) => assert_eq!(failure, super::EngineFailure::Protocol),
                _ => panic!("{without_text} of {total} textless pages routed the wrong way"),
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rejection_that_staged_markdown_anyway_is_a_protocol_error() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &worker_script(
                "staging=\"$1\"\nprintf 'leftover' > \"$staging/result.md\"\n\
                 printf '{\"protocolVersion\":1,\"engine\":{\"name\":\"apple-vision\",\"version\":\"26.6.1\",\"features\":[]},\"outcome\":{\"kind\":\"rejected\",\"code\":\"no_text_found\"}}' > \"$staging/worker-report.json\"\n",
            ),
        );
        let paths = paths(directory.path());
        let source = source(&paths.source, b"image bytes").await;

        let engine = VisionEngine::initialize(worker, Duration::from_secs(5), 1024).unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let result = engine.convert(&paths, source, cancellation, true, "").await;

        assert_eq!(result.unwrap_err(), super::EngineFailure::Protocol);
    }

    /// Vision recognizing no text is a statement about Vision, so the job has
    /// to reach the remote leg. Ending it here is what took the Datalab
    /// fallback away from every image the desktop stopped routing direct.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_recoverable_rejection_needs_remote_and_an_unreadable_one_is_terminal() {
        for (code, fallback) in [
            ("no_text_found", Some(FallbackReason::LocalQualityFailed)),
            ("output_too_large", Some(FallbackReason::OutputTooLarge)),
            // A frame count this engine cannot carry is still a file a remote
            // engine converts, and one it converted before images left
            // `PERMANENT_DIRECT_FORMATS`. Only unreadable bytes are terminal.
            (
                "multi_frame_image",
                Some(FallbackReason::LocalQualityFailed),
            ),
            ("invalid_image", None),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let worker = directory.path().join("worker");
            let report = format!(
                "{{\"protocolVersion\":1,\"engine\":{{\"name\":\"apple-vision\",\"version\":\"26.6.1\",\"features\":[]}},\"outcome\":{{\"kind\":\"rejected\",\"code\":\"{code}\"}}}}"
            );
            write_worker(
                &worker,
                &worker_script(&format!(
                    "printf '%s' '{report}' > \"$1/worker-report.json\"\n"
                )),
            );
            let paths = paths(directory.path());
            let source = source(&paths.source, b"image bytes").await;

            let engine = VisionEngine::initialize(worker, Duration::from_secs(5), 1024).unwrap();
            let (_cancel, cancellation) = watch::channel(false);
            let outcome = engine
                .convert(&paths, source, cancellation, true, "")
                .await
                .unwrap();

            match (outcome, fallback) {
                (EngineOutcome::NeedsRemote { reason_code, .. }, Some(reason)) => {
                    assert_eq!(reason_code, reason);
                }
                (EngineOutcome::Rejected { rejection }, None) => {
                    assert_eq!(rejection.code, code);
                }
                _ => panic!("{code} routed the wrong way"),
            }
        }
    }
}
