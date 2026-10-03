//! M4 PDF corpus: one generator per routing-policy input class.
//!
//! Every generator writes bytes the same way `pdf_with_content` does: objects
//! numbered from 1, offsets collected while writing, then xref and trailer.
//! Image XObjects are stored uncompressed so the corpus needs no deflate
//! dependency. Each classification below was read back from the real worker.

use std::io::Write as _;

/// 64x64 RGB, the smallest image that still reads as a page-filling picture.
/// 64x64 is 4,096 pixels, far below pdf-inspector's 500,000-pixel
/// template-image threshold, so these pages count as non-text because they
/// carry no text operators rather than because they look like a scan. An
/// 800x800 image was checked and changed no classification, so the small one
/// stays: same behaviour, 122x fewer bytes per fixture.
const IMAGE_SIDE: usize = 64;

/// N native text pages: text_based, confidence 1.0, worker converts.
pub(crate) fn native_pdf(pages: usize) -> Vec<u8> {
    text_and_image_pdf(pages, 0)
}

/// N image-only pages: scanned, worker falls back with scanned_pdf.
pub(crate) fn image_only_pdf(pages: usize) -> Vec<u8> {
    text_and_image_pdf(0, pages)
}

/// text_pages native pages plus one image page: mixed at 1, then text_based
/// with confidence text_pages/(text_pages + 1), which is the live defect.
pub(crate) fn partly_scanned_pdf(text_pages: usize) -> Vec<u8> {
    text_and_image_pdf(text_pages, 1)
}

/// A one-line title page followed by N dense text pages, no images anywhere.
///
/// The control for `partly_scanned_pdf`: it reports the same confidence, and
/// nothing is missing from the Markdown. Any policy that routes on confidence
/// alone bills a paid provider for this document.
pub(crate) fn sparse_cover_pdf(dense_pages: usize) -> Vec<u8> {
    let total = dense_pages + 1;
    let first_page = 5;
    let first_content = first_page + total;
    let kids = (0..total)
        .map(|index| format!("{} 0 R", first_page + index))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {total} >>").into_bytes(),
        helvetica(),
        image_object(),
    ];
    for index in 0..total {
        objects.push(page_object(2, 3, first_content + index, ""));
    }
    objects.push(stream_object(
        "",
        b"BT\n/F1 24 Tf\n1 0 0 1 72 700 Tm (Annual Report 2026) Tj\nET\n",
    ));
    for index in 0..dense_pages {
        objects.push(stream_object("", &page_text(index)));
    }
    assemble(&objects)
}

/// One page of ruled grid plus aligned cell text: text_based with tables.
pub(crate) fn dense_table_pdf() -> Vec<u8> {
    let mut content = Vec::new();
    let rows = 12;
    let columns = 5;
    let left = 60.0_f32;
    let top = 720.0_f32;
    let cell_width = 100.0_f32;
    let cell_height = 22.0_f32;
    let right = left + cell_width * columns as f32;
    let bottom = top - cell_height * rows as f32;

    content.extend_from_slice(b"0.6 w\n");
    for row in 0..=rows {
        let y = top - cell_height * row as f32;
        writeln!(content, "{left} {y} m {right} {y} l S").unwrap();
    }
    for column in 0..=columns {
        let x = left + cell_width * column as f32;
        writeln!(content, "{x} {top} m {x} {bottom} l S").unwrap();
    }
    content.extend_from_slice(b"BT\n/F1 9 Tf\n");
    for row in 0..rows {
        for column in 0..columns {
            let x = left + cell_width * column as f32 + 4.0;
            let y = top - cell_height * row as f32 - 15.0;
            writeln!(
                content,
                "1 0 0 1 {x} {y} Tm (R{row} C{column} {}) Tj",
                1000 + row * columns + column
            )
            .unwrap();
        }
    }
    content.extend_from_slice(b"ET\n");

    assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        helvetica(),
        page_object(2, 3, 5, ""),
        stream_object("", &content),
    ])
}

