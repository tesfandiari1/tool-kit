use std::{
    env,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use pdf_inspector::{
    process_pdf_with_options, DetectionConfig, PdfError, PdfOptions, PdfType, ScanStrategy,
};
use sha2::{Digest, Sha256};
use tool_kit_converter::worker_protocol::{
    EngineIdentity, FallbackReason, Inspection, PageReasons, PdfTypeLabel, RejectionCode,
    WorkerArtifact, WorkerOutcome, WorkerReport, MARKDOWN_FILE, PDF_INSPECTOR_VERSION,
    WORKER_MAX_OUTPUT_BYTES_ENV, WORKER_PROTOCOL_VERSION, WORKER_REPORT_FILE,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::from(70),
    }
}

fn run() -> Result<(), ()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.as_slice() == ["--version"] {
        println!(
            "tool-kit-pdf-worker protocol={} pdf-inspector={}",
            WORKER_PROTOCOL_VERSION, PDF_INSPECTOR_VERSION
        );
        return Ok(());
    }
    let [input, output_directory] = args.as_slice() else {
        return Err(());
    };
    let max_output_bytes = env::var(WORKER_MAX_OUTPUT_BYTES_ENV)
        .map_err(|_| ())?
        .parse::<u64>()
        .map_err(|_| ())?;
    if max_output_bytes == 0 {
        return Err(());
    }
    let output_directory = PathBuf::from(output_directory);
    if !output_directory.is_dir() {
        return Err(());
    }

    let detection = DetectionConfig {
        strategy: ScanStrategy::Full,
        ..DetectionConfig::default()
    };
    let outcome = match process_pdf_with_options(input, PdfOptions::new().detection(detection)) {
        Ok(result) => convert_result(result, &output_directory, max_output_bytes)?,
        Err(error) => WorkerOutcome::Rejected {
            code: rejection_code(&error),
        },
    };
    write_report(&output_directory, outcome)
}

fn convert_result(
    result: pdf_inspector::PdfProcessResult,
    output_directory: &Path,
    max_output_bytes: u64,
) -> Result<WorkerOutcome, ()> {
    let pdf_type = match result.pdf_type {
        PdfType::TextBased => PdfTypeLabel::TextBased,
        PdfType::Scanned => PdfTypeLabel::Scanned,
        PdfType::ImageBased => PdfTypeLabel::ImageBased,
        PdfType::Mixed => PdfTypeLabel::Mixed,
    };
    let inspection = Inspection {
        pdf_type,
        confidence: result.confidence,
        page_count: result.page_count,
        pages_needing_ocr: result.pages_needing_ocr,
        ocr_reasons_by_page: result
            .ocr_reasons_by_page
            .into_iter()
            .map(|entry| PageReasons {
                page: entry.page,
                reasons: entry.reasons,
            })
            .collect(),
        has_encoding_issues: result.has_encoding_issues,
        is_complex: result.layout.is_complex,
        pages_with_tables: result.layout.pages_with_tables,
        pages_with_columns: result.layout.pages_with_columns,
        processing_time_ms: result.processing_time_ms,
    };
    if inspection.page_count == 0 {
        return Ok(WorkerOutcome::Rejected {
            code: RejectionCode::InvalidPdfStructure,
        });
    }

    let fallback = match inspection.pdf_type {
        PdfTypeLabel::Scanned => Some(FallbackReason::ScannedPdf),
        PdfTypeLabel::ImageBased => Some(FallbackReason::ImageBasedPdf),
        PdfTypeLabel::Mixed => Some(FallbackReason::MixedPdf),
        PdfTypeLabel::TextBased if inspection.has_encoding_issues => {
            Some(FallbackReason::GarbledText)
        }
        PdfTypeLabel::TextBased if !inspection.pages_needing_ocr.is_empty() => {
            Some(FallbackReason::OcrRequired)
        }
        PdfTypeLabel::TextBased => None,
    };
    if let Some(reason_code) = fallback {
        return Ok(WorkerOutcome::NeedsRemote {
            inspection,
            reason_code,
        });
    }

    let Some(markdown) = result.markdown else {
        return Ok(WorkerOutcome::NeedsRemote {
            inspection,
            reason_code: FallbackReason::LocalQualityFailed,
        });
    };
    if markdown.trim().is_empty() {
        return Ok(WorkerOutcome::NeedsRemote {
            inspection,
            reason_code: FallbackReason::LocalQualityFailed,
        });
    }
    let byte_length = u64::try_from(markdown.len()).map_err(|_| ())?;
    if byte_length > max_output_bytes {
        return Ok(WorkerOutcome::NeedsRemote {
            inspection,
            reason_code: FallbackReason::OutputTooLarge,
        });
    }

    let markdown_path = output_directory.join(MARKDOWN_FILE);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&markdown_path)
        .map_err(|_| ())?;
    file.write_all(markdown.as_bytes()).map_err(|_| ())?;
    file.sync_all().map_err(|_| ())?;
    let digest = hex::encode(Sha256::digest(markdown.as_bytes()));

    Ok(WorkerOutcome::Converted {
        inspection,
        artifact: WorkerArtifact {
            relative_path: MARKDOWN_FILE.to_owned(),
            byte_length,
            sha256: digest,
        },
    })
}

fn write_report(output_directory: &Path, outcome: WorkerOutcome) -> Result<(), ()> {
    let report = WorkerReport {
        protocol_version: WORKER_PROTOCOL_VERSION,
        engine: EngineIdentity {
            name: "pdf-inspector".to_owned(),
            version: PDF_INSPECTOR_VERSION.to_owned(),
            features: Vec::new(),
        },
        outcome,
    };
    let encoded = serde_json::to_vec(&report).map_err(|_| ())?;
    let report_path = output_directory.join(WORKER_REPORT_FILE);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(report_path)
        .map_err(|_| ())?;
    file.write_all(&encoded).map_err(|_| ())?;
    file.sync_all().map_err(|_| ())
}

fn rejection_code(error: &PdfError) -> RejectionCode {
    match error {
        PdfError::Encrypted => RejectionCode::EncryptedPdf,
        PdfError::InvalidStructure => RejectionCode::InvalidPdfStructure,
        PdfError::Io(_) | PdfError::Parse(_) | PdfError::NotAPdf(_) => RejectionCode::InvalidPdf,
    }
}

#[cfg(test)]
mod tests {
    use tool_kit_converter::worker_protocol::{
        EngineIdentity, PDF_INSPECTOR_VERSION, WORKER_PROTOCOL_VERSION,
    };

    #[test]
    fn protocol_identity_matches_the_exact_engine_pin() {
        let identity = EngineIdentity {
            name: "pdf-inspector".to_owned(),
            version: PDF_INSPECTOR_VERSION.to_owned(),
            features: Vec::new(),
        };

        assert_eq!(WORKER_PROTOCOL_VERSION, 1);
        assert_eq!(identity.version, "1.15.0");
        assert!(identity.features.is_empty());
    }
}
