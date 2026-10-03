#!/usr/bin/env bash
#
# Property-based contract testing against the real service.
#
# Schemathesis reads `contract/http/openapi.yaml` and derives its own cases
# from it: hundreds of requests per run, checked for server errors, undeclared
# status codes, wrong content types, responses that do not match their schema,
# missing declared headers, and accepted data the contract says is invalid.
#
# It is here because `tests/http_contract.rs` cannot do this job. Those tests
# were written by whoever wrote the handlers, so they can only assert
# what their author already believed. This suite asserts the contract instead,
# and the first run it ever made found three admission rules the contract did
# not document.
#
# The run is namespaced away from every other stack: its own port, its own
# data root under `target/`, and a token generated per run. It never reads
# `deploy/docker/secrets/` and cannot reach the real converter-data volume.
#
# Usage: apps/converter/scripts/contract-fuzz.sh [--examples N] [--seed N] [--no-build]

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
CONVERTER_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
# The contract lives outside the crate: it describes the service, and the
# desktop client is generated from the same file.
REPO_ROOT="$(cd -- "${CONVERTER_DIR}/../.." && pwd -P)"

# Pinned. An unpinned tool is a gate that can change its mind between runs
# without a commit, and a new check arriving as a red CI job nobody asked for
# teaches people to distrust it.
SCHEMATHESIS_VERSION="4.29.0"

PORT="${TOOLKIT_FUZZ_PORT:-18082}"
BASE="http://127.0.0.1:${PORT}"
EXAMPLES=30
SEED=1
BUILD="yes"

while [ $# -gt 0 ]; do
  case "$1" in
    --examples) EXAMPLES="${2:?--examples needs a value}"; shift 2 ;;
    --seed) SEED="${2:?--seed needs a value}"; shift 2 ;;
    --no-build) BUILD="no"; shift ;;
    -h|--help) sed -n '2,/^$/p' "${BASH_SOURCE[0]}"; exit 0 ;;
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
need curl
need jq
need openssl
need uuidgen
need uvx

SPEC="${REPO_ROOT}/contract/http/openapi.yaml"
CONFIG="${REPO_ROOT}/contract/http/schemathesis.toml"
FIXTURE="${CONVERTER_DIR}/tests/fixtures/anydoc/text.docx"
DOCX_TYPE="application/vnd.openxmlformats-officedocument.wordprocessingml.document"
for f in "$SPEC" "$CONFIG" "$FIXTURE"; do
  [ -r "$f" ] || fail "missing required file: $f"
done

RUN_DIR="${CONVERTER_DIR}/target/contract-fuzz/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "${RUN_DIR}/data" "${RUN_DIR}/scratch"
TOKEN_FILE="${RUN_DIR}/token.txt"
openssl rand -hex 32 > "${TOKEN_FILE}"
chmod 600 "${TOKEN_FILE}"
TOKEN="$(cat "${TOKEN_FILE}")"

# --------------------------------------------------------------------- server

if [ "${BUILD}" = "yes" ]; then
  log "Build"
  cargo build --locked --manifest-path "${CONVERTER_DIR}/Cargo.toml" --bins
fi

SERVER_BIN="${CONVERTER_DIR}/target/debug/tool-kit-converter"
[ -x "${SERVER_BIN}" ] || fail "missing ${SERVER_BIN}, drop --no-build"

SERVER_PID=""
cleanup() {
  if [ -n "${SERVER_PID}" ] && kill -0 "${SERVER_PID}" 2>/dev/null; then
    kill "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
  fi
  # The run directory stays. It holds the server log and the full
  # Schemathesis output, which is the only record of why a run failed, and it
  # sits under target/ where `cargo clean` reaches it.
  note "run directory ${RUN_DIR}"
}
trap cleanup EXIT

log "Start ${BASE}"
TOOLKIT_CONVERTER_BIND_ADDR="127.0.0.1:${PORT}" \
TOOLKIT_CONVERTER_DATA_DIR="${RUN_DIR}/data" \
TOOLKIT_CONVERTER_TOKEN_FILE="${TOKEN_FILE}" \
TOOLKIT_CONVERTER_MAX_JOBS=16 \
RUST_LOG="${RUST_LOG:-tool_kit_converter=warn}" \
  "${SERVER_BIN}" > "${RUN_DIR}/server.log" 2>&1 &
SERVER_PID=$!

READY="no"
for _ in $(seq 1 60); do
  if [ "$(curl -s -o /dev/null -w '%{http_code}' "${BASE}/health/ready" || true)" = "200" ]; then
    READY="yes"; break
  fi
  kill -0 "${SERVER_PID}" 2>/dev/null || fail "server exited early, see ${RUN_DIR}/server.log"
  sleep 1
done
[ "${READY}" = "yes" ] || fail "server never became ready, see ${RUN_DIR}/server.log"
note "ready, pid ${SERVER_PID}"

