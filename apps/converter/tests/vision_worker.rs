#![allow(clippy::unwrap_used)]

//! Grades the Swift Vision worker's protocol, not its OCR quality.
//!
//! Every test returns early when the worker binary is missing. It is built by
//! `workers/vision/build.sh`, which refuses to run off macOS, and `bin/`
//! is gitignored, so Linux CI and a fresh checkout both reach this file with
//! nothing to spawn. A skip there is the honest answer; a failure would only
//! say the platform is not macOS.

mod support;

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use axum::http::StatusCode;
use http_body_util::BodyExt;
use serde_json::Value;
use uuid::Uuid;

use support::{corpus, digest, macos_product_version, pdf_with_content, read_report, run_worker};
use tool_kit_converter::vision_protocol::{
    VisionOutcome, VisionPages, VisionRejectionCode, VisionReport, VISION_ENGINE_NAME,
    VISION_MARKDOWN_FILE, VISION_WORKER_CUSTOM_WORDS_ENV, VISION_WORKER_IDENTITY_PREFIX,
    VISION_WORKER_LANGUAGE_CORRECTION_ENV, VISION_WORKER_MAX_OUTPUT_BYTES_ENV,
    VISION_WORKER_PROTOCOL_VERSION, VISION_WORKER_REPORT_FILE,
};

fn tool(name: &str) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    // The worker is not part of this crate, so this climbs out of
    // apps/converter/ to the repo root to reach it. A wrong path here skips
    // every test in the file silently, which is the same thing a missing
    // binary means.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../workers/vision/bin")
        .join(name);
    path.is_file().then_some(path)
}

/// Rasterizes a generated PDF with the `pdf2png` tool beside the worker, so the
/// PNG carries real rendered glyphs rather than bytes a hand-written encoder
/// guessed at.
fn png(directory: &Path, name: &str, content: &[u8]) -> PathBuf {
    let pdf2png = tool("pdf2png").expect("pdf2png is built alongside the worker");
    let pdf_path = directory.join(format!("{name}.pdf"));
    fs::write(&pdf_path, pdf_with_content(content)).unwrap();
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

/// Two pages that are only a JPEG of their text, the shape of a scanner's
/// output: pdf-inspector finds no text on either page, so every page needs OCR.
fn scanned_pdf(directory: &Path, lines: [&str; 2]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
    ];
    for page in 0..2 {
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                 /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
                5 + page,
                7 + page
            )
            .into_bytes(),
        );
    }
    for (page, line) in lines.iter().enumerate() {
        let samples = jpeg_of(
            directory,
            &format!("scan-{page}"),
            &helvetica_lines(&[line]),
            &[],
        );
        // pdf2png renders 612x792 points at 200 dpi.
        objects.push(image_object(&samples, 1700, 2200));
    }
    for _ in 0..2 {
        objects.push(corpus::stream_object("", b"q 612 0 0 792 0 0 cm /Im0 Do Q"));
    }
    corpus::assemble(&objects)
}

/// `content` rendered by pdf2png and saved as a JPEG, `sips` arguments first.
fn jpeg_of(directory: &Path, name: &str, content: &[u8], sips: &[&str]) -> Vec<u8> {
    let png = png(directory, name, content);
    let jpeg = directory.join(format!("{name}.jpg"));
    let status = Command::new("/usr/bin/sips")
        .args(sips)
        .args(["-s", "format", "jpeg"])
        .arg(&png)
        .arg("--out")
        .arg(&jpeg)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "sips failed on {name}");
    fs::read(jpeg).unwrap()
}

fn image_object(samples: &[u8], width: u32, height: u32) -> Vec<u8> {
    corpus::stream_object(
        &format!(
            "/Type /XObject /Subtype /Image /Width {width} /Height {height} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"
        ),
        samples,
    )
}

