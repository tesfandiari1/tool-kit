use std::{
    collections::HashMap,
    env,
    fs::{File, OpenOptions},
    io::{Cursor, Read, Seek, Write},
    panic,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

use anydoc::model::{Block, CellSlot, ImageSource, Inline};
#[cfg(target_os = "linux")]
use pdf_inspector::process_pdf_with_options;
use pdf_inspector::{
    extract_pages_markdown_mem, DetectionConfig, PdfError, PdfOptions, PdfType, ScanStrategy,
};
use sha2::{Digest, Sha256};
use tool_kit_converter::worker_protocol::{
    EngineIdentity, FallbackReason, Inspection, NativePages, PageReasons, PdfTypeLabel,
    RejectionCode, WorkerArtifact, WorkerOutcome, WorkerReport, MARKDOWN_FILE, NATIVE_PAGES_FILE,
    PDF_ENGINE_NAME, PDF_INSPECTOR_VERSION, WORKER_EXPECTED_SOURCE_BYTES_ENV,
    WORKER_EXPECTED_SOURCE_SHA256_ENV, WORKER_MAX_OUTPUT_BYTES_ENV, WORKER_PROTOCOL_VERSION,
    WORKER_REPORT_FILE,
};
use tool_kit_worker_protocol::anydoc::{
    AnyDocRejection, AnyDocReport, MarkedPictures, ANYDOC_MARKDOWN_FILE, ANYDOC_MARKED_FILE,
    ANYDOC_PICTURES_DIRECTORY, ANYDOC_REPORT_FILE,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::from(70),
    }
}

fn run() -> Result<(), ()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["--version"] => {
            println!(
                "tool-kit-pdf-worker protocol={} pdf-inspector={}",
                WORKER_PROTOCOL_VERSION, PDF_INSPECTOR_VERSION
            );
            Ok(())
        }
        ["--anydoc", label, "--mark", output_directory] => {
            run_anydoc(label, true, output_directory)
        }
        ["--anydoc", label, output_directory] => run_anydoc(label, false, output_directory),
        [output_directory] => run_pdf(output_directory),
        _ => Err(()),
    }
}

fn run_pdf(output_directory: &str) -> Result<(), ()> {
    let max_output_bytes = required_positive_u64_env(WORKER_MAX_OUTPUT_BYTES_ENV)?;
    let (output_directory, mut private_source, expected_source_bytes) =
        private_source(output_directory)?;

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
        Ok(result) => convert_result(
            result,
            &mut private_source,
            &output_directory,
            max_output_bytes,
        )?,
        Err(error) => WorkerOutcome::Rejected {
            code: rejection_code(&error),
        },
    };
    write_report(&output_directory, outcome)
}

fn run_anydoc(label: &str, mark: bool, output_directory: &str) -> Result<(), ()> {
    let (output_directory, mut private_source, expected_source_bytes) =
        private_source(output_directory)?;
    let mut bytes = Vec::with_capacity(usize::try_from(expected_source_bytes).map_err(|_| ())?);
    private_source.read_to_end(&mut bytes).map_err(|_| ())?;
    let report = convert_anydoc(&bytes, label, mark, &output_directory)?;
    let encoded = serde_json::to_vec(&report).map_err(|_| ())?;
    std::fs::write(output_directory.join(ANYDOC_REPORT_FILE), encoded).map_err(|_| ())
}

/// Checks the output directory and copies the source off stdin.
fn private_source(output_directory: &str) -> Result<(PathBuf, File, u64), ()> {
    let expected_source_bytes = required_positive_u64_env(WORKER_EXPECTED_SOURCE_BYTES_ENV)?;
    let expected_source_sha256 = required_lowercase_sha256_env(WORKER_EXPECTED_SOURCE_SHA256_ENV)?;
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
    Ok((output_directory, private_source, expected_source_bytes))
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
    source: &mut File,
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
        let native_pages = match reason_code {
            FallbackReason::MixedPdf
            | FallbackReason::OcrRequired
            | FallbackReason::GarbledText => {
                stage_native_pages(source, output_directory, max_output_bytes)?
            }
            _ => None,
        };
        return Ok(WorkerOutcome::NeedsRemote {
            inspection,
            reason_code,
            native_pages,
        });
    }

    let Some(markdown) = result.markdown else {
        return Ok(WorkerOutcome::NeedsRemote {
            inspection,
            reason_code: FallbackReason::LocalQualityFailed,
            native_pages: None,
        });
    };
    if markdown.trim().is_empty() {
        return Ok(WorkerOutcome::NeedsRemote {
            inspection,
            reason_code: FallbackReason::LocalQualityFailed,
            native_pages: None,
        });
    }
    let byte_length = u64::try_from(markdown.len()).map_err(|_| ())?;
    if byte_length > max_output_bytes {
        return Ok(WorkerOutcome::NeedsRemote {
            inspection,
            reason_code: FallbackReason::OutputTooLarge,
            native_pages: None,
        });
    }

    Ok(WorkerOutcome::Converted {
        inspection,
        artifact: stage(output_directory, MARKDOWN_FILE, markdown.as_bytes())?,
    })
}

