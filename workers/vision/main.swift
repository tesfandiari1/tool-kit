// tool-kit-vision-worker: read one image with Apple Vision, write Markdown.
//
// Spawned the way backend/src/engines/pdf_inspector.rs spawns
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
    case image(CGImage)
    case rejected(Rejection)
}

/// A non-image yields a live image source with zero frames rather than nil, so
/// the frame count and the decode both have to be checked.
///
/// Only one frame is ever read, so a source carrying more is refused rather
/// than truncated: a multi-page TIFF is the canonical scanned document, and
/// publishing its first page as the whole thing is a silent loss no later
/// stage can see.
private func decodeOnlyFrame(_ source: Data) -> Decoded {
    guard let imageSource = CGImageSourceCreateWithData(source as CFData, nil) else {
        return .rejected(.invalidImage)
    }
    let frames = CGImageSourceGetCount(imageSource)
    if frames > 1 { return .rejected(.multiFrameImage) }
    guard frames == 1, let image = CGImageSourceCreateImageAtIndex(imageSource, 0, nil) else {
        return .rejected(.invalidImage)
    }
    return .image(image)
}

// MARK: - Markdown rendering, ported from tk-vision.swift


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

    let image: CGImage
    switch decodeOnlyFrame(source) {
    case .image(let decoded): image = decoded
    case .rejected(let rejection): finish(.rejected(rejection), in: staging)
    }

    var request = RecognizeDocumentsRequest()
    request.textRecognitionOptions.useLanguageCorrection = languageCorrection
    request.textRecognitionOptions.customWords = customWords

    let observations: [DocumentObservation]
    do {
        observations = try await request.perform(on: image)
    } catch {
        fail("Vision could not read the image: \(error)")
    }
    // A page Vision found nothing on comes back either as no observation at
    // all or as a document with empty collections. Both are the same answer.
    guard let document = observations.first?.document else {
        finish(.rejected(.noTextFound), in: staging)
    }
    let rendered = markdown(document).trimmingCharacters(in: .whitespacesAndNewlines)
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
        guard arguments.count == 1 else {
            fail("usage: tool-kit-vision-worker <staging-directory>")
        }
        await run(arguments[0])
    }
}
