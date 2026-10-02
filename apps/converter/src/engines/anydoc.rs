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

use std::{
    collections::HashMap,
    io::{Cursor, Read, Write},
    panic,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use anydoc::model::{Block, CellSlot, ImageSource, Inline};

use sha2::{Digest, Sha256};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{watch, OwnedSemaphorePermit, Semaphore},
};

use super::child::{self, is_lowercase_sha256, wait_for_child};
use super::{EngineAnalysis, EngineFailure, EngineOutcome, EngineRejection, QualitySignals};
use crate::{
    artifacts::{AttemptPaths, ValidatedOpenFile},
    persistence::DocumentClassification,
};

pub(crate) const ANYDOC_ENGINE_NAME: &str = "anydoc";
pub(crate) const ANYDOC_VERSION: &str = "0.2.4";

/// The admission label of the one format detection cannot see.
const CSV_FORMAT_LABEL: &str = "csv";

/// Engine-specific detail persisted as attempt diagnostics and embedded in
/// the manifest. Content-free: the detected format family and wall time only.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AnyDocDiagnostics {
    pub format: String,
    pub processing_time_ms: u64,
}
#[derive(Clone, Debug)]
pub struct AnyDocEngine {
    max_output_bytes: u64,
    timeout: Duration,
    permits: Arc<Semaphore>,
    /// The Vision worker, which also describes DOCX and PPTX pictures.
    describer: Option<PathBuf>,
}

impl AnyDocEngine {
    pub fn new(max_output_bytes: u64, timeout: Duration, describer: Option<PathBuf>) -> Self {
        Self {
            max_output_bytes,
            timeout,
            permits: Arc::new(Semaphore::new(1)),
            describer,
        }
    }

    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, EngineFailure> {
        child::acquire(&self.permits).await
    }

    /// Runs blocking work on a blocking thread under a hard deadline. A hang
    /// becomes `Timeout` and a panic becomes `Crashed`; the worker loop never
    /// waits on the parse again. A timed-out task keeps running detached,
    /// bounded by AnyDoc's internal resource limits.
    async fn run_bounded<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, EngineFailure> {
        let handle = tokio::task::spawn_blocking(work);
        match tokio::time::timeout(self.timeout, handle).await {
            Err(_) => Err(EngineFailure::Timeout),
            Ok(join) => join.map_err(|_| EngineFailure::Crashed),
        }
    }

