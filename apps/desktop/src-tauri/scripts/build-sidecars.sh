#!/usr/bin/env bash
#
# Build the conversion sidecar and its assets, then stage them where the Tauri
# bundler expects them.
#
# `tauri-build` resolves `bundle.externalBin` inside the crate's build script,
# so a missing artifact fails `cargo check`, `cargo clippy`, `cargo test` and
# `pnpm tauri dev` with ResourcePathNotFound. That is why the root package.json
# runs this ahead of both `tauri` and `verify` rather than only before a
# release build.
#
# Eleven artifacts, in two places:
#
#   binaries/tool-kit-converter-aarch64-apple-darwin
#   binaries/tool-kit-pdf-worker-aarch64-apple-darwin
#   binaries/tool-kit-vision-worker-aarch64-apple-darwin
#   binaries/tool-kit-audio-worker-aarch64-apple-darwin
#   resources/pdf-inspector/bcmaps/
#   resources/pdf-inspector/pdf-inspector-MIT.txt
#   resources/pdf-inspector/adobe-bcmaps.txt
#   resources/fluidaudio/speaker-diarization-coreml/
#   resources/fluidaudio/FluidAudio-Apache-2.0.txt
#   resources/fluidaudio/speaker-diarization-CC-BY-4.0.txt
#   resources/fluidaudio/FluidAudio-ThirdPartyLicenses/
#
# The bundler strips the target suffix and writes the four binaries into
# Contents/MacOS, which is where the converter probes for its sibling workers,
# so no worker path env var is needed at runtime. The bcmaps and the diarizer
# models go to Contents/Resources instead: Contents/MacOS is sealed nested code
# and a data file there breaks the seal.
#
# Downloading the diarizer models is the one build-time network step, and it
# runs only when resources/fluidaudio holds no set for the FluidAudio version
# the worker was built against.
#
# Usage: apps/desktop/src-tauri/scripts/build-sidecars.sh [--debug]
#
# --debug builds the converter unoptimised, which is worth about four minutes
# to a session that is editing it. Never ship the result.

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
CRATE_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
REPO_ROOT="$(cd -- "${CRATE_DIR}/../../.." && pwd -P)"

# externalBin names the file by triple, so the build target is pinned rather
# than inferred. Get it wrong and the bundler looks for a file nobody wrote.
TARGET="aarch64-apple-darwin"
PROFILE="release"
CARGO_PROFILE_ARGS=("--release")

while [ $# -gt 0 ]; do
  case "$1" in
    # An empty array here would abort the script: macOS ships bash 3.2, and
    # 3.2's `set -u` treats "${CARGO_PROFILE_ARGS[@]}" on an empty array as an
    # unbound variable. --profile dev names the same build cargo already does
    # by default, so the array stays non-empty on both branches.
    --debug) PROFILE="debug"; CARGO_PROFILE_ARGS=("--profile" "dev"); shift ;;
    # Range ends at the blank line after the header, not a line number: the
    # number went stale twice while the header grew.
    -h|--help) sed -n '2,/^$/p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

step() { printf '\n== %s\n' "$*"; }
pass() { printf '   PASS %s\n' "$*"; }
fail() { printf '\nFAIL: %s\n' "$*" >&2; exit 1; }

BIN_DIR="${CRATE_DIR}/binaries"
RES_DIR="${CRATE_DIR}/resources/pdf-inspector"
RES_AUDIO="${CRATE_DIR}/resources/fluidaudio/speaker-diarization-coreml"
CONVERTER_MANIFEST="${REPO_ROOT}/apps/converter/Cargo.toml"

step "Host"
[ "$(uname -s)" = "Darwin" ] || fail "the sidecar is a macOS bundle input and builds on macOS only"
[ "$(uname -m)" = "arm64" ] || fail "this script pins ${TARGET}, and the host reports $(uname -m).
     Building a bundle for Apple Silicon from an Intel host needs a cross
     toolchain and a Swift target for the Vision worker, neither of which is
     set up here."
# The Vision worker needs macOS 26 for RecognizeDocumentsRequest, and an absent
# worker is not a build error to the converter: it logs one warning and
# advertises 18 media types instead of 24. So the gate is here, loud, rather
# than a skip that ships a DMG with no OCR.
MACOS_MAJOR="$(sw_vers -productVersion | cut -d. -f1)"
[ "${MACOS_MAJOR}" -ge 26 ] || fail "the Vision worker needs macOS 26, and this host runs $(sw_vers -productVersion).
     Shipping without it is a silent downgrade, not a smaller build, so this
     stops here instead of staging two of the three binaries."
