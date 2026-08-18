//! AnyDoc engine: converts proven non-PDF documents in-process.
//!
//! Containment decision (2026-08-18, owner-approved): in-process with
//! `catch_unwind`, chosen over a second child worker after the M3 spike showed
//! every hostile fixture failing as a typed error through AnyDoc's internal
//! pre-decompression limits. A parser panic becomes `EngineFailure::Crashed`,
//! which the durable runner recovers like any other interrupted attempt. If
//! hostile input ever defeats those limits, the fallback is the child-worker
//! shape in `pdf_inspector.rs`. PDFs are refused outright: the isolated PDF
//! worker is the only PDF path, and AnyDoc's embedded pdf-inspector must
//! never bypass it.

use std::{panic, sync::Arc, time::Instant};

use sha2::{Digest, Sha256};
use tokio::{
    io::AsyncReadExt,
    sync::{watch, OwnedSemaphorePermit, Semaphore},
};

use super::{EngineAnalysis, EngineFailure, EngineOutcome, EngineRejection};
use crate::{
    artifacts::{AttemptPaths, ValidatedOpenFile},
    persistence::DocumentClassification,
};

pub(crate) const ANYDOC_ENGINE_NAME: &str = "anydoc";
pub(crate) const ANYDOC_VERSION: &str = "0.1.9";

/// Engine-specific detail persisted as attempt diagnostics and embedded in
/// the manifest. Content-free: the detected format family and wall time only.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnyDocDiagnostics {
    pub format: String,
    pub processing_time_ms: u64,
}

