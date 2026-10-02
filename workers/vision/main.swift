// tool-kit-vision-worker: read one image or one scanned PDF with Apple Vision,
// write Markdown.
//
// Spawned the way apps/converter/src/engines/pdf_inspector.rs spawns
// tool-kit-pdf-worker: cleared environment, staging directory as argv[1],
// source bytes on stdin, stdout and stderr on /dev/null. The report file is
// the only channel back, so every decision has to land there. Exit 0 once a
// report is written, 70 on any failure.
//
// The wire contract is crates/worker-protocol/src/vision.rs. Keep the two in
// sync.

import CoreGraphics
import CryptoKit
import Darwin
import Foundation
import FoundationModels
import ImageIO
import Vision

private let protocolVersion = 1
private let engineName = "apple-vision"
private let markdownFile = "result.md"
private let reportFile = "worker-report.json"
private let identityPrefix = "tool-kit-vision-worker protocol=1 apple-vision="

// The three source-binding names are the PDF worker's verbatim: identical
// meaning, identical parent-side values.
private let expectedSourceBytesEnv = "TOOLKIT_WORKER_EXPECTED_SOURCE_BYTES"
private let expectedSourceSha256Env = "TOOLKIT_WORKER_EXPECTED_SOURCE_SHA256"
private let maxOutputBytesEnv = "TOOLKIT_WORKER_MAX_OUTPUT_BYTES"
private let languageCorrectionEnv = "TOOLKIT_VISION_WORKER_LANGUAGE_CORRECTION"
private let customWordsEnv = "TOOLKIT_VISION_WORKER_CUSTOM_WORDS"

/// The rejection codes worker-protocol declares. Nothing else may be sent.
private enum Rejection: String {
    case invalidImage = "invalid_image"
    case multiFrameImage = "multi_frame_image"
    case noTextFound = "no_text_found"
    case outputTooLarge = "output_too_large"
}

private struct Artifact: Encodable {
    let relativePath: String
    let byteLength: Int
    let sha256: String
}

private struct EngineIdentity: Encodable {
    let name: String
    let version: String
    let features: [String]
}

/// Serde tags the outcome with `kind`, so the encoding is written by hand
/// rather than synthesized.
private enum Outcome: Encodable {
    case converted(Artifact)
    case rejected(Rejection)

    enum CodingKeys: String, CodingKey {
        case kind, artifact, code
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .converted(let artifact):
            try container.encode("converted", forKey: .kind)
            try container.encode(artifact, forKey: .artifact)
        case .rejected(let code):
            try container.encode("rejected", forKey: .kind)
            try container.encode(code.rawValue, forKey: .code)
        }
    }
}

private struct Report: Encodable {
    let protocolVersion: Int
    let engine: EngineIdentity
    let outcome: Outcome
}

/// Vision ships with the OS, so the macOS product version is its only version
/// number. Foundation always carries three components; `sw_vers` drops a
/// trailing zero.
private func macosVersion() -> String {
    let version = ProcessInfo.processInfo.operatingSystemVersion
    return "\(version.majorVersion).\(version.minorVersion).\(version.patchVersion)"
}

/// stdout carries the identity line and nothing else, so a failure is a stderr
/// line plus the exit code.
private func fail(_ note: String) -> Never {
    FileHandle.standardError.write(Data("tool-kit-vision-worker: \(note)\n".utf8))
    exit(70)
}

private func environment(_ name: String) -> String? {
    ProcessInfo.processInfo.environment[name]
}

private func isLowercaseSha256(_ value: String) -> Bool {
    value.utf8.count == 64
        && value.utf8.allSatisfy { (0x30...0x39).contains($0) || (0x61...0x66).contains($0) }
}

