// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "tool-kit-audio-worker",
    platforms: [.macOS(.v26)],
    dependencies: [
        // Pinned exactly: the CoreML layout the worker loads is coupled to
        // this version. Keep `fluidAudioVersion` in protocol.swift in step.
        // `traits: []` drops the NeMo text-normalization engine, which only
        // TTS uses.
        .package(
            url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.17.5", traits: [])
    ],
    targets: [
        .executableTarget(
            name: "tool-kit-audio-worker",
            dependencies: [.product(name: "FluidAudio", package: "FluidAudio")]
        )
    ]
)
