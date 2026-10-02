// tool-kit-audio-worker: transcribe one recording into Markdown speaker turns.
//
// Spawned the way apps/converter/src/engines/vision.rs spawns
// tool-kit-vision-worker: cleared environment, staging directory as argv[1],
// attempt directory as the working directory, source bytes on stdin, stdout
// and stderr on /dev/null. The report file is the only channel back, so every
// decision has to land there. Exit 0 once a report is written, 70 on any
// failure.
//
// `--fetch diarizer <dir>` is the only path that downloads models, and the
// build scripts drive it. A job still lets the OS install SpeechAnalyzer locale
// assets on first use; a failure there comes back as speech_assets_unavailable.
//
// The wire contract is crates/worker-protocol/src/audio.rs.

import AVFoundation
import CoreML
import CryptoKit
import Darwin
import FluidAudio
import Foundation
import Speech

// MARK: - Source

/// Copy stdin to `url` in 64 KB blocks, hashing as it goes: an hour of WAV is
/// hundreds of megabytes and never belongs in memory. Reads exactly `expected`
/// bytes, then proves EOF, so a short or long source is caught here.
private func streamStdinToFile(_ url: URL, expected: Int) -> String? {
    guard FileManager.default.createFile(atPath: url.path, contents: nil),
        let handle = FileHandle(forWritingAtPath: url.path)
    else { return nil }
    defer { try? handle.close() }

    var hasher = SHA256()
    var buffer = [UInt8](repeating: 0, count: 64 * 1024)
    var remaining = expected
    while remaining > 0 {
        let want = min(buffer.count, remaining)
        let count = buffer.withUnsafeMutableBytes { read(0, $0.baseAddress, want) }
        if count < 0 {
            if errno == EINTR { continue }
            return nil
        }
        if count == 0 { return nil }  // short source
        let chunk = buffer.withUnsafeBytes { Data($0.prefix(count)) }
        hasher.update(data: chunk)
        do { try handle.write(contentsOf: chunk) } catch { return nil }
        remaining -= count
    }
    var extra: UInt8 = 0
    while true {  // one more byte must not arrive
        let count = withUnsafeMutablePointer(to: &extra) { read(0, $0, 1) }
        if count < 0 && errno == EINTR { continue }
        if count != 0 { return nil }
        break
    }
    return hexDigest(hasher.finalize())
}

// MARK: - Fetch

private struct ManifestFile: Encodable {
    let path: String
    let byteLength: Int
    let sha256: String
}

private struct Manifest: Encodable {
    let repo: String
    let fluidAudioVersion: String
    let files: [ManifestFile]
}

/// Records what landed and for which FluidAudio version, so the build scripts
/// can tell a current set from a stale one without asking HuggingFace.
private func writeManifest(in directory: URL) {
    let manifestURL = directory.appendingPathComponent("manifest.json")
    guard
        let walker = FileManager.default.enumerator(
            at: directory, includingPropertiesForKeys: [.isRegularFileKey])
    else { fail("cannot walk \(directory.path)") }

    var files: [ManifestFile] = []
    for case let url as URL in walker {
        guard (try? url.resourceValues(forKeys: [.isRegularFileKey]))?.isRegularFile == true,
            url.standardizedFileURL != manifestURL.standardizedFileURL
        else { continue }
        guard let bytes = try? Data(contentsOf: url, options: .mappedIfSafe) else {
            fail("cannot read \(url.path)")
        }
        let prefix = directory.standardizedFileURL.path + "/"
        let relative =
            url.path.hasPrefix(prefix) ? String(url.path.dropFirst(prefix.count)) : url.path
        files.append(
            ManifestFile(
                path: relative, byteLength: bytes.count, sha256: hexDigest(SHA256.hash(data: bytes)))
        )
    }
    files.sort { $0.path < $1.path }

    let manifest = Manifest(
        repo: Repo.diarizer.rawValue, fluidAudioVersion: fluidAudioVersion, files: files)
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .withoutEscapingSlashes]
    do {
        try encoder.encode(manifest).write(to: manifestURL)
    } catch {
        fail("manifest write failed: \(error)")
    }
}

private func fetchDiarizer(into directory: URL) async -> Never {
    do {
        try await ModelHub.download(.diarizer, to: directory, variant: "offline") { progress in
            if case .downloading(let done, let total) = progress.phase {
                print("progress \(done)/\(total)")
            }
        }
    } catch {
        fail("diarizer download failed: \(error)")
    }
    writeManifest(in: directory)
    exit(0)
}

// MARK: - Job

/// Apple's on-device speech stack. The results arrive on a stream while
/// `analyzeSequence` feeds the file in, so a task collects them in parallel.
private func transcribe(_ file: AVAudioFile, locale: Locale) async throws -> [Line] {
    let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
    if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
        try await request.downloadAndInstall()
    }
    let analyzer = SpeechAnalyzer(modules: [transcriber])
    let collector = Task { () -> [Line] in
        var lines: [Line] = []
        for try await result in transcriber.results {
            lines.append(
                Line(
                    start: result.range.start.seconds,
                    end: (result.range.start + result.range.duration).seconds,
                    text: String(result.text.characters)))
        }
        return lines
    }
    _ = try await analyzer.analyzeSequence(from: file)
    try await analyzer.finalizeAndFinishThroughEndOfInput()
    return try await collector.value
}