# ----------------------------------------------------------------------- seed
#
# The four id-scoped operations need a conversion that exists and has
# published artifacts. Without one they answer 404 to every generated UUID and
# their 200 responses, which carry the schemas most worth checking, are never
# exercised at all.

log "Seed one conversion"
SEED_RESPONSE="${RUN_DIR}/seed.json"
curl -sS -X POST "${BASE}/api/v1/conversions" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Idempotency-Key: $(uuidgen)" \
  -F "clientRunId=$(uuidgen)" \
  -F "profile=standard" \
  -F "source=@${FIXTURE};type=${DOCX_TYPE}" \
  -o "${SEED_RESPONSE}" -w '%{http_code}' > "${RUN_DIR}/seed.status"
[ "$(cat "${RUN_DIR}/seed.status")" = "202" ] \
  || fail "seed submit returned $(cat "${RUN_DIR}/seed.status"), body: $(cat "${SEED_RESPONSE}")"

SEED_ID="$(jq -r '.data.id' < "${SEED_RESPONSE}")"
[ -n "${SEED_ID}" ] && [ "${SEED_ID}" != "null" ] || fail "seed response carried no id"

STATUS="unknown"
for _ in $(seq 1 60); do
  STATUS="$(curl -sS -H "Authorization: Bearer ${TOKEN}" "${BASE}/api/v1/conversions/${SEED_ID}" | jq -r '.data.status')"
  case "${STATUS}" in succeeded|failed|needs_remote) break ;; esac
  sleep 1
done
[ "${STATUS}" = "succeeded" ] || fail "seed conversion finished ${STATUS}, expected succeeded"

ARTIFACTS="$(curl -sS -H "Authorization: Bearer ${TOKEN}" "${BASE}/api/v1/conversions/${SEED_ID}/artifacts" | jq '.data | length')"
[ "${ARTIFACTS}" = "2" ] || fail "seed published ${ARTIFACTS} artifacts, expected 2"
note "${SEED_ID} succeeded with 2 artifacts"

# ----------------------------------------------------------------------- fuzz

# Two passes over the same suite, differing only in the id the four id-scoped
# operations are given. See the comment on `[parameters]` in schemathesis.toml:
# a present id reaches their 200 responses, an absent one reaches their 404s,
# and running only the first hides an undeclared 404 completely.
ABSENT_ID="$(uuidgen | tr 'A-Z' 'a-z')"
FUZZ_STATUS=0

# The pass runs as an `if` condition on purpose. Under an ERR trap the obvious
# spelling is wrong: the trap fires on the failing pipeline and its own printf
# overwrites PIPESTATUS, so the status read on the next line is always 0 and
# every run reports PASS. A condition fires no trap and `pipefail` gives the
# pipeline Schemathesis's own exit code past `tee`.
run_pass() {
  local label="$1" id="$2" out="${RUN_DIR}/schemathesis-$1.log"
  log "Schemathesis ${SCHEMATHESIS_VERSION}, ${label} id ${id}"
  # A missing value is a hard config error, so this export is what proves the
  # seed step above reached the run.
  export TOOLKIT_FUZZ_SEED_ID="${id}"
  # Run from the run directory. Schemathesis writes a crash cache beside its
  # working directory, and the repo root is not the place for it. Every path
  # handed to it is absolute, so only the cache moves.
  if ( cd "${RUN_DIR}" && uvx --from "schemathesis==${SCHEMATHESIS_VERSION}" st \
      --config-file "${CONFIG}" --no-color \
      run "${SPEC}" \
      --url "${BASE}" \
      --header "Authorization: Bearer ${TOKEN}" \
      --max-examples "${EXAMPLES}" \
      --seed "${SEED}" \
      --workers 1 \
      --max-response-time 30 \
      --generation-database ':memory:' ) \
      2>&1 | tee "${out}"; then
    note "${label}: passed"
  else
    FUZZ_STATUS=1
    note "${label}: FAILED"
  fi
}

run_pass present "${SEED_ID}"
run_pass absent "${ABSENT_ID}"

# ------------------------------------------------------------------ summary

log "Summary"
for pass_name in present absent; do
  grep -E '^ +[0-9]+ generated' "${RUN_DIR}/schemathesis-${pass_name}.log" \
    | sed "s/^ */   ${pass_name}: /" || true
done
note "seed ${SEED}, ${EXAMPLES} examples per operation, two passes"
note "spec ${SPEC}"

# Two warnings are expected and permanent, one per pass. Read a third as real.
#
#   present  POST /api/v1/conversions reports a schema validation mismatch,
#            because admission checks the container signature of the uploaded
#            bytes and generated bytes never carry one. The matching check is
#            scoped off in schemathesis.toml, but warnings are computed per run
#            rather than per operation, so they cannot be scoped the same way.
#   absent   the four id-scoped operations report missing test data, because
#            every one of them answers 404. That is the whole purpose of the
#            pass.
[ "${FUZZ_STATUS}" -eq 0 ] || fail "a Schemathesis pass failed, see ${RUN_DIR}/schemathesis-*.log"

printf '\nRESULT: PASS\n'
