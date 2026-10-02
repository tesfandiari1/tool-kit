use std::{
    env,
    fs::{File, OpenOptions},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

#[cfg(target_os = "linux")]
use pdf_inspector::process_pdf_with_options;
use pdf_inspector::{DetectionConfig, PdfError, PdfOptions, PdfType, ScanStrategy};
use sha2::{Digest, Sha256};
use tool_kit_converter::worker_protocol::{
    EngineIdentity, FallbackReason, Inspection, PageReasons, PdfTypeLabel, RejectionCode,
    WorkerArtifact, WorkerOutcome, WorkerReport, MARKDOWN_FILE, PDF_ENGINE_NAME,
    PDF_INSPECTOR_VERSION, WORKER_EXPECTED_SOURCE_BYTES_ENV, WORKER_EXPECTED_SOURCE_SHA256_ENV,
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
    let [output_directory] = args.as_slice() else {
        return Err(());
    };
    let expected_source_bytes = required_positive_u64_env(WORKER_EXPECTED_SOURCE_BYTES_ENV)?;
    let expected_source_sha256 = required_lowercase_sha256_env(WORKER_EXPECTED_SOURCE_SHA256_ENV)?;
    let max_output_bytes = required_positive_u64_env(WORKER_MAX_OUTPUT_BYTES_ENV)?;
    let output_directory = PathBuf::from(output_directory);
    let output_metadata = std::fs::symlink_metadata(&output_directory).map_err(|_| ())?;
    if output_metadata.file_type().is_symlink() || !output_metadata.is_dir() {
        return Err(());
    }

    // The parent gives this process the already-open, validated source as
    // stdin. Bind both its length and digest while copying into a
    // worker-private anonymous file so pdf-inspector never reopens the mutable
    // jobs path or parses bytes that changed after parent-side validation.
    let mut private_source = tempfile::tempfile().map_err(|_| ())?;
    copy_exact_source(
        std::io::stdin().lock(),
        &mut private_source,
        expected_source_bytes,
        &expected_source_sha256,
    )?;

    let detection = DetectionConfig {
        strategy: ScanStrategy::Full,
        ..DetectionConfig::default()
    };
    let result = process_private_source(
        &mut private_source,
        expected_source_bytes,
        PdfOptions::new().detection(detection),
    )?;
    let outcome = match result {
        Ok(result) => convert_result(result, &output_directory, max_output_bytes)?,
        Err(error) => WorkerOutcome::Rejected {
            code: rejection_code(&error),
        },
    };
    write_report(&output_directory, outcome)
}

fn required_positive_u64_env(name: &str) -> Result<u64, ()> {
    let value = env::var(name)
        .map_err(|_| ())?
        .parse::<u64>()
        .map_err(|_| ())?;
    if value == 0 {
        Err(())
    } else {
        Ok(value)
    }
}

fn required_lowercase_sha256_env(name: &str) -> Result<String, ()> {
    let value = env::var(name).map_err(|_| ())?;
    if is_lowercase_sha256(&value) {
        Ok(value)
    } else {
        Err(())
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn copy_exact_source(
    mut input: impl Read,
    output: &mut File,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<(), ()> {
    if expected_bytes == 0 || !is_lowercase_sha256(expected_sha256) {
        return Err(());
    }

    let mut remaining = expected_bytes;
    let mut buffer = [0_u8; 64 * 1024];
    let mut digest = Sha256::new();
    while remaining > 0 {
        let requested = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| ())?;
        let count = read_retry(&mut input, &mut buffer[..requested])?;
        if count == 0 {
            return Err(());
        }
        output.write_all(&buffer[..count]).map_err(|_| ())?;
        digest.update(&buffer[..count]);
        remaining -= u64::try_from(count).map_err(|_| ())?;
    }

    let mut extra = [0_u8; 1];
    if read_retry(&mut input, &mut extra)? != 0 {
        return Err(());
    }
    if hex::encode(digest.finalize()) != expected_sha256 {
        return Err(());
    }
    output.flush().map_err(|_| ())?;
    output.seek(std::io::SeekFrom::Start(0)).map_err(|_| ())?;
    Ok(())
}

fn read_retry(input: &mut impl Read, buffer: &mut [u8]) -> Result<usize, ()> {
    loop {
        match input.read(buffer) {
            Ok(count) => return Ok(count),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(()),
        }
    }
}

fn process_private_source(
    source: &mut File,
    _expected_source_bytes: u64,
    options: PdfOptions,
) -> Result<Result<pdf_inspector::PdfProcessResult, PdfError>, ()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;

        let source_path = PathBuf::from(format!("/proc/self/fd/{}", source.as_raw_fd()));
        Ok(process_pdf_with_options(source_path, options))
    }

    #[cfg(not(target_os = "linux"))]
    {
        // macOS `/dev/fd/N` opens share a cursor across pdf-inspector's
        // validation and parse opens. Feeding the anonymous file through the
        // crate's memory API preserves the same immutable-handle guarantee on
        // macOS (and is the conservative fallback on other build targets).
        use pdf_inspector::process_pdf_mem_with_options;

        let capacity = usize::try_from(_expected_source_bytes).map_err(|_| ())?;
        let mut bytes = Vec::with_capacity(capacity);
        source.read_to_end(&mut bytes).map_err(|_| ())?;
        if bytes.len() != capacity {
            return Err(());
        }
        Ok(process_pdf_mem_with_options(&bytes, options))
    }
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
            name: PDF_ENGINE_NAME.to_owned(),
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
    use std::io::{Cursor, Read};

    use sha2::{Digest, Sha256};
    use tool_kit_converter::worker_protocol::{
        EngineIdentity, PDF_INSPECTOR_VERSION, WORKER_PROTOCOL_VERSION,
    };

    use super::copy_exact_source;

    fn digest(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    #[test]
    fn protocol_identity_matches_the_exact_engine_pin() {
        let identity = EngineIdentity {
            name: "pdf-inspector".to_owned(),
            version: PDF_INSPECTOR_VERSION.to_owned(),
            features: Vec::new(),
        };

        assert_eq!(WORKER_PROTOCOL_VERSION, 2);
        assert_eq!(identity.version, "1.25.2");
        assert!(identity.features.is_empty());
        // The report names this version, so a dependency bump that leaves the
        // constant behind would stamp every manifest with the wrong engine.
        assert!(include_str!("../../Cargo.toml").contains(&format!(
            "pdf-inspector = {{ version = \"={PDF_INSPECTOR_VERSION}\""
        )));
    }

    #[test]
    fn source_copy_accepts_only_the_declared_length_and_rewinds() {
        let expected = digest(b"source");
        let mut exact = tempfile::tempfile().unwrap();
        copy_exact_source(Cursor::new(b"source"), &mut exact, 6, &expected).unwrap();
        let mut bytes = Vec::new();
        exact.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"source");

        let mut short_output = tempfile::tempfile().unwrap();
        assert!(copy_exact_source(Cursor::new(b"short"), &mut short_output, 6, &expected).is_err());

        let mut long_output = tempfile::tempfile().unwrap();
        assert!(
            copy_exact_source(Cursor::new(b"too-long"), &mut long_output, 7, &expected).is_err()
        );
    }

    #[test]
    fn source_copy_rejects_same_length_content_mutation_and_noncanonical_hashes() {
        let expected = digest(b"original");
        let mut mutated_output = tempfile::tempfile().unwrap();
        assert!(
            copy_exact_source(Cursor::new(b"mutation"), &mut mutated_output, 8, &expected,)
                .is_err()
        );

        let mut uppercase_output = tempfile::tempfile().unwrap();
        assert!(copy_exact_source(
            Cursor::new(b"original"),
            &mut uppercase_output,
            8,
            &expected.to_uppercase(),
        )
        .is_err());
    }
}
