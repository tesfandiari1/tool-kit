#!/usr/bin/env bash
# Build the Vision worker and the development tools in this directory.
#
# `swiftc` ships with the Command Line Tools, so this needs no Xcode, no
# SwiftPM, and no dependencies. `tk-vision` and `pdf2png` judge Vision's output
# on real documents; `tool-kit-vision-worker` is the protocol worker itself.

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
# Both binaries compile `render.swift` so the preview tool cannot print
# markdown the worker would not publish.
swiftc -O -parse-as-library -target "$(uname -m)-apple-macos15.0" \
  "${DIR}/main.swift" "${DIR}/render.swift" -o "${OUT}/tool-kit-vision-worker"
swiftc -O -parse-as-library \
  "${DIR}/tk-vision.swift" "${DIR}/render.swift" -o "${OUT}/tk-vision"
swiftc -O "${DIR}/pdf2png.swift" -o "${OUT}/pdf2png"
printf 'Built %s/{tool-kit-vision-worker,tk-vision,pdf2png}\n' "${OUT}"
