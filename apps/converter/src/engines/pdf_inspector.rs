use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use thiserror::Error;
use tokio::{
    fs,
    process::Command,
    sync::{watch, OwnedSemaphorePermit, Semaphore},
};

use super::child::{self, is_lowercase_sha256, wait_for_child, WorkerStartupError};
use super::{EngineAnalysis, EngineFailure, EngineOutcome, EngineRejection, QualitySignals};
use crate::{
    artifacts::{AttemptPaths, ValidatedOpenFile},
    persistence::DocumentClassification,
    worker_protocol::{
        FallbackReason, Inspection, PdfTypeLabel, RejectionCode, WorkerOutcome, WorkerReport,
        MARKDOWN_FILE, PDF_INSPECTOR_VERSION, WORKER_EXPECTED_SOURCE_BYTES_ENV,
        WORKER_EXPECTED_SOURCE_SHA256_ENV, WORKER_MAX_OUTPUT_BYTES_ENV, WORKER_PROTOCOL_VERSION,
        WORKER_REPORT_FILE,
    },
};

const WORKER_LABEL: &str = "PDF";
// Ten seconds like the Swift workers: at two, a fork storm from the parallel
// test suite on a loaded Mac timed out a script that only echoes.
const WORKER_IDENTITY_TIMEOUT: Duration = Duration::from_secs(10);
const EXPECTED_WORKER_IDENTITY: &str = "tool-kit-pdf-worker protocol=2 pdf-inspector=1.25.2\n";
const REQUIRED_CMAPS: [&str; 4] = [
    "Adobe-CNS1-UCS2.bcmap",
    "Adobe-GB1-UCS2.bcmap",
    "Adobe-Japan1-UCS2.bcmap",
    "Adobe-Korea1-UCS2.bcmap",
];

#[derive(Clone, Debug)]
pub struct PdfInspectorEngine {
    worker_path: Arc<PathBuf>,
    bcmaps_dir: Option<Arc<PathBuf>>,
    timeout: Duration,
    max_output_bytes: u64,
    rayon_threads: usize,
    permits: Arc<Semaphore>,
}

impl PdfInspectorEngine {
    pub fn initialize(
        worker_path: PathBuf,
        bcmaps_dir: Option<PathBuf>,
        timeout: Duration,
        max_output_bytes: u64,
        rayon_threads: usize,
    ) -> Result<Self, EngineStartupError> {
        child::validate_worker(&worker_path, WORKER_LABEL)?;
        verify_worker_identity(&worker_path)?;
        if let Some(path) = bcmaps_dir.as_deref() {
            let metadata =
                std::fs::metadata(path).map_err(|source| EngineStartupError::InvalidCmaps {
                    path: path.to_owned(),
                    source,
                })?;
            if !metadata.is_dir() {
                return Err(EngineStartupError::CmapsNotDirectory(path.to_owned()));
            }
            validate_cmaps(path)?;
        }

        Ok(Self {
            worker_path: Arc::new(worker_path),
            bcmaps_dir: bcmaps_dir.map(Arc::new),
            timeout,
            max_output_bytes,
            rayon_threads,
            permits: Arc::new(Semaphore::new(1)),
        })
    }

    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, EngineFailure> {
        child::acquire(&self.permits).await
    }

    pub async fn convert(
        &self,
        paths: &AttemptPaths,
        source: ValidatedOpenFile,
        _permit: OwnedSemaphorePermit,
        cancellation: watch::Receiver<bool>,
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

        let mut command = Command::new(self.worker_path.as_path());
        command
            .arg(&paths.publication_staging)
            .current_dir(&paths.attempt)
            .env_clear()
            .env("RAYON_NUM_THREADS", self.rayon_threads.to_string())
            .env(
                WORKER_MAX_OUTPUT_BYTES_ENV,
                self.max_output_bytes.to_string(),
            )
            .env(WORKER_EXPECTED_SOURCE_BYTES_ENV, byte_length.to_string())
            .env(WORKER_EXPECTED_SOURCE_SHA256_ENV, sha256)
            .stdin(Stdio::from(source))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if let Some(path) = self.bcmaps_dir.as_deref() {
            command.env("PDF_INSPECTOR_BCMAPS_DIR", path.as_path());
        }

        let mut child = command.spawn().map_err(|_| EngineFailure::Unavailable)?;
        wait_for_child(&mut child, self.timeout, cancellation).await?;

        self.read_and_validate_report(paths).await
    }