private func hexDigest(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

/// Read exactly `expected` bytes from stdin, then prove EOF. Mirrors
/// `copy_exact_source` in tool-kit-pdf-worker.rs. A short read is normal on a
/// pipe, so the loop keeps going until it holds the whole declared length.
private func readExactStdin(_ expected: Int) -> Data? {
    var data = Data()
    data.reserveCapacity(expected)
    var buffer = [UInt8](repeating: 0, count: 64 * 1024)
    while data.count < expected {
        let want = min(buffer.count, expected - data.count)
        let count = buffer.withUnsafeMutableBytes { read(0, $0.baseAddress, want) }
        if count < 0 {
            if errno == EINTR { continue }
            return nil
        }
        if count == 0 { return nil }  // short source
        data.append(contentsOf: buffer[0..<count])
    }
    var extra: UInt8 = 0
    while true {  // one more byte must not arrive
        let count = withUnsafeMutablePointer(to: &extra) { read(0, $0, 1) }
        if count < 0 && errno == EINTR { continue }
        if count != 0 { return nil }
        break
    }
    return data
}

private enum Decoded {
    case image(CGImage, CGImagePropertyOrientation)
    case rejected(Rejection)
}

/// A non-image yields a live image source with zero frames rather than nil, so
/// the frame count and the decode both have to be checked.
///
/// Only one frame is ever read, so a source carrying more is refused rather
/// than truncated: a multi-page TIFF is the canonical scanned document, and
/// publishing its first page as the whole thing is a silent loss no later
/// stage can see.
///
/// The decoded pixels ignore the EXIF orientation a phone camera writes, so it
/// travels beside them to Vision. Without it Vision still reads the words but
/// its geometry is in the stored frame, and the blocks sort out of order.
private func decodeOnlyFrame(_ source: Data) -> Decoded {
    guard let imageSource = CGImageSourceCreateWithData(source as CFData, nil) else {
        return .rejected(.invalidImage)
    }
    let frames = CGImageSourceGetCount(imageSource)
    if frames > 1 { return .rejected(.multiFrameImage) }
    guard frames == 1, let image = CGImageSourceCreateImageAtIndex(imageSource, 0, nil) else {
        return .rejected(.invalidImage)
    }
    let properties = CGImageSourceCopyPropertiesAtIndex(imageSource, 0, nil) as? [CFString: Any]
    let orientation = (properties?[kCGImagePropertyOrientation] as? UInt32)
        .flatMap(CGImagePropertyOrientation.init(rawValue:)) ?? .up
    return .image(flattenedOntoWhite(image), orientation)
}

/// An image carrying real transparency has to land on an opaque background
/// before Vision sees it. Dark text drawn on a transparent background
/// composites to nothing, so the recognizer finds no text and the run comes
/// back `no_text_found`, which reads as a quality failure and spends the remote
/// fallback on a file that was always readable. Nothing downstream can tell
/// that apart from a genuinely blank scan.
///
/// White, because that is what the PDF rasterizer already fills with and what
/// printing the image would do. It is a choice, not a neutral operation: light
/// text on a transparent background disappears into it. Dark on light is what
/// documents are, and the alternative loses the common case to protect the rare
/// one.
private func flattenedOntoWhite(_ image: CGImage) -> CGImage {
    switch image.alphaInfo {
    case .none, .noneSkipLast, .noneSkipFirst:
        return image
    default:
        break
    }
    let bounds = CGRect(x: 0, y: 0, width: image.width, height: image.height)
    guard let context = CGContext(
        data: nil, width: image.width, height: image.height,
        bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ) else {
        // A context this size is what the recognizer would have allocated
        // anyway. Failing here means handing back the original rather than
        // failing the run, since an unflattened image still converts whenever
        // its transparent pixels happen to be light.
        return image
    }
    context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    context.fill(bounds)
    context.draw(image, in: bounds)
    return context.makeImage() ?? image
}

// MARK: - Run

/// Writes the report and ends the process. `withoutOverwriting` is the
/// worker's `create_new`: a collision fails the run instead of publishing over
/// someone else's bytes.
private func finish(_ outcome: Outcome, in staging: URL) -> Never {
    let report = Report(
        protocolVersion: protocolVersion,
        engine: EngineIdentity(name: engineName, version: macosVersion(), features: []),
        outcome: outcome)
    do {
        try JSONEncoder().encode(report).write(
            to: staging.appendingPathComponent(reportFile), options: .withoutOverwriting)
    } catch {
        fail("\(reportFile) write failed: \(error)")
    }
    exit(0)
}

/// One bitmap's Markdown. A page Vision found nothing on comes back either as
/// no observation at all or as a document with empty collections. Both are the
/// same answer: an empty string.
@available(macOS 26, *)
private func recognize(
    _ image: CGImage, orientation: CGImagePropertyOrientation = .up,
    with request: RecognizeDocumentsRequest
) async -> String {
    let observations: [DocumentObservation]
    do {
        observations = try await request.perform(on: image, orientation: orientation)
    } catch {
        fail("Vision could not read the image: \(error)")
    }
    guard let document = observations.first?.document else { return "" }
    return markdown(document).trimmingCharacters(in: .whitespacesAndNewlines)
}

@available(macOS 26, *)
private func run(_ stagingPath: String) async -> Never {
    // `attributesOfItem` reports a symlink as a symlink, so this rejects one
    // pointing at a directory.
    let attributes = try? FileManager.default.attributesOfItem(atPath: stagingPath)
    guard attributes?[.type] as? FileAttributeType == .typeDirectory else {
        fail("staging path is not a directory: \(stagingPath)")
    }
    let staging = URL(fileURLWithPath: stagingPath)

    guard let expectedBytes = environment(expectedSourceBytesEnv).flatMap(Int.init),
          expectedBytes > 0
    else { fail("\(expectedSourceBytesEnv) must be a positive byte count") }
    guard let expectedSha256 = environment(expectedSourceSha256Env),
          isLowercaseSha256(expectedSha256)
    else { fail("\(expectedSourceSha256Env) must be a lowercase hex sha256") }
    guard let maxOutputBytes = environment(maxOutputBytesEnv).flatMap(Int.init),
          maxOutputBytes > 0
    else { fail("\(maxOutputBytesEnv) must be a positive byte count") }

    let languageCorrection: Bool
    switch environment(languageCorrectionEnv) {
    case nil, "1": languageCorrection = true
    case "0": languageCorrection = false
    default: fail("\(languageCorrectionEnv) must be 1 or 0")
    }
    let customWords = (environment(customWordsEnv) ?? "").split(separator: "\n").map(String.init)

    guard let source = readExactStdin(expectedBytes) else {
        fail("stdin did not hold exactly \(expectedBytes) bytes")
    }
    guard hexDigest(source) == expectedSha256 else { fail("source digest mismatch") }

    var request = RecognizeDocumentsRequest()
    request.textRecognitionOptions.useLanguageCorrection = languageCorrection
    request.textRecognitionOptions.customWords = customWords

    let rendered: String
    if source.starts(with: Data("%PDF-".utf8)) {
        // A scanned PDF the inspector found no text in. One page is rendered,
        // read and released at a time, and a page with no text adds nothing.
        guard let provider = CGDataProvider(data: source as CFData),
              let pdf = CGPDFDocument(provider), pdf.numberOfPages > 0
        else { finish(.rejected(.invalidImage), in: staging) }
        var pages: [String] = []
        for number in 1...pdf.numberOfPages {
            guard let page = pdf.page(at: number), let image = renderPage(page, dpi: 200) else {
                finish(.rejected(.invalidImage), in: staging)
            }
            let text = await recognize(image, with: request)
            if !text.isEmpty { pages.append(text) }
        }
        rendered = pages.joined(separator: "\n\n")
    } else {
        switch decodeOnlyFrame(source) {
        case .image(let image, let orientation):
            rendered = await recognize(image, orientation: orientation, with: request)
        case .rejected(let rejection): finish(.rejected(rejection), in: staging)
        }
    }
    if rendered.isEmpty {
        finish(.rejected(.noTextFound), in: staging)
    }

    let bytes = Data(rendered.utf8)
    if bytes.count > maxOutputBytes {
        finish(.rejected(.outputTooLarge), in: staging)
    }
    do {
        try bytes.write(
            to: staging.appendingPathComponent(markdownFile), options: .withoutOverwriting)
    } catch {
        fail("\(markdownFile) write failed: \(error)")
    }
    finish(
        .converted(
            Artifact(
                relativePath: markdownFile, byteLength: bytes.count, sha256: hexDigest(bytes))),
        in: staging)
}

/// `--describe <dir>`: writes `<file>.txt` beside each image in `dir` that
/// Foundation Models can describe. A decorative image, a per-image error, or no
/// model writes nothing, and the run still exits 0.
private func describe(_ directory: String) async -> Never {
    guard #available(macOS 27, *) else { exit(0) }
    let model = SystemLanguageModel.default
    guard model.availability == .available, model.capabilities.contains(.vision) else { exit(0) }
    let instructions = """
        You describe images found inside documents for a reader who cannot see them.
        Write one or two plain sentences. Write more only for a complex diagram.
        For a chart, say what kind of chart it is and what its axes or categories are.
        Never state any numbers or values.
        For a logo or decorative graphic, answer with the single word: decorative.
        """
    let names = (try? FileManager.default.contentsOfDirectory(atPath: directory)) ?? []
    for name in names.sorted() where !name.hasSuffix(".txt") {
        let image = URL(fileURLWithPath: directory).appendingPathComponent(name)
        // One session per image, so no description leans on the last one.
        // Greedy sampling, so the same picture gets the same answer.
        let session = LanguageModelSession(model: model, instructions: instructions)
        guard let response = try? await session.respond(
            options: GenerationOptions(samplingMode: .greedy),
            prompt: {
                "Describe this image."
                Attachment(imageURL: image)
            })
        else { continue }
        let text = response.content.trimmingCharacters(in: .whitespacesAndNewlines)
        let word = text.lowercased().trimmingCharacters(in: .punctuationCharacters)
        if text.isEmpty || word == "decorative" { continue }
        try? Data(text.utf8).write(to: image.appendingPathExtension("txt"), options: .atomic)
    }
    exit(0)
}

@main
struct VisionWorker {
    static func main() async {
        let arguments = Array(CommandLine.arguments.dropFirst())
        // Built below the macOS 26 floor so this is reachable, and checked
        // ahead of --version: the floor belongs to the handshake, or an older
        // host hands back an identity and the engine advertises six image
        // formats it fails every time.
        guard #available(macOS 26, *) else {
            fail("RecognizeDocumentsRequest needs macOS 26, found \(macosVersion())")
        }
        if arguments == ["--version"] {
            FileHandle.standardOutput.write(Data("\(identityPrefix)\(macosVersion())\n".utf8))
            exit(0)
        }
        if arguments.count == 2, arguments[0] == "--describe" {
            await describe(arguments[1])
        }
        guard arguments.count == 1 else {
            fail("usage: tool-kit-vision-worker <staging-directory>")
        }
        await run(arguments[0])
    }
}
