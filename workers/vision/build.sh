#!/usr/bin/env bash
# Build the Vision worker and the pdf2png test tool in this directory.
#
# `swiftc` ships with the Command Line Tools, so this needs no Xcode, no
# SwiftPM, and no dependencies. `tool-kit-vision-worker` is the protocol worker.
# `pdf2png` turns a text PDF into a scan for the vision_worker tests.

set -euo pipefail

DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
OUT="${DIR}/bin"

if [ "$(uname -s)" != "Darwin" ]; then
  printf 'SKIP: the Vision tools build on macOS only\n' >&2
  exit 0
fi

# RecognizeDocumentsRequest is macOS 26. Below that these compile and then fail
# at the availability check, so refuse early with a readable reason.
MAJOR="$(sw_vers -productVersion | cut -d. -f1)"
if [ "${MAJOR}" -lt 26 ]; then
  printf 'FAIL: RecognizeDocumentsRequest needs macOS 26, found %s\n' "$(sw_vers -productVersion)" >&2
  exit 1
fi

mkdir -p "${OUT}"
# The worker deploys below the macOS 26 floor so its `#available` guard is
# reachable: carried to an older system it prints why and exits, rather than
# failing to launch at all.
swiftc -O -parse-as-library -target "$(uname -m)-apple-macos15.0" \
  "${DIR}/main.swift" "${DIR}/render.swift" -o "${OUT}/tool-kit-vision-worker"
swiftc -O "${DIR}/pdf2png.swift" -o "${OUT}/pdf2png"
printf 'Built %s/{tool-kit-vision-worker,pdf2png}\n' "${OUT}"