    async fn read_and_validate_report(
        &self,
        paths: &AttemptPaths,
    ) -> Result<EngineOutcome, EngineFailure> {
        let report_path = paths.publication_staging.join(WORKER_REPORT_FILE);
        let report: WorkerReport = child::read_report(&report_path).await?;
        validate_identity(&report)?;

        let outcome = match report.outcome {
            WorkerOutcome::Converted {
                inspection,
                artifact,
            } => {
                validate_complete_inspection(&inspection)?;
                let digest = child::validate_staged_markdown(
                    paths,
                    MARKDOWN_FILE,
                    &artifact.relative_path,
                    artifact.byte_length,
                    &artifact.sha256,
                    self.max_output_bytes,
                )
                .await?;
                EngineOutcome::Converted {
                    analysis: into_analysis(&inspection)?,
                    byte_length: artifact.byte_length,
                    sha256: digest,
                }
            }
            WorkerOutcome::NeedsRemote {
                inspection,
                reason_code,
            } => {
                validate_needs_remote(&inspection, reason_code)?;
                child::reject_if_markdown_staged(paths).await?;
                EngineOutcome::NeedsRemote {
                    analysis: into_analysis(&inspection)?,
                    reason_code,
                }
            }
            WorkerOutcome::Rejected { code } => {
                child::reject_if_markdown_staged(paths).await?;
                EngineOutcome::Rejected {
                    rejection: rejection(code),
                }
            }
        };
        fs::remove_file(report_path)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        Ok(outcome)
    }
}