/// One page of two text columns: text_based with a multi-column layout.
pub(crate) fn two_column_pdf() -> Vec<u8> {
    let mut content = b"BT\n/F1 10 Tf\n".to_vec();
    for column in 0..2 {
        let x = 72.0 + column as f32 * 250.0;
        for line in 0..30 {
            let y = 720.0 - line as f32 * 14.0;
            writeln!(
                content,
                "1 0 0 1 {x} {y} Tm (Column {column} line {line} of running body text) Tj"
            )
            .unwrap();
        }
    }
    content.extend_from_slice(b"ET\n");

    assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        helvetica(),
        page_object(2, 3, 5, ""),
        stream_object("", &content),
    ])
}

/// Identity-H composite font with no ToUnicode: text_based, suspected garbled text.
pub(crate) fn garbled_font_pdf() -> Vec<u8> {
    let mut content = b"BT\n/F1 18 Tf\n".to_vec();
    for line in 0..2 {
        let y = 720 - line * 24;
        writeln!(
            content,
            "1 0 0 1 72 {y} Tm <002400450048004F004F0052005A00520055004F0047> Tj"
        )
        .unwrap();
    }
    content.extend_from_slice(b"ET\n");

    assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [6 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /AAAAAA+Opaque /Encoding /Identity-H /DescendantFonts [4 0 R] >>".to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /AAAAAA+Opaque /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 5 0 R /DW 600 /CIDToGIDMap /Identity >>".to_vec(),
        b"<< /Type /FontDescriptor /FontName /AAAAAA+Opaque /Flags 4 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 >>".to_vec(),
        page_object(2, 3, 7, ""),
        stream_object("", &content),
    ])
}

/// AcroForm text field whose value lives in the annotation, not the content stream.
pub(crate) fn form_pdf() -> Vec<u8> {
    let content = b"BT\n/F1 11 Tf\n1 0 0 1 72 740 Tm (Name) Tj\nET\n".to_vec();
    assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] /DA (/F1 0 Tf 0 g) /DR << /Font << /F1 3 0 R >> >> >> >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        helvetica(),
        page_object(2, 3, 6, " /Annots [5 0 R]"),
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Value typed into the field) /Rect [72 700 372 728] /F 4 /P 4 0 R /DA (/F1 11 Tf 0 g) >>".to_vec(),
        stream_object("", &content),
    ])
}

/// Standard security handler, V1/R2: worker rejects with encrypted_pdf.
pub(crate) fn encrypted_pdf() -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        helvetica(),
        page_object(2, 3, 5, ""),
        stream_object("", &page_text(0)),
        // 32-byte /O and /U strings, as R2 requires. The body is not actually
        // RC4'd: detection happens before any string is decrypted.
        format!(
            "<< /Filter /Standard /V 1 /R 2 /O <{}> /U <{}> /P -1 >>",
            "a1".repeat(32),
            "b2".repeat(32)
        )
        .into_bytes(),
    ];
    assemble_with_trailer(
        &objects,
        " /Encrypt 6 0 R /ID [<0123456789abcdef0123456789abcdef> <0123456789abcdef0123456789abcdef>]",
    )
}

/// First 120 bytes only: worker rejects with invalid_pdf_structure.
pub(crate) fn truncated_pdf() -> Vec<u8> {
    let mut pdf = native_pdf(1);
    pdf.truncate(120);
    pdf
}

/// A `q Q` content stream: no text and no image, so scanned with no_text.
pub(crate) fn blank_content_pdf() -> Vec<u8> {
    assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        helvetica(),
        page_object(2, 3, 5, ""),
        stream_object("", b"q Q\n"),
    ])
}

