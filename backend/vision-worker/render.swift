// Markdown rendering from a Vision document observation.
//
// Shared by the worker and by tk-vision so the preview tool cannot drift from
// what the worker actually publishes. It drifted once: the false-table guard
// landed in the worker while tk-vision kept its own copy and went on printing
// the table the worker had stopped emitting.

import Foundation
import Vision

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
        claimed += normalize(text) + " "
        blocks.append((topEdge(title.boundingRegion), "# " + text))
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
