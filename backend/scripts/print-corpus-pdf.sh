#!/usr/bin/env bash
# Write a one-page native-text PDF from the in-process corpus generator.
# Usage: backend/scripts/print-corpus-pdf.sh [output-path]
# Default output: /tmp/toolkit-native.pdf

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
BACKEND_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
OUT="${1:-/tmp/toolkit-native.pdf}"

export TOOLKIT_CORPUS_PDF_OUT="${OUT}"
cargo test --locked --manifest-path "${BACKEND_DIR}/Cargo.toml" \
  --test routing_policy support::corpus::tests::write_native_pdf_fixture_to_env \
  -- --exact --nocapture

printf 'Wrote %s\n' "${OUT}"