fn verify_worker_identity(path: &Path) -> Result<(), WorkerStartupError> {
    let output = child::worker_identity_line(path, WORKER_LABEL, WORKER_IDENTITY_TIMEOUT)?;
    if output.as_slice() != EXPECTED_WORKER_IDENTITY.as_bytes() {
        return Err(WorkerStartupError::WorkerIdentityMismatch {
            worker: WORKER_LABEL,
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn validate_cmaps(path: &Path) -> Result<(), EngineStartupError> {
    for name in REQUIRED_CMAPS {
        let sentinel = path.join(name);
        let metadata = std::fs::metadata(&sentinel).map_err(|source| {
            EngineStartupError::InvalidCmapSentinel {
                path: sentinel.clone(),
                source,
            }
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(EngineStartupError::InvalidCmapSentinel {
                path: sentinel,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "CMap sentinel must be a non-empty regular file",
                ),
            });
        }
    }
    Ok(())
}

fn validate_identity(report: &WorkerReport) -> Result<(), EngineFailure> {
    if report.protocol_version != WORKER_PROTOCOL_VERSION
        || report.engine.name != "pdf-inspector"
        || report.engine.version != PDF_INSPECTOR_VERSION
        || !report.engine.features.is_empty()
    {
        return Err(EngineFailure::Protocol);
    }
    Ok(())
}

fn validate_inspection(inspection: &Inspection) -> Result<(), EngineFailure> {
    if !inspection.confidence.is_finite()
        || !(0.0..=1.0).contains(&inspection.confidence)
        || inspection.page_count == 0
        || inspection
            .pages_needing_ocr
            .iter()
            .any(|page| *page == 0 || *page > inspection.page_count)
        || inspection
            .pages_with_tables
            .iter()
            .chain(&inspection.pages_with_columns)
            .any(|page| *page == 0 || *page > inspection.page_count)
        || inspection
            .ocr_reasons_by_page
            .iter()
            .any(|entry| entry.page == 0 || entry.page > inspection.page_count)
    {
        return Err(EngineFailure::Protocol);
    }
    Ok(())
}

fn validate_needs_remote(
    inspection: &Inspection,
    reason: FallbackReason,
) -> Result<(), EngineFailure> {
    validate_inspection(inspection)?;
    let coherent = match reason {
        FallbackReason::ScannedPdf => inspection.pdf_type == PdfTypeLabel::Scanned,
        FallbackReason::ImageBasedPdf => inspection.pdf_type == PdfTypeLabel::ImageBased,
        FallbackReason::MixedPdf => inspection.pdf_type == PdfTypeLabel::Mixed,
        FallbackReason::GarbledText => {
            inspection.pdf_type == PdfTypeLabel::TextBased && inspection.has_encoding_issues
        }
        FallbackReason::OcrRequired => {
            inspection.pdf_type == PdfTypeLabel::TextBased
                && !inspection.pages_needing_ocr.is_empty()
        }
        FallbackReason::LocalQualityFailed | FallbackReason::OutputTooLarge => {
            inspection.pdf_type == PdfTypeLabel::TextBased
                && !inspection.has_encoding_issues
                && inspection.pages_needing_ocr.is_empty()
        }
    };
    if coherent {
        Ok(())
    } else {
        Err(EngineFailure::Protocol)
    }
}

fn validate_complete_inspection(inspection: &Inspection) -> Result<(), EngineFailure> {
    validate_inspection(inspection)?;
    if inspection.pdf_type != PdfTypeLabel::TextBased
        || inspection.has_encoding_issues
        || !inspection.pages_needing_ocr.is_empty()
    {
        return Err(EngineFailure::Protocol);
    }
    Ok(())
}

/// Maps the worker's rejection wire codes to the engine-owned rejection.
/// The strings are the public failure code and message; do not reword them
/// without a contract review.
fn rejection(code: RejectionCode) -> EngineRejection {
    match code {
        RejectionCode::EncryptedPdf => EngineRejection {
            code: code.as_str(),
            message: "Encrypted PDFs are not accepted.",
        },
        RejectionCode::InvalidPdf => EngineRejection {
            code: code.as_str(),
            message: "The uploaded file is not a valid PDF.",
        },
        RejectionCode::InvalidPdfStructure => EngineRejection {
            code: code.as_str(),
            message: "The PDF structure is invalid.",
        },
    }
}

/// Maps the worker's PDF inspection into the engine-neutral analysis. The
/// serialized diagnostics are byte-identical to the previous `Inspection`
/// JSON, so manifests and attempt rows written before this change still
/// validate.
fn into_analysis(inspection: &Inspection) -> Result<EngineAnalysis, EngineFailure> {
    let quality = QualitySignals {
        // pdf-inspector's confidence on a text-based document is exactly the
        // share of pages it read as native text: 1.0 for an all-native file at
        // any page count, 0.9 for nine native pages and one scan. Only claim it
        // as a completeness measure for the classification where that holds.
        native_text_ratio: match inspection.pdf_type {
            PdfTypeLabel::TextBased => Some(inspection.confidence),
            _ => None,
        },
        has_tables: !inspection.pages_with_tables.is_empty(),
        has_columns: !inspection.pages_with_columns.is_empty(),
    };
    Ok(EngineAnalysis {
        classification: match inspection.pdf_type {
            PdfTypeLabel::TextBased => DocumentClassification::TextBased,
            PdfTypeLabel::Scanned => DocumentClassification::Scanned,
            PdfTypeLabel::ImageBased => DocumentClassification::ImageBased,
            PdfTypeLabel::Mixed => DocumentClassification::Mixed,
        },
        quality,
        diagnostics: serde_json::to_value(inspection).map_err(|_| EngineFailure::Protocol)?,
    })
}

/// The completeness gate for a locally converted PDF: text-based, at least
/// one page, no OCR-needing pages, and no encoding damage.
pub(crate) fn is_complete_native_inspection(inspection: &Inspection) -> bool {
    inspection.pdf_type == PdfTypeLabel::TextBased
        && inspection.page_count > 0
        && inspection.pages_needing_ocr.is_empty()
        && !inspection.has_encoding_issues
}

#[derive(Debug, Error)]
pub enum EngineStartupError {
    #[error(transparent)]
    Startup(#[from] WorkerStartupError),
    #[error("PDF CMap directory is unavailable at {path:?}")]
    InvalidCmaps {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("PDF CMap path is not a directory: {0:?}")]
    CmapsNotDirectory(PathBuf),
    #[error("PDF CMap runtime asset is unavailable at {path:?}")]
    InvalidCmapSentinel {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use sha2::{Digest, Sha256};
    use tokio::sync::watch;

    use super::{
        is_complete_native_inspection, validate_cmaps, verify_worker_identity, EngineFailure,
        EngineStartupError, PdfInspectorEngine, WorkerStartupError,
    };
    use crate::artifacts::{AttemptPaths, ValidatedOpenFile};
    use crate::worker_protocol::{Inspection, PageReasons, PdfTypeLabel};

    #[cfg(unix)]
    fn write_worker(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    fn worker_script(body: &str) -> String {
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'tool-kit-pdf-worker protocol=2 pdf-inspector=1.25.2\\n'\n  exit 0\nfi\n{body}\n"
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

        let result = verify_worker_identity(&worker);
        assert!(
            matches!(
                result,
                Err(WorkerStartupError::WorkerIdentityMismatch { .. })
            ),
            "{result:?}"
        );
    }

    #[test]
    fn startup_rejects_an_incomplete_cmap_directory() {
        let directory = tempfile::tempdir().unwrap();

        assert!(matches!(
            validate_cmaps(directory.path()),
            Err(EngineStartupError::InvalidCmapSentinel { .. })
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn conversion_passes_the_validated_handle_not_a_reopened_path() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &worker_script(
                "printf '%s' \"$TOOLKIT_WORKER_EXPECTED_SOURCE_SHA256\" > observed-source-sha256\n/bin/cat > observed-source\nexit 1",
            ),
        );
        let paths = paths(directory.path());
        let source = source(&paths.source, b"original").await;
        let expected_sha256 = source.sha256.clone();
        tokio::fs::rename(&paths.source, directory.path().join("opened-source"))
            .await
            .unwrap();
        tokio::fs::write(&paths.source, b"replacement")
            .await
            .unwrap();

        let engine =
            PdfInspectorEngine::initialize(worker, None, Duration::from_secs(2), 1024, 1).unwrap();
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let result = engine.convert(&paths, source, permit, cancellation).await;

        assert_eq!(result.unwrap_err(), EngineFailure::Crashed);
        assert_eq!(
            tokio::fs::read(paths.attempt.join("observed-source"))
                .await
                .unwrap(),
            b"original"
        );
        assert_eq!(
            tokio::fs::read_to_string(paths.attempt.join("observed-source-sha256"))
                .await
                .unwrap(),
            expected_sha256
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn conversion_binds_in_place_mutation_to_the_stored_digest() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &worker_script(
                "printf '%s' \"$TOOLKIT_WORKER_EXPECTED_SOURCE_SHA256\" > observed-source-sha256\n/bin/cat > observed-source\nexit 1",
            ),
        );
        let paths = paths(directory.path());
        let source = source(&paths.source, b"original").await;
        let stored_sha256 = source.sha256.clone();
        tokio::fs::write(&paths.source, b"mutation").await.unwrap();

        let engine =
            PdfInspectorEngine::initialize(worker, None, Duration::from_secs(2), 1024, 1).unwrap();
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let result = engine.convert(&paths, source, permit, cancellation).await;

        assert_eq!(result.unwrap_err(), EngineFailure::Crashed);
        assert_eq!(
            tokio::fs::read(paths.attempt.join("observed-source"))
                .await
                .unwrap(),
            b"mutation"
        );
        assert_eq!(
            tokio::fs::read_to_string(paths.attempt.join("observed-source-sha256"))
                .await
                .unwrap(),
            stored_sha256
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn conversion_rejects_noncanonical_source_digest_before_spawn() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &worker_script("printf started > worker-started\nexit 1"),
        );
        let paths = paths(directory.path());
        let mut source = source(&paths.source, b"source").await;
        source.sha256 = "A".repeat(64);

        let engine =
            PdfInspectorEngine::initialize(worker, None, Duration::from_secs(2), 1024, 1).unwrap();
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let result = engine.convert(&paths, source, permit, cancellation).await;

        assert_eq!(result.unwrap_err(), EngineFailure::Protocol);
        assert!(!paths.attempt.join("worker-started").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_kills_and_reaps_the_worker() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &worker_script("printf started > worker-started\nexec /bin/sleep 30"),
        );
        let paths = paths(directory.path());
        let source = source(&paths.source, b"source").await;
        let engine =
            PdfInspectorEngine::initialize(worker, None, Duration::from_secs(30), 1024, 1).unwrap();
        let permit = engine.acquire().await.unwrap();
        let (cancel, cancellation) = watch::channel(false);
        let marker = paths.attempt.join("worker-started");
        let task_paths = paths.clone();
        let task = tokio::spawn(async move {
            engine
                .convert(&task_paths, source, permit, cancellation)
                .await
        });
        for _ in 0..200 {
            if marker.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(marker.exists());
        cancel.send(true).unwrap();

        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("cancelled worker should be reaped promptly")
            .unwrap();
        assert_eq!(result.unwrap_err(), EngineFailure::Interrupted);
    }

    #[test]
    fn completed_native_inspection_allows_historical_ocr_reason_details() {
        let mut inspection = Inspection {
            pdf_type: PdfTypeLabel::TextBased,
            confidence: 1.0,
            page_count: 1,
            pages_needing_ocr: Vec::new(),
            ocr_reasons_by_page: vec![PageReasons {
                page: 1,
                reasons: vec!["diagnostic_only".to_owned()],
            }],
            has_encoding_issues: false,
            is_complex: false,
            pages_with_tables: Vec::new(),
            pages_with_columns: Vec::new(),
            processing_time_ms: 1,
        };
        assert!(is_complete_native_inspection(&inspection));

        inspection.pages_needing_ocr.push(1);
        assert!(!is_complete_native_inspection(&inspection));
    }
}
