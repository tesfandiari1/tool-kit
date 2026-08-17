use std::{
    io::Read as _,
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
    sync::Arc,
    thread,
    time::Duration,
};

use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs,
    io::AsyncReadExt,
    process::Command,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};

use crate::{
    artifacts::AttemptPaths,
    worker_protocol::{
        FallbackReason, Inspection, PdfTypeLabel, RejectionCode, WorkerOutcome, WorkerReport,
        MARKDOWN_FILE, PDF_INSPECTOR_VERSION, WORKER_MAX_OUTPUT_BYTES_ENV, WORKER_PROTOCOL_VERSION,
        WORKER_REPORT_FILE,
    },
};

const MAX_REPORT_BYTES: u64 = 1024 * 1024;
const WORKER_IDENTITY_TIMEOUT: Duration = Duration::from_secs(2);
const EXPECTED_WORKER_IDENTITY: &str = "tool-kit-pdf-worker protocol=1 pdf-inspector=1.15.0\n";
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
        validate_worker(&worker_path)?;
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
        self.permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| EngineFailure::Unavailable)
    }

    pub async fn convert(
        &self,
        paths: &AttemptPaths,
        _permit: OwnedSemaphorePermit,
    ) -> Result<EngineOutcome, EngineFailure> {
        let mut command = Command::new(self.worker_path.as_path());
        command
            .arg(&paths.source)
            .arg(&paths.publication_staging)
            .current_dir(&paths.attempt)
            .env_clear()
            .env("RAYON_NUM_THREADS", self.rayon_threads.to_string())
            .env(
                WORKER_MAX_OUTPUT_BYTES_ENV,
                self.max_output_bytes.to_string(),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(path) = self.bcmaps_dir.as_deref() {
            command.env("PDF_INSPECTOR_BCMAPS_DIR", path.as_path());
        }

        let mut child = command.spawn().map_err(|_| EngineFailure::Unavailable)?;
        let status = match timeout(self.timeout, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(_)) => return Err(EngineFailure::Crashed),
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(EngineFailure::Timeout);
            }
        };
        if !status.success() {
            return Err(EngineFailure::Crashed);
        }

        self.read_and_validate_report(paths).await
    }

    async fn read_and_validate_report(
        &self,
        paths: &AttemptPaths,
    ) -> Result<EngineOutcome, EngineFailure> {
        let report_path = paths.publication_staging.join(WORKER_REPORT_FILE);
        let metadata = fs::symlink_metadata(&report_path)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_REPORT_BYTES
        {
            return Err(EngineFailure::Protocol);
        }
        let encoded = fs::read(&report_path)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        let report: WorkerReport =
            serde_json::from_slice(&encoded).map_err(|_| EngineFailure::Protocol)?;
        validate_identity(&report)?;

        let outcome = match report.outcome {
            WorkerOutcome::Converted {
                inspection,
                artifact,
            } => {
                validate_complete_inspection(&inspection)?;
                if artifact.relative_path != MARKDOWN_FILE
                    || artifact.byte_length == 0
                    || artifact.byte_length > self.max_output_bytes
                    || !is_sha256(&artifact.sha256)
                {
                    return Err(EngineFailure::Protocol);
                }
                let markdown_path = paths.staged_markdown();
                let markdown_metadata = fs::symlink_metadata(&markdown_path)
                    .await
                    .map_err(|_| EngineFailure::Protocol)?;
                if !markdown_metadata.is_file()
                    || markdown_metadata.file_type().is_symlink()
                    || markdown_metadata.len() != artifact.byte_length
                    || markdown_metadata.len() > self.max_output_bytes
                {
                    return Err(EngineFailure::Protocol);
                }
                let (digest, has_content) = hash_and_check_content(&markdown_path).await?;
                if !has_content || digest != artifact.sha256 {
                    return Err(EngineFailure::Protocol);
                }
                EngineOutcome::Converted {
                    inspection,
                    byte_length: artifact.byte_length,
                    sha256: digest,
                }
            }
            WorkerOutcome::NeedsRemote {
                inspection,
                reason_code,
            } => {
                validate_needs_remote(&inspection, reason_code)?;
                if fs::try_exists(paths.staged_markdown())
                    .await
                    .map_err(|_| EngineFailure::Protocol)?
                {
                    return Err(EngineFailure::Protocol);
                }
                EngineOutcome::NeedsRemote {
                    inspection,
                    reason_code,
                }
            }
            WorkerOutcome::Rejected { code } => {
                if fs::try_exists(paths.staged_markdown())
                    .await
                    .map_err(|_| EngineFailure::Protocol)?
                {
                    return Err(EngineFailure::Protocol);
                }
                EngineOutcome::Rejected { code }
            }
        };
        fs::remove_file(report_path)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        Ok(outcome)
    }
}

