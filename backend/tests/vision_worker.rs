#![allow(clippy::unwrap_used)]

//! Grades the Swift Vision worker's protocol, not its OCR quality.
//!
//! Every test returns early when the worker binary is missing. It is built by
//! `backend/vision-worker/build.sh`, which refuses to run off macOS, and `bin/`
//! is gitignored, so Linux CI and a fresh checkout both reach this file with
//! nothing to spawn. A skip there is the honest answer; a failure would only
//! say the platform is not macOS.

use std::{
    fs::{self, File},
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

use sha2::{Digest, Sha256};

use tool_kit_converter::vision_protocol::{
    VisionOutcome, VisionRejectionCode, VisionReport, VISION_ENGINE_NAME, VISION_MARKDOWN_FILE,
    VISION_WORKER_CUSTOM_WORDS_ENV, VISION_WORKER_EXPECTED_SOURCE_BYTES_ENV,
    VISION_WORKER_EXPECTED_SOURCE_SHA256_ENV, VISION_WORKER_IDENTITY_PREFIX,
    VISION_WORKER_LANGUAGE_CORRECTION_ENV, VISION_WORKER_MAX_OUTPUT_BYTES_ENV,
    VISION_WORKER_PROTOCOL_VERSION, VISION_WORKER_REPORT_FILE,
};

const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

fn tool(name: &str) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("vision-worker/bin")
        .join(name);
    path.is_file().then_some(path)
}

/// The worker reports Foundation's three components; `sw_vers` drops a
/// trailing zero, so pad before comparing.
fn macos_product_version() -> String {
    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .unwrap();
    let printed = String::from_utf8(output.stdout).unwrap();
    let mut parts: Vec<&str> = printed.trim().split('.').collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    parts.join(".")
}

struct Run {
    staging: PathBuf,
    status: ExitStatus,
}

/// Spawns the worker exactly as `engines::pdf_inspector` spawns the PDF worker:
/// the staging directory as argv[1], the attempt directory as the cwd, a
/// cleared environment, the source on stdin, and both output streams on
/// /dev/null. Every test runs through it, so the cleared environment is what
/// every assertion below is made under.
fn run_worker(
    worker: &Path,
    root: &Path,
    source: &Path,
    sha256: &str,
    settings: &[(&str, &str)],
) -> Run {
    let attempt = root.join("attempt");
    let staging = attempt.join("publication.staging");
    fs::create_dir_all(&staging).unwrap();
    let byte_length = fs::metadata(source).unwrap().len();

    let mut command = Command::new(worker);
    command
        .arg(&staging)
        .current_dir(&attempt)
        .env_clear()
        .env(
            VISION_WORKER_MAX_OUTPUT_BYTES_ENV,
            MAX_OUTPUT_BYTES.to_string(),
        )
        .env(
            VISION_WORKER_EXPECTED_SOURCE_BYTES_ENV,
            byte_length.to_string(),
        )
        .env(VISION_WORKER_EXPECTED_SOURCE_SHA256_ENV, sha256)
        .stdin(Stdio::from(File::open(source).unwrap()))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (name, value) in settings {
        command.env(name, value);
    }
    let status = command.status().unwrap();
    Run { staging, status }
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Rasterizes a generated PDF with the `pdf2png` tool beside the worker, so the
/// PNG carries real rendered glyphs rather than bytes a hand-written encoder
/// guessed at.
fn png(directory: &Path, name: &str, content: &[u8]) -> PathBuf {
    let pdf2png = tool("pdf2png").expect("pdf2png is built alongside the worker");
    let pdf_path = directory.join(format!("{name}.pdf"));
    fs::write(&pdf_path, one_page_pdf(content)).unwrap();
    let png_path = directory.join(format!("{name}.png"));
    let status = Command::new(pdf2png)
        .arg(&pdf_path)
        .arg(&png_path)
        .arg("200")
        .status()
        .unwrap();
    assert!(status.success(), "pdf2png failed on {name}");
    png_path
}

/// Two rendered pages in one TIFF, built with the system tools `sips` and
/// `tiffutil` so the frames are encoded frames rather than a hand-written
/// header's claim about them.
fn two_page_tiff(directory: &Path) -> PathBuf {
    let mut pages = Vec::new();
    for (name, line) in [("one", "Page One Marker"), ("two", "Page Two Marker")] {
        let source = png(directory, name, &helvetica_lines(&[line]));
        let page = directory.join(format!("{name}.tiff"));
        let status = Command::new("/usr/bin/sips")
            .args(["-s", "format", "tiff"])
            .arg(&source)
            .arg("--out")
            .arg(&page)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "sips failed on {name}");
        pages.push(page);
    }
    let combined = directory.join("two-pages.tiff");
    let status = Command::new("/usr/bin/tiffutil")
        .arg("-cat")
        .args(&pages)
        .arg("-out")
        .arg(&combined)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "tiffutil failed");
    combined
}