pass "macOS $(sw_vers -productVersion) on ${TARGET}"

step "Converter (${PROFILE})"
cargo build --locked --target "${TARGET}" "${CARGO_PROFILE_ARGS[@]}" \
  --manifest-path "${CONVERTER_MANIFEST}" --bins

# cargo answers both questions this script has about the converter's
# dependency graph, so neither the target directory nor the pdf-inspector
# version is written down twice. A find by hardcoded version would no-op
# silently the day the dependency is bumped.
METADATA="$(cargo metadata --format-version 1 --locked --manifest-path "${CONVERTER_MANIFEST}" \
  | python3 -c '
import json, os, sys
meta = json.load(sys.stdin)
source = next((p["manifest_path"] for p in meta["packages"] if p["name"] == "pdf-inspector"), None)
if source is None:
    sys.exit("pdf-inspector is not in the converter dependency graph")
print(meta["target_directory"])
print(os.path.dirname(source))
')"
TARGET_DIR="$(printf '%s\n' "${METADATA}" | sed -n 1p)"
PDF_INSPECTOR_SRC="$(printf '%s\n' "${METADATA}" | sed -n 2p)"
[ -n "${TARGET_DIR}" ] && [ -n "${PDF_INSPECTOR_SRC}" ] \
  || fail "cargo metadata did not report a target directory and a pdf-inspector source"

BUILT="${TARGET_DIR}/${TARGET}/${PROFILE}"
for name in tool-kit-converter tool-kit-pdf-worker; do
  [ -x "${BUILT}/${name}" ] || fail "cargo reported success but ${BUILT}/${name} is missing"
done
pass "tool-kit-converter, tool-kit-pdf-worker"

step "Vision worker"
"${REPO_ROOT}/workers/vision/build.sh"
VISION_BIN="${REPO_ROOT}/workers/vision/bin/tool-kit-vision-worker"
[ -x "${VISION_BIN}" ] || fail "workers/vision/build.sh left no ${VISION_BIN}"
pass "tool-kit-vision-worker"

step "Audio worker"
"${REPO_ROOT}/workers/audio/build.sh"
AUDIO_BIN="${REPO_ROOT}/workers/audio/bin/tool-kit-audio-worker"
[ -x "${AUDIO_BIN}" ] || fail "workers/audio/build.sh left no ${AUDIO_BIN}"
pass "tool-kit-audio-worker"

step "Stage binaries"
mkdir -p "${BIN_DIR}"
# install replaces the file rather than writing through it, so a re-run cannot
# leave a half-written Mach-O behind and cannot invalidate a signature on a
# copy something else is holding open.
install -m 0755 "${BUILT}/tool-kit-converter" "${BIN_DIR}/tool-kit-converter-${TARGET}"
install -m 0755 "${BUILT}/tool-kit-pdf-worker" "${BIN_DIR}/tool-kit-pdf-worker-${TARGET}"
install -m 0755 "${VISION_BIN}" "${BIN_DIR}/tool-kit-vision-worker-${TARGET}"
install -m 0755 "${AUDIO_BIN}" "${BIN_DIR}/tool-kit-audio-worker-${TARGET}"
for name in tool-kit-converter tool-kit-pdf-worker tool-kit-vision-worker tool-kit-audio-worker; do
  pass "binaries/${name}-${TARGET}"
done

step "Stage pdf-inspector assets"
[ -d "${PDF_INSPECTOR_SRC}/external/bcmaps" ] \
  || fail "no external/bcmaps under ${PDF_INSPECTOR_SRC}"
mkdir -p "${RES_DIR}"
# Build the set beside the live one and swap, so an interrupted run leaves the
# previous copy whole. A partial copy is the failure that matters here:
# validate_cmaps checks four of the 169 files at startup, so a truncated set
# starts clean and only loses CJK text encoding later.
rm -rf "${RES_DIR}/bcmaps.staging"
cp -R "${PDF_INSPECTOR_SRC}/external/bcmaps" "${RES_DIR}/bcmaps.staging"
rm -rf "${RES_DIR}/bcmaps"
mv "${RES_DIR}/bcmaps.staging" "${RES_DIR}/bcmaps"
install -m 0644 "${PDF_INSPECTOR_SRC}/LICENSE" "${RES_DIR}/pdf-inspector-MIT.txt"
install -m 0644 "${PDF_INSPECTOR_SRC}/external/bcmaps/LICENSE" "${RES_DIR}/adobe-bcmaps.txt"
pass "resources/pdf-inspector/bcmaps ($(find "${RES_DIR}/bcmaps" -type f | wc -l | tr -d ' ') files)"
pass "resources/pdf-inspector/{pdf-inspector-MIT.txt,adobe-bcmaps.txt}"