#[derive(Clone, Debug)]
pub enum EngineOutcome {
    Converted {
        inspection: Inspection,
        byte_length: u64,
        sha256: String,
    },
    NeedsRemote {
        inspection: Inspection,
        reason_code: FallbackReason,
    },
    Rejected {
        code: RejectionCode,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineFailure {
    Unavailable,
    Timeout,
    Crashed,
    Protocol,
}

impl EngineFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "worker_unavailable",
            Self::Timeout => "worker_timeout",
            Self::Crashed => "worker_crash",
            Self::Protocol => "worker_protocol_error",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "The local PDF worker is unavailable.",
            Self::Timeout => "The local PDF worker exceeded its time limit.",
            Self::Crashed => "The local PDF worker stopped unexpectedly.",
            Self::Protocol => "The local PDF worker returned an invalid result.",
        }
    }
}

fn validate_worker(path: &Path) -> Result<(), EngineStartupError> {
    let metadata = std::fs::metadata(path).map_err(|source| EngineStartupError::InvalidWorker {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(EngineStartupError::WorkerNotFile(path.to_owned()));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(EngineStartupError::WorkerNotExecutable(path.to_owned()));
        }
    }
    Ok(())
}

fn verify_worker_identity(path: &Path) -> Result<(), EngineStartupError> {
    let mut child = StdCommand::new(path)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| EngineStartupError::WorkerHandshake {
            path: path.to_owned(),
            source,
        })?;
    let deadline = std::time::Instant::now() + WORKER_IDENTITY_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EngineStartupError::WorkerIdentityTimeout(path.to_owned()));
            }
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EngineStartupError::WorkerHandshake {
                    path: path.to_owned(),
                    source,
                });
            }
        }
    };
    let mut output = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        stdout
            .take(4097)
            .read_to_end(&mut output)
            .map_err(|source| EngineStartupError::WorkerHandshake {
                path: path.to_owned(),
                source,
            })?;
    }
    if !status.success() || output.as_slice() != EXPECTED_WORKER_IDENTITY.as_bytes() {
        return Err(EngineStartupError::WorkerIdentityMismatch(path.to_owned()));
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

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn hash_and_check_content(path: &Path) -> Result<(String, bool), EngineFailure> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|_| EngineFailure::Protocol)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut has_content = false;
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        has_content |= buffer[..count]
            .iter()
            .any(|byte| !byte.is_ascii_whitespace());
    }
    Ok((hex::encode(digest.finalize()), has_content))
}

#[derive(Debug, Error)]
pub enum EngineStartupError {
    #[error("PDF worker is unavailable at {path:?}")]
    InvalidWorker {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("PDF worker path is not a regular file: {0:?}")]
    WorkerNotFile(PathBuf),
    #[error("PDF worker path is not executable: {0:?}")]
    WorkerNotExecutable(PathBuf),
    #[error("PDF worker handshake failed at {path:?}")]
    WorkerHandshake {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("PDF worker identity does not match the pinned protocol: {0:?}")]
    WorkerIdentityMismatch(PathBuf),
    #[error("PDF worker identity check timed out: {0:?}")]
    WorkerIdentityTimeout(PathBuf),
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
    use super::{validate_cmaps, verify_worker_identity, EngineStartupError};

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
            Err(EngineStartupError::WorkerIdentityMismatch(_))
        ));
    }

    #[test]
    fn startup_rejects_an_incomplete_cmap_directory() {
        let directory = tempfile::tempdir().unwrap();

        assert!(matches!(
            validate_cmaps(directory.path()),
            Err(EngineStartupError::InvalidCmapSentinel { .. })
        ));
    }
}