/// FluidAudio's offline diarizer, from the staged CoreML bundles alone.
private func diarize(_ source: URL, directory: URL, speakers: Int?) async throws
    -> [TimedSpeakerSegment]
{
    let configuration = MLModelConfiguration()
    // Keeps the work off the GPU, and it was the fastest of the three settings.
    configuration.computeUnits = .cpuAndNeuralEngine
    let models = try await OfflineDiarizerModels.load(
        from: directory, configuration: configuration)

    var config = OfflineDiarizerConfig.default
    if let speakers { config = config.withSpeakers(exactly: speakers) }
    let manager = OfflineDiarizerManager(config: config)
    manager.initialize(models: models)

    // The URL overload resamples into a memory-mapped temp file, so a
    // four-hour recording never sits in memory as one array.
    // ponytail: a killed worker leaves that file in the per-user temp folder,
    // which macOS sweeps. Pass a scratch path if FluidAudio ever takes one.
    return try await manager.process(source).segments
}

private func run(_ stagingPath: String) async -> Never {
    // The job path never goes online, and an unset models directory is fatal
    // rather than a silent fallback to FluidAudio's own cache.
    ModelHub.offlineMode = true

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
    guard let mediaType = environment(mediaTypeEnv),
        let sourceExtension = sourceExtensions[mediaType]
    else { fail("\(mediaTypeEnv) must be one of \(sourceExtensions.keys.sorted())") }
    guard let diarizerPath = environment(diarizerDirEnv),
        (try? FileManager.default.attributesOfItem(atPath: diarizerPath))?[.type]
            as? FileAttributeType == .typeDirectory
    else { fail("\(diarizerDirEnv) must name a directory") }
    let diarizerDir = URL(fileURLWithPath: diarizerPath)

    let requestedSpeakers: Int?
    switch environment(speakerCountEnv) {
    case nil, "": requestedSpeakers = nil
    case let value?:
        guard let count = Int(value), count > 0 else {
            fail("\(speakerCountEnv) must be empty or a positive integer")
        }
        requestedSpeakers = count
    }

    let source = URL(fileURLWithPath: "source.\(sourceExtension)")
    scratchSource = source
    guard let digest = streamStdinToFile(source, expected: expectedBytes) else {
        fail("stdin did not hold exactly \(expectedBytes) bytes")
    }
    guard digest == expectedSha256 else { fail("source digest mismatch") }

    guard let file = try? AVAudioFile(forReading: source) else {
        finish(.rejected(.invalidAudio), in: staging)
    }
    guard file.length > 0 else { finish(.rejected(.noSpeechFound), in: staging) }
    let audioSeconds = Double(file.length) / file.processingFormat.sampleRate

    let requestedLocale = (environment(localeEnv).flatMap { $0.isEmpty ? nil : $0 })
        .map(Locale.init(identifier:)) ?? Locale.current
    guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: requestedLocale) else {
        finish(.rejected(.localeUnsupported), in: staging)
    }

    let lines: [Line]
    do {
        lines = try await transcribe(file, locale: locale)
    } catch {
        // Everything the speech stack can throw here comes back to the same
        // missing asset: the OS would not install the locale's model.
        finish(.rejected(.speechAssetsUnavailable), in: staging)
    }
    guard
        lines.contains(where: { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
    else { finish(.rejected(.noSpeechFound), in: staging) }

    let turns: [Turn]
    let speakersFound: Int
    if requestedSpeakers == 1 {
        // One voice needs no clustering, and the diarizer would still spend a
        // model load on it.
        turns = merge(lines) { _ in "Speaker 1" }
        speakersFound = 1
    } else {
        var segments: [TimedSpeakerSegment] = []
        do {
            segments = try await diarize(source, directory: diarizerDir, speakers: requestedSpeakers)
        } catch OfflineDiarizationError.noSpeechDetected {
            // The recogniser already found words. The diarizer's own speech
            // detector is stricter on short or sparse audio, and one speaker
            // is the honest answer there, not a crash.
        } catch {
            // A worker crash reads as a transient engine failure. A rejection
            // code would misfile a broken install as the user's audio.
            fail("diarization failed: \(error)")
        }
        let order = speakerOrder(segments)
        if let busiest = order.first {
            // A line no segment covers keeps the speaker of the line before
            // it, so the transcript never names a voice the count omits.
            var lastId = busiest
            turns = numbered(
                merge(lines) { line in
                    if let id = speaker(for: line, in: segments) { lastId = id }
                    return lastId
                }, order: order)
            speakersFound = Set(turns.map(\.speaker)).count
        } else {
            turns = merge(lines) { _ in "Speaker 1" }
            speakersFound = 1
        }
    }

    let bytes = Data(
        markdown(turns, guessedSpeakers: requestedSpeakers == nil ? speakersFound : nil).utf8)
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
                relativePath: markdownFile, byteLength: bytes.count,
                sha256: hexDigest(SHA256.hash(data: bytes))),
            Detail(
                audioSeconds: audioSeconds, speakersFound: speakersFound,
                speakerCountGuessed: requestedSpeakers == nil,
                locale: locale.identifier(.bcp47))),
        in: staging)
}

// MARK: - Entry

let arguments = Array(CommandLine.arguments.dropFirst())
if arguments == ["--version"] {
    FileHandle.standardOutput.write(Data("\(identityPrefix)\(engineVersion())\n".utf8))
    exit(0)
}
if arguments.count == 3, arguments[0] == "--fetch", arguments[1] == "diarizer" {
    await fetchDiarizer(into: URL(fileURLWithPath: arguments[2]))
}
guard arguments.count == 1 else {
    fail("usage: tool-kit-audio-worker <staging-directory> | --version | --fetch diarizer <dir>")
}
await run(arguments[0])