/// The native pages of a PDF that needs OCR on some pages only. `None` when
/// every page or no page needs it: a whole scan goes to Vision as it is, and
/// with nothing to OCR there is nothing to splice.
fn stage_native_pages(
    source: &mut File,
    output_directory: &Path,
    max_output_bytes: u64,
) -> Result<Option<WorkerArtifact>, ()> {
    source.rewind().map_err(|_| ())?;
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes).map_err(|_| ())?;
    let Ok(extraction) = extract_pages_markdown_mem(&bytes, None) else {
        return Ok(None);
    };
    let pages: Vec<Option<String>> = extraction
        .pages
        .into_iter()
        .map(|page| (!page.needs_ocr).then_some(page.markdown))
        .collect();
    if pages.iter().all(Option::is_some) || pages.iter().all(Option::is_none) {
        return Ok(None);
    }
    let encoded = serde_json::to_vec(&NativePages { pages }).map_err(|_| ())?;
    if u64::try_from(encoded.len()).map_err(|_| ())? > max_output_bytes {
        return Ok(None);
    }
    stage(output_directory, NATIVE_PAGES_FILE, &encoded).map(Some)
}

fn stage(output_directory: &Path, name: &str, bytes: &[u8]) -> Result<WorkerArtifact, ()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output_directory.join(name))
        .map_err(|_| ())?;
    file.write_all(bytes).map_err(|_| ())?;
    file.sync_all().map_err(|_| ())?;
    Ok(WorkerArtifact {
        relative_path: name.to_owned(),
        byte_length: u64::try_from(bytes.len()).map_err(|_| ())?,
        sha256: hex::encode(Sha256::digest(bytes)),
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

/// The admission label of the one format detection cannot see.
const CSV_FORMAT_LABEL: &str = "csv";

/// Detects and converts one non-PDF document. Detection needs the whole file,
/// so it runs here and the converter never holds the document.
fn convert_anydoc(
    bytes: &[u8],
    label: &str,
    mark: bool,
    output_directory: &Path,
) -> Result<AnyDocReport, ()> {
    let detected = anydoc::Format::from_bytes(bytes);
    // The PDF mode is the only PDF path. AnyDoc's embedded pdf-inspector must
    // never bypass it.
    if detected == Some(anydoc::Format::Pdf) {
        return Ok(AnyDocReport::Rejected {
            code: AnyDocRejection::UnsupportedDocument,
        });
    }
    // CSV is the one signature-less format: detection always returns
    // `None`, so the admitted extension names it. Every other format is
    // content-detected, and content that detects as nothing is corrupt or
    // mislabeled, not "unsupported".
    let detected = detected.or_else(|| (label == CSV_FORMAT_LABEL).then_some(anydoc::Format::Csv));
    // docx, xlsx, pptx and epub share one ZIP magic (doc/xls/ppt one OLE
    // magic), so admission cannot tell them apart. Catch the cross-family
    // mislabel here, before parsing: otherwise the file converts fine and
    // the manifest check rejects it afterwards as `artifact_integrity_failed`,
    // a corruption code, for what is only a misnamed upload.
    let Some(format) = detected.filter(|format| format_label(*format) == label) else {
        return Ok(AnyDocReport::Rejected {
            code: AnyDocRejection::InvalidDocument,
        });
    };

    let started = Instant::now();
    let markdown = match anydoc::to_markdown_bytes(bytes, format) {
        Ok(markdown) => markdown,
        Err(error) => {
            return Ok(AnyDocReport::Rejected {
                code: anydoc_rejection(&error),
            })
        }
    };
    let processing_time_ms = u64::try_from(started.elapsed().as_millis()).map_err(|_| ())?;
    // Left unwritten when blank, so the converter can stream it unread.
    if !markdown.trim().is_empty() {
        std::fs::write(output_directory.join(ANYDOC_MARKDOWN_FILE), markdown).map_err(|_| ())?;
    }

    // A failed or panicking marking pass leaves the plain Markdown standing.
    let marked = (mark && matches!(format, anydoc::Format::Docx | anydoc::Format::Pptx))
        .then(|| panic::catch_unwind(|| stage_marked(bytes, format, output_directory)).ok())
        .flatten()
        .flatten();
    Ok(AnyDocReport::Converted {
        processing_time_ms,
        marked,
    })
}

/// Writes the marked Markdown, tokens unfilled, and each distinct picture
/// once. `None` when there is nothing to describe or anything goes wrong.
fn stage_marked(
    bytes: &[u8],
    format: anydoc::Format,
    output_directory: &Path,
) -> Option<MarkedPictures> {
    let (marked, pictures, placements, rendered) = marked_pictures(bytes, format)?;
    // The converter reads at most 1 MiB of report, and a placement takes up
    // to 25 bytes of it. Past this many the plain Markdown stands.
    if placements.len() > 40_000 {
        return None;
    }
    let markdown = anydoc::to_markdown_bytes(&marked, format).ok()?;
    std::fs::write(output_directory.join(ANYDOC_MARKED_FILE), markdown).ok()?;
    let directory = output_directory.join(ANYDOC_PICTURES_DIRECTORY);
    std::fs::create_dir(&directory).ok()?;
    // Two parts can still hold the same bytes, so those share one file.
    let mut files: HashMap<&[u8], String> = HashMap::new();
    let mut names = Vec::with_capacity(pictures.len());
    for (bytes, extension) in &pictures {
        if !files.contains_key(bytes.as_slice()) {
            let name = format!("{}.{extension}", files.len());
            std::fs::write(directory.join(&name), bytes).ok()?;
            files.insert(bytes, name);
        }
        names.push(files.get(bytes.as_slice())?.clone());
    }
    let placements = placements
        .into_iter()
        .map(|(n, index)| Some((n, names.get(index)?.clone())))
        .collect::<Option<_>>()?;
    Some(MarkedPictures {
        placements,
        rendered,
    })
}

fn anydoc_rejection(error: &anydoc::ConvertError) -> AnyDocRejection {
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

/// AnyDoc's own ceilings for one part and for all parts together, so the
/// marker pass reads nothing AnyDoc would refuse and a ZIP bomb costs no more
/// memory here than in AnyDoc.
const MAX_MARKED_PART_BYTES: u64 = 128 * 1024 * 1024;
const MAX_MARKED_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

/// A picture's bytes and its file extension.
type Picture = (Vec<u8>, &'static str);
/// A marker token's number and the index of its picture.
type Placement = (u32, usize);
/// The marked package, its pictures, their placements, and the token count.
type Marked = (Vec<u8>, Vec<Picture>, Vec<Placement>, usize);

/// Gives every picture with no alt text a `tkimg{n}tk` alt, so the renderer
/// prints the token where the picture sits, and returns the marked package,
/// each distinct PNG or JPEG once, where each token points, and the number of
/// tokens the Markdown will hold. `None` when there is nothing to describe or
/// anything goes wrong.
fn marked_pictures(bytes: &[u8], format: anydoc::Format) -> Option<Marked> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut next = 0;
    let mut left = MAX_MARKED_TOTAL_BYTES;
    for index in 0..archive.len() {
        let name = archive.name_for_index(index)?.to_owned();
        let open = match format {
            anydoc::Format::Docx if name == "word/document.xml" => "<wp:docPr",
            anydoc::Format::Pptx
                if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") =>
            {
                "<p:cNvPr"
            }
            _ => {
                out.raw_copy_file(archive.by_index_raw(index).ok()?).ok()?;
                continue;
            }
        };
        let mut xml = String::new();
        archive
            .by_index(index)
            .ok()?
            .take(MAX_MARKED_PART_BYTES.min(left))
            .read_to_string(&mut xml)
            .ok()?;
        left = left
            .checked_sub(xml.len() as u64)
            .filter(|left| *left > 0)?;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        out.start_file(name, options).ok()?;
        out.write_all(mark_tags(&xml, open, &mut next).as_bytes())
            .ok()?;
    }
    if next == 0 {
        return None;
    }
    let marked = out.finish().ok()?.into_inner();

    let mut document = anydoc::to_document(&marked, format).ok()?;
    let mut tokens = Vec::new();
    collect_tokens(&document.blocks, &mut tokens);
    for note in &document.notes {
        collect_tokens(&note.blocks, &mut tokens);
    }
    let rendered = tokens.len();
    // AnyDoc keeps one asset per package part, so a logo on 300 slides is one
    // asset under 300 tokens. Its bytes move out once, never per token.
    let mut pictures = Vec::new();
    let mut taken = HashMap::new();
    let placements: Vec<_> = tokens
        .into_iter()
        .filter_map(|(n, id)| {
            let id = id?;
            if let Some(&index) = taken.get(&id) {
                return Some((n, index));
            }
            let asset = document.assets.iter_mut().find(|asset| asset.id == id)?;
            let extension = match asset.media_type.as_str() {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                _ => return None,
            };
            pictures.push((std::mem::take(&mut asset.bytes), extension));
            taken.insert(id, pictures.len() - 1);
            Some((n, pictures.len() - 1))
        })
        .collect();
    (!placements.is_empty()).then_some((marked, pictures, placements, rendered))
}

/// Rewrites each `open` start tag whose `descr` is absent or empty to carry
/// `descr="tkimg{n}tk"`. Plain scanning: a tag it misreads yields XML AnyDoc
/// rejects, and the caller then falls back to the unmarked package.
fn mark_tags(xml: &str, open: &str, next: &mut u32) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find(open) {
        let (before, tag) = rest.split_at(start + open.len());
        out.push_str(before);
        let Some(end) = tag.find('>') else {
            rest = tag;
            break;
        };
        let (attributes, after) = tag.split_at(end);
        rest = after;
        if !attributes.is_empty() && !attributes.starts_with([' ', '\t', '\r', '\n', '/']) {
            out.push_str(attributes);
            continue;
        }
        let token = format!(" descr=\"tkimg{next}tk\"");
        match attributes.split_once(" descr=") {
            // A `descr` this scan cannot read stays as written.
            None if attributes.contains("descr") => {
                out.push_str(attributes);
                continue;
            }
            None => {
                out.push_str(&token);
                out.push_str(attributes);
            }
            Some((head, value)) => {
                let mut chars = value.chars();
                let quoted = chars
                    .next()
                    .and_then(|quote| chars.as_str().split_once(quote));
                match quoted {
                    // Office writes its own guess as alt text and signs it
                    // with this line. Only author alt text stays as written.
                    Some((text, tail))
                        if text.trim().is_empty()
                            || text.contains("Description automatically generated") =>
                    {
                        out.push_str(head);
                        out.push_str(&token);
                        out.push_str(tail);
                    }
                    _ => {
                        out.push_str(attributes);
                        continue;
                    }
                }
            }
        }
        *next += 1;
    }
    out.push_str(rest);
    out
}

type Token = (u32, Option<anydoc::model::AssetId>);

fn collect_tokens(blocks: &[Block], tokens: &mut Vec<Token>) {
    for block in blocks {
        match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => {
                collect_inline_tokens(content, tokens);
            }
            Block::List(list) => {
                for item in &list.items {
                    collect_tokens(&item.blocks, tokens);
                }
            }
            Block::Table(table) => {
                for slot in table.grid.iter().flatten() {
                    if let CellSlot::Origin(cell) = slot {
                        collect_tokens(&cell.blocks, tokens);
                    }
                }
            }
            Block::BlockQuote(blocks) => collect_tokens(blocks, tokens),
            Block::CodeBlock { .. } | Block::Math(_) | Block::Rule => {}
        }
    }
}

fn collect_inline_tokens(inlines: &[Inline], tokens: &mut Vec<Token>) {
    for inline in inlines {
        match inline {
            Inline::Image { alt, source } => {
                let n = alt
                    .strip_prefix("tkimg")
                    .and_then(|tail| tail.strip_suffix("tk"))
                    .and_then(|n| n.parse().ok());
                if let Some(n) = n {
                    let id = match source {
                        ImageSource::Asset(id) => Some(*id),
                        _ => None,
                    };
                    tokens.push((n, id));
                }
            }
            Inline::Link { content, .. } => collect_inline_tokens(content, tokens),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};

    use sha2::{Digest, Sha256};
    use tool_kit_converter::worker_protocol::{
        EngineIdentity, PDF_INSPECTOR_VERSION, WORKER_PROTOCOL_VERSION,
    };
    use tool_kit_worker_protocol::anydoc::{
        AnyDocRejection, AnyDocReport, ANYDOC_MARKDOWN_FILE, ANYDOC_MARKED_FILE,
        ANYDOC_PICTURES_DIRECTORY,
    };

    use super::{convert_anydoc, copy_exact_source, mark_tags, marked_pictures};

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

        assert_eq!(WORKER_PROTOCOL_VERSION, 3);
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

    #[test]
    fn markers_go_on_pictures_without_alt_text() {
        let mut next = 0;
        let xml = r#"<p:cNvPr id="1"/><p:cNvPr id="2" descr=""/><p:cNvPr id="3" descr="Kept"/><p:cNvPr id="4" descr="A cat&#xA;&#xA;Description automatically generated"/><p:cNvPr id="5"	descr="Tabbed"/><p:cNvPicPr/>"#;
        assert_eq!(
            mark_tags(xml, "<p:cNvPr", &mut next),
            r#"<p:cNvPr descr="tkimg0tk" id="1"/><p:cNvPr id="2" descr="tkimg1tk"/><p:cNvPr id="3" descr="Kept"/><p:cNvPr id="4" descr="tkimg2tk"/><p:cNvPr id="5"	descr="Tabbed"/><p:cNvPicPr/>"#
        );
        assert_eq!(next, 3);
    }

    /// A logo on every slide is one part under many tokens. Its bytes were
    /// copied per token, so a 20 MB photo on 300 slides held 6 GB.
    #[test]
    fn a_picture_placed_many_times_is_held_once() {
        let fixture = include_bytes!("../../tests/fixtures/anydoc/text.docx");
        let mut archive = zip::ZipArchive::new(Cursor::new(&fixture[..])).unwrap();
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            let name = file.name().to_owned();
            let mut content = Vec::new();
            file.read_to_end(&mut content).unwrap();
            if name == "word/document.xml" {
                // Drop the alt text so the picture is marked, then place it
                // three times.
                let xml = String::from_utf8(content)
                    .unwrap()
                    .replace(" descr=\"tiny red dot\"></wp:docPr>", "></wp:docPr>");
                let (head, rest) = xml.split_once("<w:drawing>").unwrap();
                let (body, tail) = rest.split_once("</w:drawing>").unwrap();
                let drawing = format!("<w:drawing>{body}</w:drawing>");
                content = format!("{head}{}{tail}", drawing.repeat(3)).into_bytes();
            }
            out.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            out.write_all(&content).unwrap();
        }
        let docx = out.finish().unwrap().into_inner();

        let (_, pictures, placements, rendered) =
            marked_pictures(&docx, anydoc::Format::Docx).unwrap();
        assert_eq!(rendered, 3);
        assert_eq!(placements.len(), 3);
        assert_eq!(pictures.len(), 1);
        assert!(placements.iter().all(|(_, index)| *index == 0));

        // The worker stages that one picture once, for all three tokens.
        let directory = tempfile::tempdir().unwrap();
        let AnyDocReport::Converted {
            marked: Some(marked),
            ..
        } = convert_anydoc(&docx, "docx", true, directory.path()).unwrap()
        else {
            panic!("the picture must be marked");
        };
        assert_eq!(marked.rendered, 3);
        assert_eq!(
            marked.placements,
            [0, 1, 2].map(|n| (n, "0.png".to_owned()))
        );
        let pictures = directory.path().join(ANYDOC_PICTURES_DIRECTORY);
        assert_eq!(std::fs::read_dir(pictures).unwrap().count(), 1);
        let marked_markdown =
            std::fs::read_to_string(directory.path().join(ANYDOC_MARKED_FILE)).unwrap();
        assert!(marked_markdown.contains("tkimg2tk"), "{marked_markdown}");
    }

    #[test]
    fn anydoc_refuses_pdf_bytes_and_converts_rtf() {
        let directory = tempfile::tempdir().unwrap();
        let refused = convert_anydoc(b"%PDF-1.4\nbody", "rtf", true, directory.path()).unwrap();
        assert!(matches!(
            refused,
            AnyDocReport::Rejected {
                code: AnyDocRejection::UnsupportedDocument
            }
        ));
        assert!(!directory.path().join(ANYDOC_MARKDOWN_FILE).exists());

        let converted = convert_anydoc(
            br#"{\rtf1\ansi Hello from AnyDoc.}"#,
            "rtf",
            true,
            directory.path(),
        )
        .unwrap();
        assert!(matches!(
            converted,
            AnyDocReport::Converted { marked: None, .. }
        ));
        let markdown =
            std::fs::read_to_string(directory.path().join(ANYDOC_MARKDOWN_FILE)).unwrap();
        assert!(markdown.contains("Hello from AnyDoc."));
    }
}
