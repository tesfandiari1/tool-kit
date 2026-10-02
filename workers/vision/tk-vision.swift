// tk-vision: read a PDF or image with Apple Vision, print Markdown.
// A development tool. Nothing in Tool-Kit calls it.
//
//   tk-vision <file> [--flat] [--page N] [--dpi N] [--fast]
//
// --flat prints document.text.transcript, the ungrouped baseline, so the
// structured output can be judged against what plain OCR would have given.

import CoreGraphics
import Foundation
import ImageIO
import Vision

struct Options {
    var path: String
    var flat = false
    var dpi: CGFloat = 200
    var fast = false
    var page: Int? = nil
}

enum Fail: Error, CustomStringConvertible {
    case usage(String)
    case unreadable(String)
    var description: String {
        switch self {
        case .usage(let m): return "usage: \(m)"
        case .unreadable(let m): return "cannot read: \(m)"
        }
    }
}

func parseArgs() throws -> Options {
    var args = Array(CommandLine.arguments.dropFirst())
    guard let path = args.first(where: { !$0.hasPrefix("--") }) else {
        throw Fail.usage("tk-vision <file> [--flat] [--page N] [--dpi N] [--fast]")
    }
    args.removeAll { $0 == path }
    var o = Options(path: path)
    var i = 0
    while i < args.count {
        switch args[i] {
        case "--flat": o.flat = true
        case "--fast": o.fast = true
        case "--page":
            i += 1
            guard i < args.count, let v = Int(args[i]) else { throw Fail.usage("--page needs a number") }
            o.page = v
        case "--dpi":
            i += 1
            guard i < args.count, let v = Double(args[i]) else { throw Fail.usage("--dpi needs a number") }
            o.dpi = CGFloat(v)
        default: throw Fail.usage("unknown flag \(args[i])")
        }
        i += 1
    }
    return o
}

/// Every page of the input as a bitmap. A PDF is rasterized at `dpi`; an image
/// file is one page.
func rasterize(_ url: URL, dpi: CGFloat) throws -> [CGImage] {
    if url.pathExtension.lowercased() == "pdf", let doc = CGPDFDocument(url as CFURL) {
        return try (1...max(doc.numberOfPages, 1)).compactMap { n -> CGImage? in
            guard let page = doc.page(at: n) else { return nil }
            guard let image = renderPage(page, dpi: dpi) else {
                throw Fail.unreadable("could not render page \(n)")
            }
            return image
        }
    }
    guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
          let img = CGImageSourceCreateImageAtIndex(src, 0, nil)
    else { throw Fail.unreadable(url.path) }
    return [img]
}

@main
struct Tool {
    static func main() async {
        do {
            let o = try parseArgs()
            let url = URL(fileURLWithPath: o.path)
            let started = Date()
            var pages = try rasterize(url, dpi: o.dpi)
            let offset = (o.page ?? 1) - 1
            if let only = o.page {
                guard only >= 1, only <= pages.count else { throw Fail.usage("--page \(only) but the file has \(pages.count)") }
                pages = [pages[only - 1]]
            }
            let rasterMs = Date().timeIntervalSince(started) * 1000
            FileHandle.standardError.write(
                "rasterized \(pages.count) page(s) at \(Int(o.dpi))dpi in \(Int(rasterMs))ms\n".data(using: .utf8)!)

            var request = RecognizeDocumentsRequest()
            request.textRecognitionOptions.useLanguageCorrection = !o.fast
            request.textRecognitionOptions.automaticallyDetectLanguage = true

            for (i, image) in pages.enumerated() {
                let n = i + offset
                let t0 = Date()
                let observations = try await request.perform(on: image)
                let ms = Date().timeIntervalSince(t0) * 1000
                guard let doc = observations.first?.document else {
                    FileHandle.standardError.write("page \(n + 1): no document observation\n".data(using: .utf8)!)
                    continue
                }
                FileHandle.standardError.write(
                    "page \(n + 1): \(Int(ms))ms, \(doc.paragraphs.count) paragraphs, \(doc.tables.count) tables, \(doc.lists.count) lists\n"
                        .data(using: .utf8)!)
                if pages.count > 1 { print("<!-- page \(n + 1) -->\n") }
                print(o.flat ? doc.text.transcript : markdown(doc))
                print("")
            }
            FileHandle.standardError.write(
                "total \(Int(Date().timeIntervalSince(started) * 1000))ms\n".data(using: .utf8)!)
        } catch {
            FileHandle.standardError.write("\(error)\n".data(using: .utf8)!)
            exit(2)
        }
    }
}
