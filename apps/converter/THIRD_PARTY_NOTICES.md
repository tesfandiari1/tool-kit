# Third-party runtime notices

The M1 container includes `pdf-inspector` 1.15.0 under the MIT License and its
bundled Adobe CMap resources under their separate BSD-style notice. The image
stores the complete notices at:

```text
/usr/share/licenses/pdf-inspector/pdf-inspector-MIT.txt
/usr/share/licenses/pdf-inspector/adobe-bcmaps.txt
```

The dependency is compiled with default features disabled. The runtime image
does not include the optional OCR, rendering, model-cache, or model-download
stack.

This file is not yet the complete transitive Rust license inventory. M1 is a
development-only image. CVR-076 requires an SBOM, full transitive notices, and a
current advisory review before distribution or LAN deployment; that review must
also resolve or explicitly block on the unmaintained `ttf-parser 0.25.1`
advisory RUSTSEC-2026-0192.

## Audio worker

The bundled `tool-kit-audio-worker` links FluidAudio 0.15.6 under the Apache
License 2.0. The app ships the complete notice at
`Contents/Resources/fluidaudio/FluidAudio-Apache-2.0.txt`. FluidAudio is a
macOS build input only: the Linux container has no audio engine.

FluidAudio links the prebuilt `NemoTextProcessing` xcframework, the
`text-processing-rs` build of NVIDIA NeMo Text Processing, also under the
Apache License 2.0.

It also links vendored fastcluster (BSD 2-Clause, which requires the notice to
ship with a binary redistribution) and the VBx diarization code (Apache License
2.0, copyright BUT Speech@FIT). Both notices, and NeMo's, ship at
`Contents/Resources/fluidaudio/FluidAudio-ThirdPartyLicenses/`.

The speaker diarization models are `FluidInference/speaker-diarization-coreml`
under Creative Commons Attribution 4.0 International, converted for the Apple
Neural Engine from `pyannote/speaker-diarization-community-1`, which carries the
same licence. The attribution ships beside the models at
`Contents/Resources/fluidaudio/speaker-diarization-CC-BY-4.0.txt`.
