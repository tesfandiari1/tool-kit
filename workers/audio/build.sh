#!/usr/bin/env bash
# Build the audio worker: Apple SpeechAnalyzer for the words, FluidAudio for
# the speakers.
#
# SwiftPM rather than `swiftc`, because FluidAudio is a package dependency.
# Only the Mach-O is installed: SwiftPM also emits
# FluidAudio_FluidAudio.bundle beside it, which is LuxTTS data the worker never
# reads and which would break the code seal in Contents/MacOS.

set -euo pipefail

DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
OUT="${DIR}/bin"

if [ "$(uname -s)" != "Darwin" ]; then
  printf 'SKIP: the audio worker builds on macOS only\n' >&2
  exit 0
fi

# SpeechAnalyzer is macOS 26, and the package declares that floor, so a lower
# host fails the build with a SwiftPM error instead of a readable reason.
MAJOR="$(sw_vers -productVersion | cut -d. -f1)"
if [ "${MAJOR}" -lt 26 ]; then
  printf 'FAIL: SpeechAnalyzer needs macOS 26, found %s\n' "$(sw_vers -productVersion)" >&2
  exit 1
fi

mkdir -p "${OUT}"
swift build --package-path "${DIR}" -c release 2>&1 | tail -n 40
BIN="$(swift build --package-path "${DIR}" -c release --show-bin-path)"
install -m 0755 "${BIN}/tool-kit-audio-worker" "${OUT}/tool-kit-audio-worker"
printf 'Built %s/tool-kit-audio-worker\n' "${OUT}"

# apps/converter/tests/audio_worker.rs spawns the worker against these models
# and skips every test without them, and nothing else stages them here.
# Re-fetch when the set was fetched for another FluidAudio version, the
# same check build-sidecars.sh makes. A stale set fails the worker at load.
MODELS="${DIR}/models/speaker-diarization-coreml"
WANT="$("${OUT}/tool-kit-audio-worker" --version | sed -n 's/.*fluidaudio-\([^[:space:]]*\).*/\1/p')"
HAVE="$(sed -n 's/.*"fluidAudioVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  "${MODELS}/manifest.json" 2>/dev/null || true)"
if [ -z "${WANT}" ] || [ "${HAVE}" != "${WANT}" ]; then
  rm -rf "${MODELS}.staging"
  "${OUT}/tool-kit-audio-worker" --fetch diarizer "${MODELS}.staging"
  rm -rf "${MODELS}"
  mv "${MODELS}.staging" "${MODELS}"
  printf 'Fetched %s for FluidAudio %s\n' "${MODELS}" "${WANT}"
fi