    pub async fn convert(
        &self,
        paths: &AttemptPaths,
        source: ValidatedOpenFile,
        permit: OwnedSemaphorePermit,
        cancellation: watch::Receiver<bool>,
        admitted_label: &str,
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

        let detected = anydoc::Format::from_bytes(&bytes);
        if detected == Some(anydoc::Format::Pdf) {
            return Ok(rejected(AnyDocRejection::UnsupportedDocument));
        }
        // CSV is the one signature-less format: detection always returns
        // `None`, so the admitted extension names it. Every other format is
        // content-detected, and content that detects as nothing is corrupt or
        // mislabeled, not "unsupported".
        let detected = detected
            .or_else(|| (admitted_label == CSV_FORMAT_LABEL).then_some(anydoc::Format::Csv));
        let Some(format) = detected else {
            return Ok(rejected(AnyDocRejection::InvalidDocument));
        };
        // docx, xlsx, pptx and epub share one ZIP magic (doc/xls/ppt one OLE
        // magic), so admission cannot tell them apart. Catch the cross-family
        // mislabel here, before parsing: otherwise the file converts fine and
        // the manifest check rejects it afterwards as `artifact_integrity_failed`,
        // a corruption code, for what is only a misnamed upload.
        if format_label(format) != admitted_label {
            return Ok(rejected(AnyDocRejection::InvalidDocument));
        }

        // Pictures with no alt text get a description where the model is
        // available. A failed run leaves today's output untouched. Once one
        // picture is described, a picture the model skipped also loses
        // Office's own "Description automatically generated" guess.
        // ponytail: a timeout in the marking parse still fails the job, since
        // the detached task keeps the permit. Merge both parses into one
        // bounded closure if a document ever parses in time but marks too slowly.
        let mut described = None;
        let mut permit = permit;
        if let (Some(worker), anydoc::Format::Docx | anydoc::Format::Pptx) =
            (self.describer.as_deref(), format)
        {
            let (returned, original, prepared) = self
                .run_bounded(move || {
                    let prepared = panic::catch_unwind(|| marked_pictures(&bytes, format))
                        .ok()
                        .flatten();
                    (permit, bytes, prepared)
                })
                .await?;
            (permit, bytes) = (returned, original);
            if let Some((marked, pictures, placements, rendered)) = prepared {
                let found = describe(worker, &pictures, &placements, cancellation.clone()).await;
                if !found.is_empty() {
                    described = Some((marked, found, rendered));
                }
            }
        }

        let conversion = self
            .run_bounded(move || {
                // Held inside the blocking closure, not by `convert`. On
                // timeout the task detaches and keeps parsing, so releasing
                // the permit when this function returns would let the next
                // job parse alongside it and break the one-parse-per-engine
                // invariant the epic states.
                let _permit = permit;
                let started = Instant::now();
                let filled = described.and_then(|(marked, found, rendered)| {
                    let markdown =
                        panic::catch_unwind(|| anydoc::to_markdown_bytes(&marked, format)).ok()?;
                    fill_descriptions(&markdown.ok()?, &found, rendered)
                });
                let result = match filled {
                    Some(markdown) => Ok(Ok(markdown)),
                    None => panic::catch_unwind(panic::AssertUnwindSafe(|| {
                        anydoc::to_markdown_bytes(&bytes, format)
                    })),
                };
                (result, started.elapsed())
            })
            .await?;
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
        if output_bytes > self.max_output_bytes {
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
                // AnyDoc reports no completeness measure. Its own part-level
                // "skip a broken piece and continue" recovery is silent, so
                // claiming a measurement here would be inventing one.
                quality: QualitySignals::unmeasured(),
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

/// Runs the Vision worker once over every distinct picture and returns the
/// descriptions it wrote by token. A decorative picture, a per-picture error,
/// or no model leaves a token out, and a failed run returns what it finished.
async fn describe(
    worker: &Path,
    pictures: &[Picture],
    placements: &[Placement],
    cancellation: watch::Receiver<bool>,
) -> HashMap<u32, String> {
    let mut found = HashMap::new();
    let Ok(directory) = tempfile::tempdir() else {
        return found;
    };
    // Two parts can still hold the same bytes, so those share one file.
    let mut files: HashMap<&[u8], String> = HashMap::new();
    let mut names = Vec::with_capacity(pictures.len());
    for (bytes, extension) in pictures {
        if !files.contains_key(bytes.as_slice()) {
            let name = format!("{}.{extension}", files.len());
            if tokio::fs::write(directory.path().join(&name), bytes)
                .await
                .is_err()
            {
                return found;
            }
            files.insert(bytes, name);
        }
        names.push(files.get(bytes.as_slice()).cloned());
    }
    let Ok(mut child) = Command::new(worker)
        .arg("--describe")
        .arg(directory.path())
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
    else {
        return found;
    };
    // ponytail: about 5 s per picture after a cold model load, measured on
    // one Mac. A huge deck waits that long, cancellation still stops it.
    let budget = Duration::from_secs(20 + 5 * files.len() as u64);
    let _ = wait_for_child(&mut child, budget, cancellation).await;
    for (n, index) in placements {
        let Some(Some(name)) = names.get(*index) else {
            continue;
        };
        let path = directory.path().join(format!("{name}.txt"));
        let Ok(text) = tokio::fs::read_to_string(path).await else {
            continue;
        };
        let text = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(['*', '|', '\\', '`', '_', '~', '[', ']', '<', '>'], "");
        if !text.is_empty() {
            found.insert(*n, text);
        }
    }
    found
}

/// Replaces each `tkimg{n}tk` with `*Image: <description>*`, or with nothing
/// when there is none. A token alone on its line takes its blank line with it.
/// `None` when the Markdown holds more tokens than the pictures put there,
/// because then the document's own text spells one.
fn fill_descriptions(
    markdown: &str,
    found: &HashMap<u32, String>,
    expected: usize,
) -> Option<String> {
    let mut seen = 0;
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("tkimg") {
        let (before, tail) = rest.split_at(start);
        let tail = tail.strip_prefix("tkimg").unwrap_or(tail);
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let (number, after) = tail.split_at(digits);
        let Some(after) = after.strip_prefix("tk").filter(|_| digits > 0) else {
            out.push_str(before);
            out.push_str("tkimg");
            rest = tail;
            continue;
        };
        out.push_str(before);
        rest = after;
        seen += 1;
        match number.parse().ok().and_then(|n: u32| found.get(&n)) {
            Some(text) => {
                out.push_str("*Image: ");
                out.push_str(text);
                out.push('*');
            }
            None if out.is_empty() || out.ends_with("\n\n") => {
                rest = rest.trim_start_matches('\n');
            }
            None => {}
        }
    }
    out.push_str(rest);
    (seen == expected).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest and capabilities report this constant, so a dependency
    /// bump that leaves it behind would publish the wrong engine version.
    #[test]
    fn the_pinned_version_matches_the_crate() {
        assert!(
            include_str!("../../Cargo.toml").contains(&format!("anydoc = \"={ANYDOC_VERSION}\""))
        );
    }

    #[test]
    fn markers_go_on_pictures_without_alt_text_and_come_out_filled() {
        let mut next = 0;
        let xml = r#"<p:cNvPr id="1"/><p:cNvPr id="2" descr=""/><p:cNvPr id="3" descr="Kept"/><p:cNvPr id="4" descr="A cat&#xA;&#xA;Description automatically generated"/><p:cNvPr id="5"	descr="Tabbed"/><p:cNvPicPr/>"#;
        assert_eq!(
            mark_tags(xml, "<p:cNvPr", &mut next),
            r#"<p:cNvPr descr="tkimg0tk" id="1"/><p:cNvPr id="2" descr="tkimg1tk"/><p:cNvPr id="3" descr="Kept"/><p:cNvPr id="4" descr="tkimg2tk"/><p:cNvPr id="5"	descr="Tabbed"/><p:cNvPicPr/>"#
        );
        assert_eq!(next, 3);

        let found = HashMap::from([(0, "A grid.".to_owned())]);
        assert_eq!(
            fill_descriptions(
                "Intro\n\ntkimg0tk\n\ntkimg1tk\n\nEnd tkimg7tk.\n",
                &found,
                3
            )
            .as_deref(),
            Some("Intro\n\n*Image: A grid.*\n\nEnd .\n")
        );
        // Body text that spells a token makes one too many: today's output.
        assert_eq!(fill_descriptions("tkimg0tk tkimg0tk\n", &found, 1), None);
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

        let engine = AnyDocEngine::new(1024 * 1024, Duration::from_secs(30), None);
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let outcome = engine
            .convert(&paths, source, permit, cancellation, "rtf")
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

        let engine = AnyDocEngine::new(1024 * 1024, Duration::from_secs(30), None);
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let outcome = engine
            .convert(&paths, source, permit, cancellation, "rtf")
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

    /// The timeout frees the runner, not the engine. `convert` holds the
    /// permit inside the blocking closure, so a detached parse keeps it and
    /// two parses still cannot overlap. That is why `execute_claimed` bounds
    /// its own permit wait rather than blocking forever.
    #[tokio::test]
    async fn a_timed_out_parse_keeps_its_permit_until_it_finishes() {
        let engine = AnyDocEngine::new(1024, Duration::from_millis(50), None);
        let permit = engine.acquire().await.unwrap();
        let hung = engine
            .run_bounded(move || {
                let _permit = permit;
                std::thread::sleep(Duration::from_millis(400));
            })
            .await;
        assert_eq!(hung.unwrap_err(), EngineFailure::Timeout);

        assert!(
            tokio::time::timeout(Duration::from_millis(50), engine.acquire())
                .await
                .is_err(),
            "a detached parse must still hold the only permit"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), engine.acquire())
                .await
                .is_ok(),
            "the permit must come back when the parse ends"
        );
    }

    #[tokio::test]
    async fn a_panicking_conversion_is_a_crash_not_a_wedge() {
        let engine = AnyDocEngine::new(1024, Duration::from_secs(5), None);
        let result: Result<(), EngineFailure> =
            engine.run_bounded(|| panic!("deliberate test panic")).await;
        assert_eq!(result.unwrap_err(), EngineFailure::Crashed);
    }
}
