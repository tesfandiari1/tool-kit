#!/usr/bin/env bash
#
# M4 local-corpus verification, no container required.
#
# Two suites, and together they are the corpus claim:
#   routing_policy   every case in evals/corpus-manifest.yaml driven through
#                    the real HTTP surface and the real worker
#   anydoc sweep     every format family the capabilities route advertises as
#                    AnyDoc-routable converts through the same surface
#
# Both run in-process under cargo test: no Docker, no token file, no data
# directory. cargo builds the pdf worker binary itself (CARGO_BIN_EXE).
#
# Usage: backend/scripts/verify-local-corpus.sh

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
BACKEND_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"

while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

log()  { printf '\n== %s\n' "$*"; }
note() { printf '   %s\n' "$*"; }
fail() { printf '\nFAIL: %s\n' "$*" >&2; exit 1; }

trap 'printf "\nERR at %s:%s\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

# ------------------------------------------------------------------ preflight

need() { command -v "$1" >/dev/null 2>&1 || fail "missing required tool: $1"; }
need cargo

[ -f "${BACKEND_DIR}/Cargo.toml" ] || fail "missing ${BACKEND_DIR}/Cargo.toml"
[ -f "${BACKEND_DIR}/evals/corpus-manifest.yaml" ] || fail "missing corpus manifest"

# --------------------------------------------------------------------- suites

log "Routing-policy corpus (tests/routing_policy.rs)"
cargo test --locked --manifest-path "${BACKEND_DIR}/Cargo.toml" --test routing_policy

log "AnyDoc advertised families (tests/http_contract.rs)"
cargo test --locked --manifest-path "${BACKEND_DIR}/Cargo.toml" \
  --test http_contract every_advertised_anydoc_family_converts

# -------------------------------------------------------------------- summary

log "Local corpus summary"
note "routing_policy: all corpus cases passed"
note "every_advertised_anydoc_family_converts: passed"
note "local-text PDF cases covered:"
note "  native-text-1-page"
note "  native-text-10-pages"
note "  sparse-cover-page"
note "  dense-table"
note "  two-column-text"

printf '\nRESULT: PASS\n'
