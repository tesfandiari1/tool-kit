// The wire contract, mirroring crates/worker-protocol/src/audio.rs. Keep the
// two in sync: the Rust side pins the exact JSON in a serde test.

import CryptoKit
import Foundation

let protocolVersion = 1
let engineName = "local-audio"
let markdownFile = "result.md"
let reportFile = "worker-report.json"
let identityPrefix = "tool-kit-audio-worker protocol=1 local-audio="

/// The pin in Package.swift. Part of the engine version because the CoreML
/// layout the worker loads is this package's, not the OS's.
let fluidAudioVersion = "0.15.6"

// The three source-binding names are the PDF worker's verbatim: identical
// meaning, identical parent-side values.
let expectedSourceBytesEnv = "TOOLKIT_WORKER_EXPECTED_SOURCE_BYTES"
let expectedSourceSha256Env = "TOOLKIT_WORKER_EXPECTED_SOURCE_SHA256"
let maxOutputBytesEnv = "TOOLKIT_WORKER_MAX_OUTPUT_BYTES"
let mediaTypeEnv = "TOOLKIT_AUDIO_WORKER_MEDIA_TYPE"
let speakerCountEnv = "TOOLKIT_AUDIO_WORKER_SPEAKER_COUNT"
let localeEnv = "TOOLKIT_AUDIO_WORKER_LOCALE"
let diarizerDirEnv = "TOOLKIT_AUDIO_WORKER_DIARIZER_DIR"

/// AVAudioFile reads the extension, so the scratch copy is named from the
/// media type the converter admitted.
let sourceExtensions = [
    "audio/wav": "wav",
    "audio/mp4": "m4a",
    "video/mp4": "mp4",
    "video/quicktime": "mov",
    "audio/mpeg": "mp3",
    "audio/flac": "flac",
]

/// The rejection codes worker-protocol declares. Nothing else may be sent.
enum Rejection: String {
    case invalidAudio = "invalid_audio"
    case noSpeechFound = "no_speech_found"
    case localeUnsupported = "locale_unsupported"
    case speechAssetsUnavailable = "speech_assets_unavailable"
    case outputTooLarge = "output_too_large"
}

struct Artifact: Encodable {
    let relativePath: String
    let byteLength: Int
    let sha256: String
}

struct Detail: Encodable {
    let audioSeconds: Double
    let speakersFound: Int
    let speakerCountGuessed: Bool
    let locale: String
}

struct EngineIdentity: Encodable {
    let name: String
    let version: String
    let features: [String]
}

/// Serde tags the outcome with `kind`, so the encoding is written by hand
/// rather than synthesized.
enum Outcome: Encodable {
    case converted(Artifact, Detail)
    case rejected(Rejection)

    enum CodingKeys: String, CodingKey {
        case kind, artifact, detail, code
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .converted(let artifact, let detail):
            try container.encode("converted", forKey: .kind)
            try container.encode(artifact, forKey: .artifact)
            try container.encode(detail, forKey: .detail)
        case .rejected(let code):
            try container.encode("rejected", forKey: .kind)
            try container.encode(code.rawValue, forKey: .code)
        }
    }
}

struct Report: Encodable {
    let protocolVersion: Int
    let engine: EngineIdentity
    let outcome: Outcome
}

/// The scratch copy of the source, cleared on every exit path once it exists.
nonisolated(unsafe) var scratchSource: URL?

private func removeScratch() {
    if let scratchSource { try? FileManager.default.removeItem(at: scratchSource) }
}

/// The speech stack ships with the OS and the diarizer with the package, so
/// the engine version names both.
func engineVersion() -> String {
    let version = ProcessInfo.processInfo.operatingSystemVersion
    return
        "\(version.majorVersion).\(version.minorVersion).\(version.patchVersion)+fluidaudio-\(fluidAudioVersion)"
}

/// stdout carries the identity and progress lines only, so a failure is a
/// stderr line plus the exit code.
func fail(_ note: String) -> Never {
    removeScratch()
    FileHandle.standardError.write(Data("tool-kit-audio-worker: \(note)\n".utf8))
    exit(70)
}

func environment(_ name: String) -> String? {
    ProcessInfo.processInfo.environment[name]
}

func isLowercaseSha256(_ value: String) -> Bool {
    value.utf8.count == 64
        && value.utf8.allSatisfy { (0x30...0x39).contains($0) || (0x61...0x66).contains($0) }
}

func hexDigest(_ digest: SHA256.Digest) -> String {
    digest.map { String(format: "%02x", $0) }.joined()
}

/// Writes the report and ends the process. `withoutOverwriting` is the
/// worker's `create_new`: a collision fails the run instead of publishing over
/// someone else's bytes.
func finish(_ outcome: Outcome, in staging: URL) -> Never {
    let report = Report(
        protocolVersion: protocolVersion,
        engine: EngineIdentity(
            name: engineName, version: engineVersion(),
            features: ["speech-analyzer", "diarization"]),
        outcome: outcome)
    do {
        try JSONEncoder().encode(report).write(
            to: staging.appendingPathComponent(reportFile), options: .withoutOverwriting)
    } catch {
        fail("\(reportFile) write failed: \(error)")
    }
    removeScratch()
    exit(0)
}