fn helvetica_lines(lines: &[&str]) -> Vec<u8> {
    let mut content = b"BT\n/F1 24 Tf\n".to_vec();
    for (index, line) in lines.iter().enumerate() {
        let y = 700 - index * 40;
        writeln!(content, "1 0 0 1 72 {y} Tm ({line}) Tj").unwrap();
    }
    content.extend_from_slice(b"ET\n");
    content
}

fn one_page_pdf(content: &[u8]) -> Vec<u8> {
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
           /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream,
    ];

    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        writeln!(pdf, "{} 0 obj", index + 1).unwrap();
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    write!(pdf, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
    for offset in offsets {
        writeln!(pdf, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    pdf
}

fn read_report(staging: &Path) -> VisionReport {
    let encoded = fs::read(staging.join(VISION_WORKER_REPORT_FILE)).unwrap();
    serde_json::from_slice(&encoded).unwrap()
}

#[test]
fn the_handshake_line_is_the_identity_the_contract_declares() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };

    let output = Command::new(&worker)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "{VISION_WORKER_IDENTITY_PREFIX}{}\n",
            macos_product_version()
        )
    );
}

/// Also the cleared-environment case: the worker gets no PATH, HOME, or TMPDIR
/// and still loads Vision, reads stdin, and publishes into the staging
/// directory.
#[test]
fn a_page_of_text_converts_and_the_artifact_matches_what_landed_on_disk() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = png(
        directory.path(),
        "text",
        &helvetica_lines(&[
            "Quarterly Report",
            "The quick brown fox jumps over the lazy dog.",
            "Revenue grew in every region this quarter.",
        ]),
    );
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &worker,
        directory.path(),
        &source,
        &sha256,
        &[
            (VISION_WORKER_LANGUAGE_CORRECTION_ENV, "1"),
            (VISION_WORKER_CUSTOM_WORDS_ENV, "Uniwise\nTool-Kit"),
        ],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let report = read_report(&run.staging);
    assert_eq!(report.protocol_version, VISION_WORKER_PROTOCOL_VERSION);
    assert_eq!(report.engine.name, VISION_ENGINE_NAME);
    assert_eq!(report.engine.version, macos_product_version());
    assert!(report.engine.features.is_empty());

    let VisionOutcome::Converted { artifact } = report.outcome else {
        panic!("a page of rendered text should convert");
    };
    assert_eq!(artifact.relative_path, VISION_MARKDOWN_FILE);
    let markdown = fs::read(run.staging.join(VISION_MARKDOWN_FILE)).unwrap();
    assert!(!markdown.is_empty());
    assert_eq!(artifact.byte_length, markdown.len() as u64);
    assert_eq!(artifact.sha256, digest(&markdown));
}