/// One scanned page stored sideways with `/Rotate 90`, the way a scanner fixes
/// a landscape feed. The short line sits above the long one, and a sideways
/// read sorts the long line first.
fn rotated_scan(directory: &Path) -> Vec<u8> {
    // sips turns clockwise, so 270 lays the page on its left side.
    let samples = jpeg_of(
        directory,
        "sideways",
        b"BT\n/F1 24 Tf\n1 0 0 1 72 700 Tm (Opening) Tj\n\
          1 0 0 1 72 300 Tm (The closing line runs far longer than the opening) Tj\nET\n",
        &["-r", "270"],
    );
    corpus::assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 792 612] /Rotate 90 \
           /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>"
            .to_vec(),
        image_object(&samples, 2200, 1700),
        corpus::stream_object("", b"q 792 0 0 612 0 0 cm /Im0 Do Q"),
    ])
}

/// A native text page, then a page that is only a JPEG of its text: the shape
/// of a statement with a scanned cover. Only the second page needs OCR.
fn mixed_pdf(directory: &Path) -> Vec<u8> {
    let samples = jpeg_of(
        directory,
        "scanned",
        &helvetica_lines(&["Scanned Page Two"]),
        &[],
    );
    corpus::assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
           /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>"
            .to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
           /Resources << /XObject << /Im0 7 0 R >> >> /Contents 8 0 R >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        corpus::stream_object(
            "",
            &helvetica_lines(&[
                "Native Page One",
                "Balance 99,000.00 on the closing date.",
                "Every figure here is exact native text.",
            ]),
        ),
        image_object(&samples, 1700, 2200),
        corpus::stream_object("", b"q 612 0 0 792 0 0 cm /Im0 Do Q"),
    ])
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
    let report = read_report::<VisionReport>(&run.staging);
    assert_eq!(report.protocol_version, VISION_WORKER_PROTOCOL_VERSION);
    assert_eq!(report.engine.name, VISION_ENGINE_NAME);
    assert_eq!(report.engine.version, macos_product_version());
    assert!(report.engine.features.is_empty());

    let VisionOutcome::Converted { artifact, pages } = report.outcome else {
        panic!("a page of rendered text should convert");
    };
    assert_eq!(artifact.relative_path, VISION_MARKDOWN_FILE);
    let markdown = fs::read(run.staging.join(VISION_MARKDOWN_FILE)).unwrap();
    assert!(!markdown.is_empty());
    assert_eq!(artifact.byte_length, markdown.len() as u64);
    assert_eq!(artifact.sha256, digest(&markdown));
    assert_eq!(pages, None, "a single image has no pages");
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
    let report = read_report::<VisionReport>(&run.staging);
    let VisionOutcome::Rejected { code } = report.outcome else {
        panic!("a blank page should not convert");
    };
    assert_eq!(code, VisionRejectionCode::NoTextFound);
    assert!(!run.staging.join(VISION_MARKDOWN_FILE).exists());
}

/// The contrast case to the blank page above, and the reason it needs its own
/// rasterizer. `png` goes through `pdf2png`, which fills white, so every other
/// PNG in this file is opaque and not one of them can catch this. `sips`
/// renders the same page straight off the PDF and leaves the background
/// transparent, which is what an image exported from a design tool or dragged
/// out of a PDF viewer actually carries.
///
/// Vision reads composited pixels. Dark glyphs on a transparent background
/// composite to nothing, so an unflattened source comes back `no_text_found`,
/// which is indistinguishable downstream from a genuinely blank scan and
/// spends the remote fallback on a file that was always readable.
#[test]
fn a_transparent_background_is_flattened_rather_than_read_as_blank() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let pdf_path = directory.path().join("transparent.pdf");
    fs::write(
        &pdf_path,
        pdf_with_content(&helvetica_lines(&["Transparent Background Marker"])),
    )
    .unwrap();
    let source = directory.path().join("transparent.png");
    let status = Command::new("/usr/bin/sips")
        .args(["-s", "format", "png"])
        .arg(&pdf_path)
        .arg("--out")
        .arg(&source)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "sips failed to rasterize the pdf");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(&worker, directory.path(), &source, &sha256, &[]);

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let report = read_report::<VisionReport>(&run.staging);
    let VisionOutcome::Converted { .. } = report.outcome else {
        panic!(
            "a transparent background is not a blank page: {:?}",
            report.outcome
        );
    };
    let markdown = fs::read_to_string(run.staging.join(VISION_MARKDOWN_FILE)).unwrap();
    assert!(
        markdown.contains("Transparent Background Marker"),
        "the flattened page should read back its own text: {markdown}"
    );
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
    let VisionOutcome::Rejected { code } = read_report::<VisionReport>(&run.staging).outcome else {
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
    let VisionOutcome::Rejected { code } = read_report::<VisionReport>(&run.staging).outcome else {
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
    let VisionOutcome::Rejected { code } = read_report::<VisionReport>(&run.staging).outcome else {
        panic!("a one byte ceiling cannot hold a page of Markdown");
    };
    assert_eq!(code, VisionRejectionCode::OutputTooLarge);
    assert!(!run.staging.join(VISION_MARKDOWN_FILE).exists());
}

/// Admission accepts a PDF behind a UTF-8 BOM or whitespace, so the worker
/// has to take it as a PDF too, not hand it to the image decoder. The report
/// counts the page Vision read nothing on.
#[test]
fn a_scan_behind_a_bom_is_read_as_a_pdf_and_its_blank_page_is_counted() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut pdf = b"\xEF\xBB\xBF\n".to_vec();
    pdf.extend(scanned_pdf(directory.path(), ["Scanned Page One", ""]));
    let source = directory.path().join("scan.pdf");
    fs::write(&source, &pdf).unwrap();

    let run = run_worker(&worker, directory.path(), &source, &digest(&pdf), &[]);

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let VisionOutcome::Converted { pages, .. } = read_report::<VisionReport>(&run.staging).outcome
    else {
        panic!("a scan with one readable page should convert");
    };
    assert_eq!(
        pages,
        Some(VisionPages {
            total: 2,
            without_text: 1
        })
    );
}