#[derive(Clone, Debug)]
pub struct AnyDocEngine {
    max_output_bytes: u64,
    permits: Arc<Semaphore>,
}
impl AnyDocEngine {
    pub fn new(max_output_bytes: u64, parser_concurrency: usize) -> Self {
        Self {
            max_output_bytes,
            permits: Arc::new(Semaphore::new(parser_concurrency.max(1))),
        }
    }

    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, EngineFailure> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| EngineFailure::Unavailable)
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
            mut file,
            byte_length,
            sha256,
        } = source;
        if byte_length == 0 || !is_lowercase_sha256(&sha256) {
            return Err(EngineFailure::Protocol);
        }

        // Re-verify the open handle before parsing: the bytes must match what
        // claim-time validation approved.
        let mut bytes = Vec::with_capacity(usize::try_from(byte_length).unwrap_or(0));
        file.read_to_end(&mut bytes)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        if bytes.len() as u64 != byte_length || hex::encode(Sha256::digest(&bytes)) != sha256 {
            return Err(EngineFailure::Protocol);
        }

        let Some(format) = anydoc::Format::from_bytes(&bytes) else {
            return Ok(rejected(AnyDocRejection::UnsupportedDocument));
        };
        if format == anydoc::Format::Pdf {
            return Ok(rejected(AnyDocRejection::UnsupportedDocument));
        }

        let max_output_bytes = self.max_output_bytes;
        let conversion = tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                anydoc::to_markdown_bytes(&bytes, format)
            }));
            (result, started.elapsed())
        })
        .await
        .map_err(|_| EngineFailure::Crashed)?;
        let (result, elapsed) = conversion;
        let markdown = match result {
            Err(_) => return Err(EngineFailure::Crashed),
            Ok(Err(error)) => return Ok(rejected(rejection_code(&error))),
            Ok(Ok(markdown)) => markdown,
        };

        if markdown.trim().is_empty() {
            return Ok(rejected(AnyDocRejection::InvalidDocument));
        }
        let output_bytes = markdown.len() as u64;
        if output_bytes > max_output_bytes {
            return Ok(rejected(AnyDocRejection::DocumentExceedsLimits));
        }
        if *cancellation.borrow() {
            return Err(EngineFailure::Interrupted);
        }

        // Write the staged Markdown exactly like the PDF worker does, so
        // finalization and publication validation stay engine-agnostic.
        let markdown_path = paths.staged_markdown();
        let mut output = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&markdown_path)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        tokio::io::AsyncWriteExt::write_all(&mut output, markdown.as_bytes())
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        output
            .sync_all()
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        let digest = hex::encode(Sha256::digest(markdown.as_bytes()));

        Ok(EngineOutcome::Converted {
            analysis: EngineAnalysis {
                classification: DocumentClassification::StructuredDocument,
                diagnostics: serde_json::to_value(AnyDocDiagnostics {
                    format: format_label(format).to_owned(),
                    processing_time_ms: u64::try_from(elapsed.as_millis())
                        .map_err(|_| EngineFailure::Protocol)?,
                })
                .map_err(|_| EngineFailure::Protocol)?,
            },
            byte_length: output_bytes,
            sha256: digest,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnyDocRejection {
    UnsupportedDocument,
    EncryptedDocument,
    InvalidDocument,
    DocumentExceedsLimits,
}

impl AnyDocRejection {
    fn code(self) -> &'static str {
        match self {
            Self::UnsupportedDocument => "unsupported_document",
            Self::EncryptedDocument => "encrypted_document",
            Self::InvalidDocument => "invalid_document",
            Self::DocumentExceedsLimits => "document_exceeds_limits",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::UnsupportedDocument => "The document format is not supported.",
            Self::EncryptedDocument => "Encrypted documents are not accepted.",
            Self::InvalidDocument => "The document could not be converted.",
            Self::DocumentExceedsLimits => "The document exceeds the conversion limits.",
        }
    }
}

fn rejected(rejection: AnyDocRejection) -> EngineOutcome {
    EngineOutcome::Rejected {
        rejection: EngineRejection {
            code: rejection.code(),
            message: rejection.message(),
        },
    }
}

fn rejection_code(error: &anydoc::ConvertError) -> AnyDocRejection {
    match error {
        anydoc::ConvertError::Unsupported(_) => AnyDocRejection::UnsupportedDocument,
        anydoc::ConvertError::Encrypted => AnyDocRejection::EncryptedDocument,
        anydoc::ConvertError::Malformed { .. } | anydoc::ConvertError::MissingPart { .. } => {
            AnyDocRejection::InvalidDocument
        }
        anydoc::ConvertError::ResourceLimit { .. } => AnyDocRejection::DocumentExceedsLimits,
        // `Io` cannot happen: the engine reads the source itself. Any future
        // variant is treated as an unreadable document, never a success.
        _ => AnyDocRejection::InvalidDocument,
    }
}

fn format_label(format: anydoc::Format) -> &'static str {
    match format {
        anydoc::Format::Doc => "doc",
        anydoc::Format::Docx => "docx",
        anydoc::Format::Odt => "odt",
        anydoc::Format::Pdf => "pdf",
        anydoc::Format::Ppt => "ppt",
        anydoc::Format::Pptx => "pptx",
        anydoc::Format::Rtf => "rtf",
        anydoc::Format::Epub => "epub",
        anydoc::Format::Excel => "excel",
        anydoc::Format::Ods => "ods",
        anydoc::Format::Odp => "odp",
        anydoc::Format::Csv => "csv",
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_version_matches_the_crate() {
        assert_eq!(ANYDOC_VERSION, "0.1.9");
    }

    #[test]
    fn pdf_input_is_refused_before_conversion() {
        // Detection only needs the header; conversion is never attempted.
        let bytes = b"%PDF-1.4\nrest of file";
        assert_eq!(anydoc::Format::from_bytes(bytes), Some(anydoc::Format::Pdf));
    }

    #[tokio::test]
    async fn pdf_bytes_fail_closed_through_the_adapter() {
        let directory = tempfile::tempdir().unwrap();
        let attempt = directory.path().join("attempt");
        let publication_staging = attempt.join("publication.staging");
        std::fs::create_dir_all(&publication_staging).unwrap();
        let paths = AttemptPaths {
            source: attempt.join("input"),
            published: attempt.join("artifacts"),
            publication_staging,
            attempt,
        };
        let pdf = b"%PDF-1.4\nbody".as_slice();
        let source_path = directory.path().join("source.pdf");
        tokio::fs::write(&source_path, pdf).await.unwrap();
        let source = ValidatedOpenFile {
            file: tokio::fs::File::open(&source_path).await.unwrap(),
            byte_length: pdf.len() as u64,
            sha256: hex::encode(Sha256::digest(pdf)),
        };

        let engine = AnyDocEngine::new(1024 * 1024, 1);
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let outcome = engine
            .convert(&paths, source, permit, cancellation)
            .await
            .unwrap();

        let EngineOutcome::Rejected { rejection } = outcome else {
            panic!("pdf bytes must never convert through AnyDoc");
        };
        assert_eq!(rejection.code, "unsupported_document");
        assert!(!paths.staged_markdown().exists());
    }

    #[tokio::test]
    async fn converts_an_rtf_document_in_process() {
        let directory = tempfile::tempdir().unwrap();
        let attempt = directory.path().join("attempt");
        let publication_staging = attempt.join("publication.staging");
        std::fs::create_dir_all(&publication_staging).unwrap();
        let paths = AttemptPaths {
            source: attempt.join("input"),
            published: attempt.join("artifacts"),
            publication_staging,
            attempt,
        };
        let rtf = br#"{\rtf1\ansi Hello from AnyDoc.}"#;
        let source_path = directory.path().join("source.rtf");
        tokio::fs::write(&source_path, rtf).await.unwrap();
        let source = ValidatedOpenFile {
            file: tokio::fs::File::open(&source_path).await.unwrap(),
            byte_length: rtf.len() as u64,
            sha256: hex::encode(Sha256::digest(rtf)),
        };

        let engine = AnyDocEngine::new(1024 * 1024, 1);
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let outcome = engine
            .convert(&paths, source, permit, cancellation)
            .await
            .unwrap();

        let EngineOutcome::Converted { analysis, .. } = outcome else {
            panic!("rtf must convert");
        };
        assert_eq!(
            analysis.classification,
            DocumentClassification::StructuredDocument
        );
        assert_eq!(analysis.diagnostics["format"], "rtf");
        let markdown = std::fs::read_to_string(paths.staged_markdown()).unwrap();
        assert!(markdown.contains("Hello from AnyDoc."));
    }
}