/// Vision reads a page of evenly spaced identical lines as a wide table with
/// almost every column empty. Publishing that puts the page's prose inside a
/// pipe grid nothing reads. `prune` in render.swift drops columns that are
/// empty in every row, and what survives here is one column, which is prose.
#[test]
fn evenly_spaced_lines_publish_as_prose_rather_than_a_false_table() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let lines: Vec<String> = (0..12)
        .map(|n| format!("Sheet 0 line {n}: the quick brown fox jumps over the lazy dog."))
        .collect();
    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
    let source = png(directory.path(), "grid", &helvetica_lines(&borrowed));
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &worker,
        directory.path(),
        &source,
        &sha256,
        &[(VISION_WORKER_LANGUAGE_CORRECTION_ENV, "1")],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let markdown = fs::read_to_string(run.staging.join(VISION_MARKDOWN_FILE)).unwrap();
    assert!(
        !markdown.lines().any(|line| line.starts_with('|')),
        "no line may open a pipe table, got:\n{markdown}"
    );
    assert!(
        markdown.contains("quick brown fox"),
        "dropping the false table must not drop the text, got:\n{markdown}"
    );
}

#[test]
fn a_source_digest_that_does_not_match_fails_the_run_and_publishes_nothing() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = png(
        directory.path(),
        "text",
        &helvetica_lines(&["Annual Report"]),
    );

    let run = run_worker(
        &worker,
        directory.path(),
        &source,
        &digest(b"some other bytes"),
        &[],
    );

    assert_eq!(run.status.code(), Some(70));
    assert!(!run.staging.join(VISION_WORKER_REPORT_FILE).exists());
    assert!(!run.staging.join(VISION_MARKDOWN_FILE).exists());
}

#[test]
fn an_image_carrying_no_text_is_rejected_rather_than_converted_empty() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    // A page that draws nothing: no glyphs for Vision to find.
    let source = png(directory.path(), "blank", b"q\nQ\n");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(&worker, directory.path(), &source, &sha256, &[]);

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let report = read_report(&run.staging);
    let VisionOutcome::Rejected { code } = report.outcome else {
        panic!("a blank page should not convert");
    };
    assert_eq!(code, VisionRejectionCode::NoTextFound);
    assert!(!run.staging.join(VISION_MARKDOWN_FILE).exists());
}

/// Only the first frame is ever read, so publishing it would hand back page one
/// of a scanned document marked as the whole of it.
#[test]
fn a_source_holding_more_than_one_frame_is_rejected_rather_than_truncated() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = two_page_tiff(directory.path());
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(&worker, directory.path(), &source, &sha256, &[]);

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let VisionOutcome::Rejected { code } = read_report(&run.staging).outcome else {
        panic!("a two page TIFF converts to half a document or to nothing");
    };
    assert_eq!(code, VisionRejectionCode::MultiFrameImage);
    assert!(!run.staging.join(VISION_MARKDOWN_FILE).exists());
}

#[test]
fn bytes_no_decoder_can_read_are_rejected_as_an_invalid_image() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("not-an-image.png");
    fs::write(&source, b"this is not an image\n").unwrap();
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(&worker, directory.path(), &source, &sha256, &[]);

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let VisionOutcome::Rejected { code } = read_report(&run.staging).outcome else {
        panic!("text bytes are not an image");
    };
    assert_eq!(code, VisionRejectionCode::InvalidImage);
    assert!(!run.staging.join(VISION_MARKDOWN_FILE).exists());
}

#[test]
fn markdown_over_the_ceiling_is_rejected_and_nothing_is_published() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = png(
        directory.path(),
        "text",
        &helvetica_lines(&["Annual Report"]),
    );
    let sha256 = digest(&fs::read(&source).unwrap());

    // A settings entry replaces the base environment, which is how the ceiling
    // drops below anything Vision could return.
    let run = run_worker(
        &worker,
        directory.path(),
        &source,
        &sha256,
        &[(VISION_WORKER_MAX_OUTPUT_BYTES_ENV, "1")],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let VisionOutcome::Rejected { code } = read_report(&run.staging).outcome else {
        panic!("a one byte ceiling cannot hold a page of Markdown");
    };
    assert_eq!(code, VisionRejectionCode::OutputTooLarge);
    assert!(!run.staging.join(VISION_MARKDOWN_FILE).exists());
}
