// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "tool-kit-audio-worker",
    // A string, not `.macOS(.v26)`: that enum case needs a SwiftPM newer than
    // the one the Command Line Tools ship.
    platforms: [.macOS("26.0")],
    dependencies: [
        // Pinned exactly: the CoreML layout the worker loads is coupled to
        // this version. Keep `fluidAudioVersion` in protocol.swift in step.
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.15.6")
    ],
    targets: [
        .executableTarget(
            name: "tool-kit-audio-worker",
            dependencies: [.product(name: "FluidAudio", package: "FluidAudio")]
        )
    ]
)
