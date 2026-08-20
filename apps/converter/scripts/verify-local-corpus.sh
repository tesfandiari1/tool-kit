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
# Usage: apps/converter/scripts/verify-local-corpus.sh

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
CONVERTER_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"

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

[ -f "${CONVERTER_DIR}/Cargo.toml" ] || fail "missing ${CONVERTER_DIR}/Cargo.toml"
MANIFEST="${CONVERTER_DIR}/evals/corpus-manifest.yaml"
[ -f "${MANIFEST}" ] || fail "missing corpus manifest"

# The five Phase 1 local-text cases. routing_policy below runs the whole
# corpus, which is a superset, so these names are what the summary cites
# rather than what it filters. Assert the manifest still carries them, or the
# summary would keep naming cases that had been renamed away.
PHASE1_CASES=(
  native-text-1-page
  native-text-10-pages
  sparse-cover-page
  dense-table
  two-column-text
)

for case_id in "${PHASE1_CASES[@]}"; do
  grep -qE "^[[:space:]]*-[[:space:]]+id:[[:space:]]+${case_id}[[:space:]]*$" "${MANIFEST}" \
    || fail "corpus manifest no longer carries the Phase 1 case: ${case_id}"
done

# --------------------------------------------------------------------- suites

log "Routing-policy corpus (tests/routing_policy.rs)"
cargo test --locked --manifest-path "${CONVERTER_DIR}/Cargo.toml" --test routing_policy

log "AnyDoc advertised families (tests/http_contract.rs)"
# A name filter that matches nothing is "ok. 0 passed" and exit 0, which
# pipefail cannot see, so assert the sweep actually ran.
ANYDOC_LOG="$(mktemp)"
trap 'rm -f "${ANYDOC_LOG}"' EXIT
cargo test --locked --manifest-path "${CONVERTER_DIR}/Cargo.toml" \
  --test http_contract every_advertised_anydoc_family_converts 2>&1 | tee "${ANYDOC_LOG}"
grep -qE '^test result: ok\. [1-9][0-9]* passed' "${ANYDOC_LOG}" \
  || fail "the AnyDoc sweep matched no test: every_advertised_anydoc_family_converts"

# -------------------------------------------------------------------- summary

log "Local corpus summary"
note "routing_policy: all corpus cases passed"
note "every_advertised_anydoc_family_converts: passed"
note "local-text PDF cases covered:"
for case_id in "${PHASE1_CASES[@]}"; do note "  ${case_id}"; done

printf '\nRESULT: PASS\n'