/// Shared body for the native / image / partly-scanned family: text pages first,
/// then image pages, all in one page tree.
fn text_and_image_pdf(text_pages: usize, image_pages: usize) -> Vec<u8> {
    let total = text_pages + image_pages;
    assert!(total > 0, "a PDF needs at least one page");

    // 1 catalog, 2 pages, 3 font, 4 image (present even when unused: fixed
    // numbering keeps the page objects' references simple), then page objects,
    // then content streams.
    let first_page = 5;
    let first_content = first_page + total;

    let kids = (0..total)
        .map(|index| format!("{} 0 R", first_page + index))
        .collect::<Vec<_>>()
        .join(" ");

    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {total} >>").into_bytes(),
        helvetica(),
        image_object(),
    ];
    for index in 0..total {
        let extra = if index < text_pages {
            String::new()
        } else {
            " /XObject << /Im0 4 0 R >>".to_owned()
        };
        objects.push(page_object(2, 3, first_content + index, &extra));
    }
    for index in 0..total {
        let content = if index < text_pages {
            page_text(index)
        } else {
            b"q\n612 0 0 792 0 0 cm\n/Im0 Do\nQ\n".to_vec()
        };
        objects.push(stream_object("", &content));
    }
    assemble(&objects)
}

/// Lines must not start with "Page": pdf-inspector's page-number prefilter
/// strips them, and the empty Markdown then reads as sparse extraction, which
/// flags every page for OCR.
fn page_text(index: usize) -> Vec<u8> {
    let mut content = b"BT\n/F1 12 Tf\n".to_vec();
    for line in 0..24 {
        let y = 720 - line * 16;
        writeln!(
            content,
            "1 0 0 1 72 {y} Tm (Sheet {index} line {line}: the quick brown fox jumps over the lazy dog.) Tj"
        )
        .unwrap();
    }
    content.extend_from_slice(b"ET\n");
    content
}

pub(crate) fn helvetica() -> Vec<u8> {
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()
}

fn page_object(parent: usize, font: usize, contents: usize, extra_resources: &str) -> Vec<u8> {
    format!(
        "<< /Type /Page /Parent {parent} 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 {font} 0 R >>{extra_resources} >> /Contents {contents} 0 R >>"
    )
    .into_bytes()
}

/// Uncompressed RGB image XObject: 12 KiB of raw samples beats a flate dependency.
fn image_object() -> Vec<u8> {
    let mut samples = Vec::with_capacity(IMAGE_SIDE * IMAGE_SIDE * 3);
    for y in 0..IMAGE_SIDE {
        for x in 0..IMAGE_SIDE {
            // Coarse checkerboard: ink and paper, like a scanned page.
            let ink = ((x / 8) + (y / 8)) % 2 == 0;
            let value = if ink { 24 } else { 232 };
            samples.extend_from_slice(&[value, value, value]);
        }
    }
    stream_object(
        &format!(
            "/Type /XObject /Subtype /Image /Width {IMAGE_SIDE} /Height {IMAGE_SIDE} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8"
        ),
        &samples,
    )
}

pub(crate) fn stream_object(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let mut object = format!("<< {dictionary} /Length {} >>\nstream\n", data.len()).into_bytes();
    object.extend_from_slice(data);
    object.extend_from_slice(b"\nendstream");
    object
}

pub(crate) fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    assemble_with_trailer(objects, "")
}

fn assemble_with_trailer(objects: &[Vec<u8>], extra_trailer: &str) -> Vec<u8> {
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
        "trailer\n<< /Size {} /Root 1 0 R{extra_trailer} >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    pdf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generator_writes_a_pdf_header_and_a_body() {
        let corpus: Vec<(&str, Vec<u8>)> = vec![
            ("native_1", native_pdf(1)),
            ("native_10", native_pdf(10)),
            ("image_only_1", image_only_pdf(1)),
            ("image_only_3", image_only_pdf(3)),
            ("partly_scanned_1", partly_scanned_pdf(1)),
            ("partly_scanned_9", partly_scanned_pdf(9)),
            ("dense_table", dense_table_pdf()),
            ("two_column", two_column_pdf()),
            ("garbled_font", garbled_font_pdf()),
            ("form", form_pdf()),
            ("encrypted", encrypted_pdf()),
            ("truncated", truncated_pdf()),
            ("blank_content", blank_content_pdf()),
        ];
        for (name, bytes) in corpus {
            assert!(bytes.starts_with(b"%PDF-"), "{name} lacks a PDF header");
            assert!(bytes.len() > b"%PDF-1.4\n".len(), "{name} has no body");
        }
    }
}
