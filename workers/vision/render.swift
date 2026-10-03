// Markdown rendering from a Vision document observation.

import CoreGraphics
import Foundation
import Vision

/// The longest side a rendered page may have, in pixels. A page past it renders
/// at a lower dpi: 8192 square is 256 MB of bitmap, where a 200-inch page at
/// 200 dpi would be gigabytes.
private let maxPageSide: CGFloat = 8192

/// One PDF page as an opaque bitmap at `dpi`, white behind the page. Callers
/// render a page, read it and drop it before the next, so a long scan never
/// holds every bitmap at once.
///
/// The crop box is the visible page, and CoreGraphics already clips it to the
/// media box. The media box can carry bleed or a whole sheet around it.
///
/// `drawPDFPage` ignores `/Rotate`, so the page is turned here. Vision reads a
/// sideways scan's words but sorts them into the wrong reading order.
func renderPage(_ page: CGPDFPage, dpi: CGFloat) -> CGImage? {
    let box = page.getBoxRect(.cropBox)
    guard box.width.isFinite, box.height.isFinite, box.width > 0, box.height > 0 else {
        return nil
    }
    let rotation = (Int(page.rotationAngle) % 360 + 360) % 360
    let quarter = rotation == 90 || rotation == 270
    let (pageW, pageH) = quarter ? (box.height, box.width) : (box.width, box.height)
    let scale = min(dpi / 72.0, maxPageSide / max(pageW, pageH))
    let w = max(1, Int((pageW * scale).rounded()))
    let h = max(1, Int((pageH * scale).rounded()))
    guard let ctx = CGContext(
        data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ) else { return nil }
    ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
    ctx.scaleBy(x: scale, y: scale)
    // `/Rotate` turns the page clockwise for display.
    switch rotation {
    case 90:
        ctx.translateBy(x: 0, y: box.width)
        ctx.rotate(by: -.pi / 2)
    case 180:
        ctx.translateBy(x: box.width, y: box.height)
        ctx.rotate(by: .pi)
    case 270:
        ctx.translateBy(x: box.height, y: 0)
        ctx.rotate(by: .pi / 2)
    default:
        break
    }
    ctx.translateBy(x: -box.origin.x, y: -box.origin.y)
    ctx.drawPDFPage(page)
    return ctx.makeImage()
}

/// Vision normalizes with the origin at the bottom left, so a larger top edge
/// means higher on the page. Sorting descending puts blocks in reading order.
@available(macOS 26, *)
func topEdge(_ region: NormalizedRegion) -> Double {
    region.points.map { Double($0.y) }.max() ?? 0
}

func escapeCell(_ text: String) -> String {
    text.replacingOccurrences(of: "|", with: "\\|")
        .replacingOccurrences(of: "\n", with: " ")
        .trimmingCharacters(in: .whitespacesAndNewlines)
}

/// Drops columns and rows that are empty the whole way across.
///
/// Vision's table detection is unreliable in both directions and has to be
/// checked here. It under-reaches, returning a real form of labeled fields as
/// loose paragraphs, and it over-reaches: a page of 24 evenly spaced identical
/// lines came back as a 12-column table with 11 of the columns empty. Pruning
/// answers the second without touching the first, because a real table has
/// content in most of its columns and loses nothing here.
func prune(_ rows: [[String]]) -> [[String]] {
    guard let width = rows.map(\.count).max(), width > 0 else { return [] }
    let padded = rows.map { $0 + Array(repeating: "", count: width - $0.count) }
    let keep = (0..<width).filter { column in padded.contains { !$0[column].isEmpty } }
    return padded
        .map { row in keep.map { row[$0] } }
        .filter { row in row.contains { !$0.isEmpty } }
}

@available(macOS 26, *)
func markdown(_ table: DocumentObservation.Container.Table) -> String {
    let rows = prune(table.rows.map { row in
        row.map { $0.content.text.transcript.trimmingCharacters(in: .whitespacesAndNewlines) }
    })
    guard let header = rows.first else { return "" }
    // One surviving column is a run of lines, not a table. A one-cell-wide
    // pipe grid would put the page's prose in a table nothing reads usefully.
    if header.count == 1 {
        return rows.compactMap(\.first).filter { !$0.isEmpty }.joined(separator: "\n\n")
    }
    var out = [
        "| " + header.map(escapeCell).joined(separator: " | ") + " |",
        "| " + header.map { _ in "---" }.joined(separator: " | ") + " |",
    ]
    for row in rows.dropFirst() {
        out.append("| " + row.map(escapeCell).joined(separator: " | ") + " |")
    }
    return out.joined(separator: "\n")
}

@available(macOS 26, *)
func markdown(_ list: DocumentObservation.Container.List) -> String {
    list.items
        .map { "- " + $0.content.text.transcript.trimmingCharacters(in: .whitespacesAndNewlines) }
        .joined(separator: "\n")
}

func normalize(_ text: String) -> String {
    text.lowercased().split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
}

/// Paragraphs, tables, and lists are separate collections rather than one
/// ordered stream, so reading order has to be rebuilt from the geometry.
///
/// They also overlap: the title and every list item and table cell appear in
/// `paragraphs` too, so anything already emitted has to be claimed and skipped
/// or the same sentence prints two or three times.
@available(macOS 26, *)
func markdown(_ page: DocumentObservation.Container) -> String {
    var blocks: [(Double, String)] = []
    var claimed = ""

    if let title = page.title {
        let text = title.transcript.trimmingCharacters(in: .whitespacesAndNewlines)
        // An empty title would publish a bare "# " heading.
        if !text.isEmpty {
            claimed += normalize(text) + " "
            blocks.append((topEdge(title.boundingRegion), "# " + text))
        }
    }
    for table in page.tables {
        for row in table.rows {
            for cell in row { claimed += normalize(cell.content.text.transcript) + " " }
        }
        blocks.append((topEdge(table.boundingRegion), markdown(table)))
    }
    for list in page.lists {
        for item in list.items { claimed += normalize(item.content.text.transcript) + " " }
        blocks.append((topEdge(list.boundingRegion), markdown(list)))
    }
    for para in page.paragraphs {
        let text = para.transcript.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty { continue }
        let normalized = normalize(text)
        // Short fragments match too easily, so only a substantial run counts
        // as already claimed.
        if normalized.count >= 12, claimed.contains(normalized) { continue }
        blocks.append((topEdge(para.boundingRegion), text))
    }
    return blocks.sorted { $0.0 > $1.0 }.map(\.1).joined(separator: "\n\n")
}
