// tool-kit-audio-worker: transcribe one recording into Markdown speaker turns.
//
// Spawned the way apps/converter/src/engines/vision.rs spawns
// tool-kit-vision-worker: cleared environment, staging directory as argv[1],
// attempt directory as the working directory, source bytes on stdin, stdout
// on /dev/null. The converter logs the last stderr line of a failed run, so a
// fail() note reaches converter.log. The report file is the only channel the
// converter acts on, so every decision has to land there. Exit 0 once a
// report is written, 70 on any failure.
//
// `--fetch diarizer <dir>` is the only path that downloads models, and the
// build scripts drive it. A job still lets the OS install SpeechAnalyzer locale
// assets on first use. A failure there comes back as speech_assets_unavailable.
//
// `--speech status` and `--speech install` let the desktop's Settings check for
// and fetch that locale model ahead of a job. Both print one JSON line, and
// install prints `progress <0...1>` lines before it.
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

// MARK: - Speech model

/// The transcriber a job builds: the env locale or the system one, matched to
/// what SpeechAnalyzer supports. Nil when it supports neither, beside the
/// locale that was asked for.
private func speechTranscriber() async -> (Locale, SpeechTranscriber?) {
    let requested =
        (environment(localeEnv).flatMap { $0.isEmpty ? nil : $0 })
        .map(Locale.init(identifier:)) ?? Locale.current
    guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: requested) else {
        return (requested, nil)
    }
    return (locale, SpeechTranscriber(locale: locale, preset: .transcription))
}

private struct SpeechChoice: Encodable {
    let locale: String
    let language: String
    let installed: Bool
}

private struct SpeechModel: Encodable {
    let state: String
    let locale: String
    /// In English, because the desktop's copy is.
    let language: String
    /// What an unset locale resolves to, for "Same as this Mac".
    let systemLocale: String
    let systemLanguage: String
    /// Every language SpeechAnalyzer offers, the ones on this Mac first.
    let choices: [SpeechChoice]
}

private func englishName(_ locale: Locale) -> String {
    Locale(identifier: "en").localizedString(forIdentifier: locale.identifier)
        ?? locale.identifier(.bcp47)
}

/// Unbuffered: `print` holds a pipe's output until exit, and the desktop reads
/// progress live.
private func emit(_ line: String) {
    FileHandle.standardOutput.write(Data("\(line)\n".utf8))
}

private func reportSpeechModel(_ locale: Locale, _ transcriber: SpeechTranscriber?) async -> Never {
    var state = "unsupported"
    if let transcriber {
        switch await AssetInventory.status(forModules: [transcriber]) {
        case .installed: state = "installed"
        case .downloading: state = "downloading"
        case .supported: state = "supported"
        case .unsupported: state = "unsupported"
        @unknown default: state = "unsupported"
        }
    }
    let system =
        await SpeechTranscriber.supportedLocale(equivalentTo: Locale.current) ?? Locale.current
    let installed = Set(await SpeechTranscriber.installedLocales.map { $0.identifier(.bcp47) })
    let choices = await SpeechTranscriber.supportedLocales
        .map {
            SpeechChoice(
                locale: $0.identifier(.bcp47), language: englishName($0),
                installed: installed.contains($0.identifier(.bcp47)))
        }
        .sorted { ($0.installed ? 0 : 1, $0.language) < ($1.installed ? 0 : 1, $1.language) }
    let model = SpeechModel(
        state: state, locale: locale.identifier(.bcp47), language: englishName(locale),
        systemLocale: system.identifier(.bcp47), systemLanguage: englishName(system),
        choices: choices)
    guard let json = try? JSONEncoder().encode(model) else { fail("speech model encode failed") }
    emit(String(decoding: json, as: UTF8.self))
    exit(0)
}

/// Apple gives each app a few locale slots (`maximumReservedLocales`), and an
/// install claims one. Someone who switches languages would run out and every
/// later install would fail, so before one, every slot but this language's and
/// the Mac's own goes back. A released model stays while another app uses it.
/// Nothing is released when the model is already here.
private func freeReservations(for locale: Locale, _ transcriber: SpeechTranscriber) async {
    guard await AssetInventory.status(forModules: [transcriber]) != .installed else { return }
    var keep = [locale.identifier(.bcp47)]
    if let system = await SpeechTranscriber.supportedLocale(equivalentTo: Locale.current) {
        keep.append(system.identifier(.bcp47))
    }
    for reserved in await AssetInventory.reservedLocales
    where !keep.contains(reserved.identifier(.bcp47)) {
        _ = await AssetInventory.release(reservedLocale: reserved)
    }
}

private func installSpeechModel() async -> Never {
    let (locale, transcriber) = await speechTranscriber()
    if let transcriber {
        await freeReservations(for: locale, transcriber)
        do {
            if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
                let ticker = Task {
                    var last = -1.0
                    while !Task.isCancelled {
                        let done = request.progress.fractionCompleted
                        if done != last {
                            emit("progress \(done)")
                            last = done
                        }
                        try? await Task.sleep(for: .milliseconds(250))
                    }
                }
                defer { ticker.cancel() }
                try await request.downloadAndInstall()
            }
        } catch {
            fail("speech model install failed: \(error)")
        }
    }
    await reportSpeechModel(locale, transcriber)
}

// MARK: - Job

/// Apple's on-device speech stack. The results arrive on a stream while
/// `analyzeSequence` feeds the file in, so a task collects them in parallel.
private func transcribe(_ file: AVAudioFile, with transcriber: SpeechTranscriber) async throws
    -> [Line]
{
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

    let (locale, found) = await speechTranscriber()
    guard let transcriber = found else { finish(.rejected(.localeUnsupported), in: staging) }
    await freeReservations(for: locale, transcriber)
    do {
        if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
            try await request.downloadAndInstall()
        }
    } catch {
        // The OS would not install the locale's model.
        finish(.rejected(.speechAssetsUnavailable), in: staging)
    }
    let lines: [Line]
    do {
        lines = try await transcribe(file, with: transcriber)
    } catch {
        // An analyzer or decoder error is an engine failure, not a missing
        // model, so it reads as a crash the way the diarizer's does.
        fail("transcription failed: \(error)")
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
if arguments == ["--speech", "status"] {
    let (locale, transcriber) = await speechTranscriber()
    await reportSpeechModel(locale, transcriber)
}
if arguments == ["--speech", "install"] {
    await installSpeechModel()
}
guard arguments.count == 1 else {
    fail(
        "usage: tool-kit-audio-worker <staging-directory> | --version | --fetch diarizer <dir>"
            + " | --speech status | --speech install")
}
await run(arguments[0])