step "Stage diarizer models"
mkdir -p "$(dirname "${RES_AUDIO}")"
# The manifest records the FluidAudio version the set was fetched for. Compare
# it against the worker's own pin, or a version bump ships the previous
# release's CoreML set against a worker compiled for the new one.
WANT_FLUIDAUDIO="$("${AUDIO_BIN}" --version | sed -n 's/.*fluidaudio-\([^[:space:]]*\).*/\1/p')"
[ -n "${WANT_FLUIDAUDIO}" ] \
  || fail "${AUDIO_BIN} --version printed no fluidaudio-<version> to stage the models against"
HAVE_FLUIDAUDIO="$(sed -n 's/.*"fluidAudioVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  "${RES_AUDIO}/manifest.json" 2>/dev/null || true)"
if [ "${HAVE_FLUIDAUDIO}" = "${WANT_FLUIDAUDIO}" ]; then
  pass "resources/fluidaudio/speaker-diarization-coreml (already staged, FluidAudio ${HAVE_FLUIDAUDIO})"
else
  # Same staging-and-swap as the bcmaps: an interrupted download leaves the
  # previous set whole rather than a half-fetched one the worker loads anyway.
  rm -rf "${RES_AUDIO}.staging"
  "${AUDIO_BIN}" --fetch diarizer "${RES_AUDIO}.staging"
  [ -f "${RES_AUDIO}.staging/manifest.json" ] \
    || fail "--fetch diarizer wrote no manifest.json under ${RES_AUDIO}.staging"
  rm -rf "${RES_AUDIO}"
  mv "${RES_AUDIO}.staging" "${RES_AUDIO}"
  pass "resources/fluidaudio/speaker-diarization-coreml (fetched for FluidAudio ${WANT_FLUIDAUDIO})"
fi
FLUIDAUDIO_SRC="${REPO_ROOT}/workers/audio/.build/checkouts/FluidAudio"
[ -f "${FLUIDAUDIO_SRC}/LICENSE" ] \
  || fail "no LICENSE in the FluidAudio checkout at ${FLUIDAUDIO_SRC}"
install -m 0644 "${FLUIDAUDIO_SRC}/LICENSE" "${RES_AUDIO%/*}/FluidAudio-Apache-2.0.txt"
cat > "${RES_AUDIO%/*}/speaker-diarization-CC-BY-4.0.txt" <<'NOTICE'
Speaker diarization CoreML models

Model:   FluidInference/speaker-diarization-coreml
Source:  https://huggingface.co/FluidInference/speaker-diarization-coreml
Licence: Creative Commons Attribution 4.0 International (CC BY 4.0)
         https://creativecommons.org/licenses/by/4.0/

Converted for the Apple Neural Engine from pyannote/speaker-diarization-community-1
(https://huggingface.co/pyannote/speaker-diarization-community-1), which carries
the same CC BY 4.0 licence. The FluidAudio SDK that loads them is Apache-2.0;
see FluidAudio-Apache-2.0.txt.
NOTICE
# The worker links FluidAudio's vendored fastcluster (BSD, which requires the
# notice to ship with a binary redistribution) and the VBx diarization code.
# Apache-2.0 alone does not cover them.
[ -d "${FLUIDAUDIO_SRC}/ThirdPartyLicenses" ] \
  || fail "no ThirdPartyLicenses in the FluidAudio checkout at ${FLUIDAUDIO_SRC}"
rm -rf "${RES_AUDIO%/*}/FluidAudio-ThirdPartyLicenses"
cp -R "${FLUIDAUDIO_SRC}/ThirdPartyLicenses" "${RES_AUDIO%/*}/FluidAudio-ThirdPartyLicenses"
chmod -R u+w,a+r "${RES_AUDIO%/*}/FluidAudio-ThirdPartyLicenses"
pass "resources/fluidaudio ($(find "${RES_AUDIO}" -type f | wc -l | tr -d ' ') model files, \
$(find "${RES_AUDIO%/*}" -maxdepth 2 -type f \( -name '*.txt' -o -name '*LICENSE*.md' \) | wc -l | tr -d ' ') licence texts)"

printf '\nRESULT: PASS\n'