/// `drawPDFPage` ignores `/Rotate`. Vision still reads a sideways page's words,
/// so only the order shows the page reached it on its side.
#[test]
fn a_page_turned_by_its_rotate_key_reads_top_to_bottom() {
    let Some(worker) = tool("tool-kit-vision-worker") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let pdf = rotated_scan(directory.path());
    let source = directory.path().join("rotated.pdf");
    fs::write(&source, &pdf).unwrap();

    let run = run_worker(&worker, directory.path(), &source, &digest(&pdf), &[]);

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let VisionOutcome::Converted { .. } = read_report::<VisionReport>(&run.staging).outcome else {
        panic!("a rotated scan should convert");
    };
    let markdown = fs::read_to_string(run.staging.join(VISION_MARKDOWN_FILE)).unwrap();
    let opening = markdown.find("Opening").expect("the opening line was read");
    let closing = markdown
        .find("closing line")
        .expect("the closing line was read");
    assert!(opening < closing, "read out of order:\n{markdown}");
}

/// The native page keeps pdf-inspector's exact text and only the scanned page
/// goes to Vision. OCR over the native page drops figures a bank statement
/// cannot lose.
#[tokio::test]
async fn a_mixed_pdf_keeps_its_native_text_and_reads_only_the_scanned_page() {
    let Some(harness) = support::TestHarness::with_vision_worker() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let pdf = mixed_pdf(directory.path());

    let (data, markdown) = convert_scan(&harness, &pdf).await;

    assert_eq!(data["warnings"], serde_json::json!([]), "{data}");
    let native = markdown
        .find("Balance 99,000.00 on the closing date.")
        .expect("the native line survives byte for byte");
    let scanned = markdown
        .find("Scanned Page Two")
        .expect("the scanned page was read");
    assert!(native < scanned, "pages out of order:\n{markdown}");
}

