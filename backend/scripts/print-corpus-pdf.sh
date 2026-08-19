#!/usr/bin/env bash
# Write a one-page corpus PDF from the in-process generator.
# Usage: backend/scripts/print-corpus-pdf.sh [output-path] [native|scanned]
# Default output: /tmp/toolkit-native.pdf, kind native.
#
# `scanned` produces the image-only page acceptance step D.6 needs, which
# must finish needs_remote under local_only rather than succeed quietly.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
BACKEND_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
OUT="${1:-/tmp/toolkit-native.pdf}"
KIND="${2:-native}"

case "${KIND}" in
  native|scanned) ;;
  *) printf 'FAIL: kind must be native or scanned, got %s\n' "${KIND}" >&2; exit 2 ;;
esac

export TOOLKIT_CORPUS_PDF_OUT="${OUT}"
export TOOLKIT_CORPUS_PDF_KIND="${KIND}"
cargo test --locked --manifest-path "${BACKEND_DIR}/Cargo.toml" \
  --test routing_policy support::corpus::tests::write_native_pdf_fixture_to_env \
  -- --exact --nocapture

# cargo exits 0 when --exact matches nothing, so check the file before claiming it.
[ -s "${OUT}" ] || { printf 'FAIL: %s was not written\n' "${OUT}" >&2; exit 1; }
printf 'Wrote %s (%s bytes)\n' "${OUT}" "$(wc -c < "${OUT}" | tr -d ' ')"