/// Submits a scanned PDF and waits for the job to settle. Returns the job and
/// the published Markdown.
async fn convert_scan(harness: &support::TestHarness, pdf: &[u8]) -> (Value, String) {
    let app = harness.app().await;
    let response = app
        .submit(
            support::multipart_body(Uuid::new_v4(), "standard", pdf, "scan.pdf"),
            "scan",
            support::TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id = support::json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Vision's first model load can take most of a minute.
    let mut data = Value::Null;
    for _ in 0..1200 {
        let response = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}"))
            .await;
        data = support::json_body(response).await["data"].clone();
        if matches!(
            data["status"].as_str(),
            Some("succeeded" | "failed" | "needs_remote")
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(data["status"], "succeeded", "{data}");
    assert_eq!(data["route"]["kind"], "local_vision", "{data}");
    let response = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (data, String::from_utf8(body.to_vec()).unwrap())
}

/// A scan converts on this machine instead of waiting for Datalab: the
/// inspector gives up on a PDF with no text on any page, and Vision reads it.
#[tokio::test]
async fn a_pdf_with_no_text_on_any_page_converts_locally_through_vision() {
    let Some(harness) = support::TestHarness::with_vision_worker() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let pdf = scanned_pdf(directory.path(), ["Scanned Page One", "Scanned Page Two"]);

    let (data, markdown) = convert_scan(&harness, &pdf).await;

    assert_eq!(data["warnings"], serde_json::json!([]), "{data}");
    assert!(markdown.contains("Scanned Page One"), "{markdown}");
    assert!(markdown.contains("Scanned Page Two"), "{markdown}");
}

/// A scan page Vision reads nothing on still publishes the rest, but never as
/// a clean success: the page may be blank, or its content may be gone.
#[tokio::test]
async fn a_scan_with_an_unreadable_page_publishes_with_a_warning() {
    let Some(harness) = support::TestHarness::with_vision_worker() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let pdf = scanned_pdf(directory.path(), ["Scanned Page One", ""]);

    let (data, markdown) = convert_scan(&harness, &pdf).await;

    assert_eq!(
        data["warnings"],
        serde_json::json!(["pages_without_extractable_text"]),
        "{data}"
    );
    assert!(markdown.contains("Scanned Page One"), "{markdown}");
}

/// A DOCX picture with no alt text may gain a description from Foundation
/// Models, one with author alt text keeps it, and the marker token never
/// reaches the Markdown. Runs with or without the worker and the model.
#[tokio::test]
async fn docx_pictures_never_leak_the_description_marker() {
    let harness =
        support::TestHarness::with_vision_worker().unwrap_or_else(support::TestHarness::new);
    let picture = |descr: &str| {
        format!(
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="Picture"{descr}/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="Picture"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImage"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
        )
    };
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body><w:p><w:r><w:t>Before the pictures.</w:t></w:r></w:p>{}{}<w:p><w:r><w:t>After the pictures.</w:t></w:r></w:p></w:body></w:document>"#,
        picture(r#" descr="""#),
        picture(r#" descr="A red dot""#),
    );
    let parts = [
        (
            "[Content_Types].xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.as_slice(),
        ),
        (
            "_rels/.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.as_slice(),
        ),
        (
            "word/_rels/document.xml.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/></Relationships>"#.as_slice(),
        ),
        ("word/document.xml", document.as_bytes()),
        (
            "word/media/image1.png",
            // A 1x1 red PNG.
            b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\x0d\x49\x44\x41\x54\x78\xda\x63\xfc\xcf\xc0\x50\x0f\x00\x04\x85\x01\x80\x84\xa9\x8c\x21\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82".as_slice(),
        ),
    ];
    let mut docx = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        docx.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        docx.write_all(bytes).unwrap();
    }
    let docx = docx.finish().unwrap().into_inner();

    let app = harness.app().await;
    let response = app
        .submit(
            support::multipart(
                &[
                    ("clientRunId", &Uuid::new_v4().to_string()),
                    ("profile", "standard"),
                ],
                Some((
                    &docx,
                    "pictures.docx",
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                )),
            ),
            "pictures",
            support::TOKEN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job_id = support::json_body(response).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Foundation Models' first load can take most of a minute.
    let mut data = Value::Null;
    for _ in 0..1200 {
        let response = app
            .authorized_get(&format!("/api/v1/conversions/{job_id}"))
            .await;
        data = support::json_body(response).await["data"].clone();
        if matches!(
            data["status"].as_str(),
            Some("succeeded" | "failed" | "needs_remote")
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(data["status"], "succeeded", "{data}");

    let response = app
        .authorized_get(&format!("/api/v1/conversions/{job_id}/artifacts/markdown"))
        .await;
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let markdown = String::from_utf8(body.to_vec()).unwrap();
    assert!(!markdown.contains("tkimg"), "{markdown}");
    assert!(markdown.contains("A red dot"), "{markdown}");
    assert!(markdown.contains("Before the pictures."), "{markdown}");
    assert!(markdown.contains("After the pictures."), "{markdown}");
}
