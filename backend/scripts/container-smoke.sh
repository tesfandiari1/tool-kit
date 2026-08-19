#!/usr/bin/env bash
#
# M2 release smoke against the built container.
#
# Two runs, and the difference between them is the whole point:
#   graceful  submit -> SIGTERM stop -> start -> poll -> download -> verify hashes
#   sigkill   submit -> hold in converting_local -> SIGKILL -> start -> recover
#
# Everything is namespaced away from a developer's stack: its own Compose
# project (so its own volume), its own port, its own image tag, and its own
# bootstrap token. It never reads or writes backend/secrets/ and it can never
# reach the real converter-data volume.
#
# Usage: backend/scripts/container-smoke.sh [--phase graceful|sigkill|all] [--keep] [--no-build]
#
# The image build is the only part that can saturate the machine, and the
# default builder runs inside the Docker VM where `docker update` cannot reach
# it. To cap it, build through a container-driver builder instead:
#
#   docker buildx create --name toolkit-capped --driver docker-container --bootstrap
#   docker update --cpus 4 buildx_buildkit_toolkit-capped0
#   TOOLKIT_SMOKE_BUILDER=toolkit-capped backend/scripts/container-smoke.sh
#
# Measured on a 12-core host: the builder holds at ~405% CPU and host load stays
# near 4, leaving two thirds of the machine free. The running service is already
# capped by `cpus: 2.0` in compose.yaml, so only the build needed this.

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
BACKEND_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"

# A developer's exported shell or a gitignored backend/.env must not bend the
# run. Compose interpolation reads the environment, so clear the names it uses.
unset TOOLKIT_CONVERTER_PORT TOOLKIT_CONVERTER_TOKEN_FILE RUST_LOG || true

PROJECT="tool-kit-converter-smoke"
IMAGE="tool-kit-converter:smoke"
PORT="${TOOLKIT_SMOKE_PORT:-18080}"
BASE="http://127.0.0.1:${PORT}"
PHASE="all"
KEEP="no"
BUILD="yes"

# The exact bytes tests/support/mod.rs builds with clean_pdf(). Pinned so an
# edit to the generator below fails here instead of failing later as a
# confusing needs_remote.
FIXTURE_BYTES=668
FIXTURE_SHA=8e31b03a650dd6034c15557c52108392adf5221350f234cdf2679dc7c9cabf35

while [ $# -gt 0 ]; do
  case "$1" in
    --phase) PHASE="${2:?--phase needs a value}"; shift 2 ;;
    --keep) KEEP="yes"; shift ;;
    --no-build) BUILD="no"; shift ;;
    -h|--help) sed -n '2,27p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$PHASE" in graceful|sigkill|all) ;; *) echo "bad --phase: $PHASE" >&2; exit 2 ;; esac

RUN_DIR="${BACKEND_DIR}/target/container-smoke/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "${RUN_DIR}/stub" "${RUN_DIR}/download-1" "${RUN_DIR}/download-2" "${RUN_DIR}/download-3" "${RUN_DIR}/download-4"
EVIDENCE="${RUN_DIR}/evidence.md"

log()  { printf '\n== %s\n' "$*"; }
note() { printf '   %s\n' "$*"; }
fail() { printf '\nFAIL: %s\n' "$*" >&2; exit 1; }

assert_eq() {
  local name="$1" actual="$2" expected="$3"
  if [ "$actual" != "$expected" ]; then
    fail "${name}: expected '${expected}', got '${actual}'"
  fi
  printf -- '- PASS %s: `%s`\n' "$name" "$actual" >> "$EVIDENCE"
  note "PASS ${name} = ${actual}"
}

assert_json() { assert_eq "$1" "$(jq -r "$2" < "$3")" "$4"; }

trap 'printf "\nERR at %s:%s\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

# ---------------------------------------------------------------- preflight

need() { command -v "$1" >/dev/null 2>&1 || fail "missing required tool: $1"; }
need docker; need curl; need jq; need openssl; need awk; need sed
docker info >/dev/null 2>&1 || fail "the Docker daemon is not reachable"
docker compose version >/dev/null 2>&1 || fail "docker compose v2 is required"
[ -f "${BACKEND_DIR}/compose.yaml" ] || fail "missing ${BACKEND_DIR}/compose.yaml"
[ -f "${BACKEND_DIR}/Dockerfile" ] || fail "missing ${BACKEND_DIR}/Dockerfile"

if command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | awk '{print $1}'; }
else
  fail "need shasum or sha256sum"
fi

if command -v uuidgen >/dev/null 2>&1; then
  newuuid() { uuidgen | tr 'A-Z' 'a-z'; }
elif command -v python3 >/dev/null 2>&1; then
  newuuid() { python3 -c 'import uuid;print(uuid.uuid4())'; }
else
  fail "need uuidgen or python3 for client run ids"
fi

if command -v nc >/dev/null 2>&1 && nc -z 127.0.0.1 "$PORT" >/dev/null 2>&1; then
  fail "port ${PORT} is already in use. Set TOOLKIT_SMOKE_PORT to something free."
fi

# ------------------------------------------------------------- token + files

umask 077
openssl rand -hex 32 > "${RUN_DIR}/bootstrap-token.txt"
TOKEN="$(tr -d '\r\n' < "${RUN_DIR}/bootstrap-token.txt")"

# Compose reads this for the `secrets:` file source only. The container's own
# TOOLKIT_CONVERTER_TOKEN_FILE is a literal in compose.yaml with no
# interpolation, so exporting this cannot change the in-container path.
export TOOLKIT_CONVERTER_TOKEN_FILE="${RUN_DIR}/bootstrap-token.txt"
export TOOLKIT_CONVERTER_PORT="${PORT}"

: > "${RUN_DIR}/compose.env"

cat > "${RUN_DIR}/compose.smoke.yaml" <<YAML
services:
  converter:
    image: ${IMAGE}
    # Compose treats a kill as a manual stop, but relying on that during a
    # SIGKILL smoke is a flake waiting to happen.
    restart: "no"
YAML

# Holds a job in converting_local for as long as we want. The engine verifies
# the worker identity string byte for byte before using it, and it spawns with
# env_clear(), so PATH is empty inside: `echo` is a builtin and `sleep` has to
# be absolute.
cat > "${RUN_DIR}/stub/hold-worker.sh" <<'STUB'
#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 'tool-kit-pdf-worker protocol=2 pdf-inspector=1.15.0'
  exit 0
fi
exec /bin/sleep 3600
STUB
chmod 0755 "${RUN_DIR}/stub/hold-worker.sh"

# Relative paths in a Compose file resolve against the project directory, which
# is the first -f file's directory, so the bind mount has to be absolute.
cat > "${RUN_DIR}/compose.hold.yaml" <<YAML
services:
  converter:
    environment:
      TOOLKIT_CONVERTER_PDF_WORKER_PATH: /opt/smoke/hold-worker.sh
      # The 60s default fires as a TERMINAL timeout failure, which would end the
      # job instead of holding it.
      TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS: "3600"
    volumes:
      - ${RUN_DIR}/stub:/opt/smoke:ro
YAML

BASE_ARGS=(-f "${BACKEND_DIR}/compose.yaml")
SMOKE=(-f "${RUN_DIR}/compose.smoke.yaml")
HOLD=(-f "${RUN_DIR}/compose.hold.yaml")

compose() {
  docker compose --env-file "${RUN_DIR}/compose.env" -p "$PROJECT" "${BASE_ARGS[@]}" "$@"
}

# ------------------------------------------------------------------ teardown

assert_smoke_volume() {
  [ "$PROJECT" != "tool-kit-converter" ] || fail "refusing to touch the real project"
  local vol
  while read -r vol; do
    [ -z "$vol" ] && continue
    case "$vol" in
      tool-kit-converter-smoke_*) ;;
      *) fail "refusing to remove volume outside the smoke project: ${vol}" ;;
    esac
  done < <(docker volume ls --filter "label=com.docker.compose.project=${PROJECT}" --format '{{.Name}}')
}

cleanup() {
  rc=$?
  compose "${SMOKE[@]}" logs --no-color --timestamps converter > "${RUN_DIR}/container.log" 2>&1 || true
  compose "${SMOKE[@]}" ps -a >> "${RUN_DIR}/container.log" 2>&1 || true
  if [ "$rc" -ne 0 ]; then
    printf '\n--- last 120 log lines ---\n' >&2
    tail -n 120 "${RUN_DIR}/container.log" >&2 || true
  fi
  if [ "$KEEP" = "yes" ]; then
    printf '\n--keep: leaving the stack up. Tear down with:\n  docker compose -p %s -f %s -f %s down -v\n' \
      "$PROJECT" "${BACKEND_DIR}/compose.yaml" "${RUN_DIR}/compose.smoke.yaml"
  else
    assert_smoke_volume
    compose "${SMOKE[@]}" down -v --remove-orphans --timeout 45 >/dev/null 2>&1 || true
  fi
  printf '\nEvidence: %s\n' "$EVIDENCE"
  exit "$rc"
}
trap cleanup EXIT

# ------------------------------------------------------------------- fixture

make_pdf() {
  local out="$1" content="${RUN_DIR}/content.bin" len i off xref
  printf 'BT\n/F1 18 Tf\n72 720 Td\n(Clean PDF Fixture) Tj\n0 -24 Td\n(Second native text line) Tj\n0 -24 Td\n(Third native text line) Tj\nET\n' > "$content"
  len=$(wc -c < "$content" | tr -d ' ')

  : > "$out"
  printf '%%PDF-1.4\n' >> "$out"

  local -a objects offsets
  objects=(
    '<< /Type /Catalog /Pages 2 0 R >>'
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>'
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>'
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'
  )
  i=1
  for obj in "${objects[@]}"; do
    offsets+=("$(wc -c < "$out" | tr -d ' ')")
    printf '%d 0 obj\n%s\nendobj\n' "$i" "$obj" >> "$out"
    i=$((i + 1))
  done

  offsets+=("$(wc -c < "$out" | tr -d ' ')")
  printf '5 0 obj\n<< /Length %d >>\nstream\n' "$len" >> "$out"
  cat "$content" >> "$out"
  printf 'endstream\nendobj\n' >> "$out"

  xref=$(wc -c < "$out" | tr -d ' ')
  printf 'xref\n0 6\n0000000000 65535 f \n' >> "$out"
  for off in "${offsets[@]}"; do
    printf '%010d 00000 n \n' "$off" >> "$out"
  done
  printf 'trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n' "$xref" >> "$out"
}

FIXTURE="${RUN_DIR}/fixture.pdf"
make_pdf "$FIXTURE"
assert_eq "fixture bytes" "$(wc -c < "$FIXTURE" | tr -d ' ')" "$FIXTURE_BYTES"
assert_eq "fixture sha256" "$(sha256 "$FIXTURE")" "$FIXTURE_SHA"
SOURCE_SHA="$(sha256 "$FIXTURE")"

# The AnyDoc half of the image. The PDF above is synthesized so the script stays
# self-contained, but a real docx cannot be; this is the same vendored fixture
# the contract suite uses, so a container run and `cargo test` prove one engine.
DOCX_FIXTURE="${BACKEND_DIR}/tests/fixtures/anydoc/text.docx"
[ -r "$DOCX_FIXTURE" ] || fail "missing AnyDoc fixture ${DOCX_FIXTURE}"
DOCX_SHA="$(sha256 "$DOCX_FIXTURE")"
DOCX_BYTES="$(wc -c < "$DOCX_FIXTURE" | tr -d ' ')"

# ---------------------------------------------------------------- http helpers

api() {
  local method="$1" path="$2" out="$3" hdr="$4"; shift 4
  # `Expect:` disables 100-continue. curl turns it on for a body over 1KB, and
  # the extra "HTTP/1.1 100 Continue" block would make status_of read 100.
  curl -sS -X "$method" -D "$hdr" -o "$out" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Expect:" \
    "$@" "${BASE}${path}"
}

status_of() { awk 'NR==1{print $2}' "$1" | tr -d '\r'; }
header_of() { awk -v k="$(echo "$2" | tr 'A-Z' 'a-z')" 'tolower($1)==k":"{sub(/^[^:]*: */,"");print}' "$1" | tr -d '\r'; }

submit() {
  local key="$1" run_id="$2" out="$3" hdr="$4"
  api POST /api/v1/conversions "$out" "$hdr" \
    -H "Idempotency-Key: ${key}" \
    -F "clientRunId=${run_id}" \
    -F "profile=standard" \
    -F "source=@${FIXTURE};type=application/pdf"
}

submit_docx() {
  local key="$1" run_id="$2" out="$3" hdr="$4"
  api POST /api/v1/conversions "$out" "$hdr" \
    -H "Idempotency-Key: ${key}" \
    -F "clientRunId=${run_id}" \
    -F "profile=standard" \
    -F "source=@${DOCX_FIXTURE};type=application/vnd.openxmlformats-officedocument.wordprocessingml.document"
}

wait_ready() {
  local deadline=$((SECONDS + ${1:-120}))
  while [ $SECONDS -lt $deadline ]; do
    if curl -sS --fail "${BASE}/health/ready" -o "${RUN_DIR}/ready.json" 2>/dev/null; then return 0; fi
    sleep 1
  done
  fail "the service never became ready"
}

poll_until() {
  local job="$1" want="$2" limit="$3" out="${RUN_DIR}/poll.json" hdr="${RUN_DIR}/poll.h"
  local deadline=$((SECONDS + limit)) status
  while [ $SECONDS -lt $deadline ]; do
    api GET "/api/v1/conversions/${job}" "$out" "$hdr"
    status="$(jq -r '.data.status' < "$out")"
    if [ "$status" = "$want" ]; then return 0; fi
    case "$status" in
      failed|needs_remote)
        fail "job ${job} reached ${status} ($(jq -rc '.data.failure' < "$out")) while waiting for ${want}" ;;
    esac
    sleep 1
  done
  fail "job ${job} stayed at ${status} instead of reaching ${want} within ${limit}s"
}

container_state() {
  local cid; cid="$(compose "$@" ps -a -q converter)"
  [ -n "$cid" ] || fail "no converter container found"
  docker inspect -f '{{.State.ExitCode}} {{.State.Running}} {{.State.OOMKilled}}' "$cid"
}

# Downloads one artifact and checks the bytes against the ledger, the ETag, and
# Content-Length. The served hash is recomputed from the file on read, so a
# matching ETag proves the database row and the bytes on the volume still agree.
verify_artifact() {
  local job="$1" kind="$2" dest="$3" listing="$4"
  local hdr="${RUN_DIR}/art.h" expected_sha expected_len local_sha
  expected_sha="$(jq -r --arg k "$kind" '.data[]|select(.kind==$k)|.sha256' < "$listing")"
  expected_len="$(jq -r --arg k "$kind" '.data[]|select(.kind==$k)|.byteLength' < "$listing")"
  api GET "/api/v1/conversions/${job}/artifacts/${kind}" "$dest" "$hdr"
  assert_eq "${kind} download status" "$(status_of "$hdr")" "200"
  local_sha="$(sha256 "$dest")"
  assert_eq "${kind} bytes match the ledger" "$local_sha" "$expected_sha"
  assert_eq "${kind} content-length" "$(header_of "$hdr" content-length)" "$expected_len"
  assert_eq "${kind} etag" "$(header_of "$hdr" etag)" "\"sha256-${local_sha}\""
}

verify_bundle() {
  local job="$1" dir="$2" listing="${RUN_DIR}/artifacts.json" hdr="${RUN_DIR}/artifacts.h"
  api GET "/api/v1/conversions/${job}/artifacts" "$listing" "$hdr"
  assert_eq "artifact listing status" "$(status_of "$hdr")" "200"
  assert_json "artifact count" '.data|length' "$listing" "2"
  verify_artifact "$job" markdown "${dir}/result.md" "$listing"
  verify_artifact "$job" manifest "${dir}/manifest.json" "$listing"

  local m="${dir}/manifest.json"
  assert_json "manifest schemaVersion" '.schemaVersion' "$m" "1"
  assert_json "manifest jobId" '.jobId' "$m" "$job"
  assert_json "manifest route" '.route.kind' "$m" "local_pdf"
  assert_json "manifest reason codes" '.route.reasonCodes|join(",")' "$m" "native_text_pdf"
  assert_json "manifest engine version" '.engine.version' "$m" "1.15.0"
  assert_json "manifest pdf type" '.document.pdfType' "$m" "text_based"
  # The host computed this before the bytes ever entered the container.
  assert_json "manifest source sha256" '.source.sha256' "$m" "$SOURCE_SHA"
  assert_json "manifest source byteLength" '.source.byteLength' "$m" "$FIXTURE_BYTES"
  assert_json "manifest output sha256" '.output.sha256' "$m" "$(sha256 "${dir}/result.md")"
  if grep -q 'fixture.pdf' "$m"; then fail "the manifest leaked the uploaded filename"; fi
  printf -- '- PASS manifest carries no source filename\n' >> "$EVIDENCE"
}

# ------------------------------------------------------------------ evidence

{
  printf '# Container smoke evidence\n\n'
  printf -- '- date (UTC): `%s`\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf -- '- commit: `%s`\n' "$(git -C "$BACKEND_DIR" rev-parse HEAD 2>/dev/null || echo unknown)"
  # The build context is the working tree, not the commit. Say so, or evidence
  # taken over a dirty tree reads as if it covered the commit it names.
  printf -- '- tree: `%s`\n' \
    "$(test -n "$(git -C "$BACKEND_DIR" status --porcelain -- "$BACKEND_DIR" 2>/dev/null)" \
       && echo 'dirty (image built from uncommitted working tree)' || echo clean)"
  printf -- '- docker: `%s`\n' "$(docker version --format '{{.Server.Version}}')"
  printf -- '- compose: `%s`\n' "$(docker compose version --short)"
  printf -- '- phase: `%s`\n\n' "$PHASE"
} > "$EVIDENCE"

log "Preparing ${IMAGE} (project ${PROJECT}, port ${PORT})"
assert_smoke_volume
compose "${SMOKE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true

if [ "$BUILD" = "no" ]; then
  docker image inspect "$IMAGE" >/dev/null 2>&1 || fail "--no-build needs ${IMAGE} to exist already"
  note "reusing the existing ${IMAGE}"
elif [ -n "${TOOLKIT_SMOKE_BUILDER:-}" ]; then
  note "building through capped builder ${TOOLKIT_SMOKE_BUILDER}"
  docker buildx build --builder "${TOOLKIT_SMOKE_BUILDER}" --load -t "$IMAGE" "$BACKEND_DIR"
else
  compose "${SMOKE[@]}" build
fi
printf -- '- image id: `%s`\n' "$(docker image inspect -f '{{.Id}}' "$IMAGE")" >> "$EVIDENCE"

MD_SHA=""

# ------------------------------------------------------------------- smoke A

if [ "$PHASE" = "all" ] || [ "$PHASE" = "graceful" ]; then
  log "Smoke A: graceful restart"
  compose "${SMOKE[@]}" up -d --wait --wait-timeout 300
  wait_ready 120

  api GET /health/live "${RUN_DIR}/live.json" "${RUN_DIR}/live.h"
  assert_eq "health/live" "$(status_of "${RUN_DIR}/live.h")" "200"
  api GET /health/ready "${RUN_DIR}/ready.json" "${RUN_DIR}/ready.h"
  assert_eq "health/ready" "$(status_of "${RUN_DIR}/ready.h")" "200"
  assert_json "readiness database" '.checks.database' "${RUN_DIR}/ready.json" "ok"
  assert_json "readiness dataRoot" '.checks.dataRoot' "${RUN_DIR}/ready.json" "ok"
  assert_json "readiness worker" '.checks.worker' "${RUN_DIR}/ready.json" "ok"

  api GET /api/v1/capabilities "${RUN_DIR}/capabilities.json" "${RUN_DIR}/capabilities.h"
  assert_eq "capabilities" "$(status_of "${RUN_DIR}/capabilities.h")" "200"
  # The two CVR-027 contract corrections, confirmed against the real container.
  assert_json "durability" '.data.conversion.durability' "${RUN_DIR}/capabilities.json" "persistent"
  assert_json "maxEphemeralJobs is gone" '.data.conversion.limits.maxEphemeralJobs' \
    "${RUN_DIR}/capabilities.json" "null"
  assert_json "maxActiveJobs" '.data.conversion.limits.maxActiveJobs|type' \
    "${RUN_DIR}/capabilities.json" "number"

  submit "smoke-graceful-0001" "$(newuuid)" "${RUN_DIR}/a-submit.json" "${RUN_DIR}/a-submit.h"
  assert_eq "submit status" "$(status_of "${RUN_DIR}/a-submit.h")" "202"
  assert_eq "submit replayed" "$(header_of "${RUN_DIR}/a-submit.h" idempotency-replayed)" "false"
  JOB_A="$(jq -r '.data.id' < "${RUN_DIR}/a-submit.json")"
  printf -- '- graceful job: `%s`\n' "$JOB_A" >> "$EVIDENCE"

  log "Stopping gracefully right after the 202"
  # -t 45 is load-bearing. Docker's default is 10s but the service's own
  # shutdown grace is 30s, so a default stop would SIGKILL it mid-drain and
  # silently turn this into the other smoke.
  compose "${SMOKE[@]}" stop -t 45 converter
  assert_eq "graceful container state" "$(container_state "${SMOKE[@]}")" "0 false false"

  compose "${SMOKE[@]}" up -d --wait --wait-timeout 300
  wait_ready 120
  poll_until "$JOB_A" succeeded 180
  assert_json "graceful job status" '.data.status' "${RUN_DIR}/poll.json" "succeeded"

  verify_bundle "$JOB_A" "${RUN_DIR}/download-1"
  MD_SHA="$(sha256 "${RUN_DIR}/download-1/result.md")"

  # AnyDoc inside the container, not merely linked into it. The Rust suite
  # proves the adapter; only this proves the shipped image can run it.
  log "AnyDoc conversion against the restarted container"
  submit_docx "smoke-anydoc-0001" "$(newuuid)" \
    "${RUN_DIR}/d-submit.json" "${RUN_DIR}/d-submit.h"
  assert_eq "docx submit status" "$(status_of "${RUN_DIR}/d-submit.h")" "202"
  JOB_D="$(jq -r '.data.id' < "${RUN_DIR}/d-submit.json")"
  printf -- '- anydoc job: `%s`\n' "$JOB_D" >> "$EVIDENCE"
  poll_until "$JOB_D" succeeded 180
  api GET "/api/v1/conversions/${JOB_D}/artifacts" \
    "${RUN_DIR}/d-artifacts.json" "${RUN_DIR}/d-artifacts.h"
  assert_json "anydoc artifact count" '.data|length' "${RUN_DIR}/d-artifacts.json" "2"
  api GET "/api/v1/conversions/${JOB_D}/artifacts/manifest" \
    "${RUN_DIR}/d-manifest.json" "${RUN_DIR}/d-manifest.h"
  DM="${RUN_DIR}/d-manifest.json"
  assert_json "anydoc route" '.route.kind' "$DM" "local_anydoc"
  assert_json "anydoc engine" '.engine.name' "$DM" "anydoc"
  assert_json "anydoc engine version" '.engine.version' "$DM" "0.1.9"
  assert_json "anydoc document format" '.document.format' "$DM" "docx"
  assert_json "anydoc source sha256" '.source.sha256' "$DM" "$DOCX_SHA"
  assert_json "anydoc source byteLength" '.source.byteLength' "$DM" "$DOCX_BYTES"
  if grep -q 'text.docx' "$DM"; then fail "the anydoc manifest leaked the uploaded filename"; fi
  api GET "/api/v1/conversions/${JOB_D}/artifacts/markdown" \
    "${RUN_DIR}/d-result.md" "${RUN_DIR}/d-result.h"
  assert_eq "anydoc markdown status" "$(status_of "${RUN_DIR}/d-result.h")" "200"
  [ -s "${RUN_DIR}/d-result.md" ] || fail "anydoc published empty markdown"
  printf -- '- PASS AnyDoc converted a docx inside the container (route local_anydoc)\n' >> "$EVIDENCE"

  submit "smoke-graceful-0001" "$(newuuid)" "${RUN_DIR}/a-conflict.json" "${RUN_DIR}/a-conflict.h"
  assert_eq "conflict after restart" "$(status_of "${RUN_DIR}/a-conflict.h")" "409"

  log "Second restart, then byte comparison"
  compose "${SMOKE[@]}" stop -t 45 converter
  assert_eq "second graceful state" "$(container_state "${SMOKE[@]}")" "0 false false"
  compose "${SMOKE[@]}" up -d --wait --wait-timeout 300
  wait_ready 120
  verify_bundle "$JOB_A" "${RUN_DIR}/download-2"
  cmp "${RUN_DIR}/download-1/result.md" "${RUN_DIR}/download-2/result.md" \
    || fail "markdown changed across a restart"
  cmp "${RUN_DIR}/download-1/manifest.json" "${RUN_DIR}/download-2/manifest.json" \
    || fail "manifest changed across a restart"
  printf -- '- PASS artifacts are byte-identical across a graceful restart\n' >> "$EVIDENCE"
fi

# ------------------------------------------------------------------- smoke B

if [ "$PHASE" = "all" ] || [ "$PHASE" = "sigkill" ]; then
  log "Smoke B: SIGKILL against a held converting_local"
  compose "${SMOKE[@]}" "${HOLD[@]}" up -d --wait --wait-timeout 300
  wait_ready 120

  submit "smoke-sigkill-0001" "$(newuuid)" "${RUN_DIR}/b-submit.json" "${RUN_DIR}/b-submit.h"
  assert_eq "sigkill submit status" "$(status_of "${RUN_DIR}/b-submit.h")" "202"
  JOB_B="$(jq -r '.data.id' < "${RUN_DIR}/b-submit.json")"
  ATTEMPT_B1="$(jq -r '.data.activeAttemptId' < "${RUN_DIR}/b-submit.json")"
  printf -- '- sigkill job: `%s` attempt `%s`\n' "$JOB_B" "$ATTEMPT_B1" >> "$EVIDENCE"

  # Observed, never assumed. The claim and the converting_local transition
  # commit in one transaction before the worker is even spawned.
  poll_until "$JOB_B" converting_local 90
  assert_json "held state" '.data.status' "${RUN_DIR}/poll.json" "converting_local"
  sleep 1

  compose "${SMOKE[@]}" "${HOLD[@]}" kill -s SIGKILL converter
  assert_eq "sigkill container state" "$(container_state "${SMOKE[@]}" "${HOLD[@]}")" "137 false false"

  # synchronous=FULL fsynced the converting_local commit into the WAL seconds
  # ago and nothing had a chance to checkpoint, so the recovered state lives
  # only there.
  WAL_BYTES="$(compose "${SMOKE[@]}" run --rm --no-deps --entrypoint /bin/sh converter \
    -c 'wc -c < /data/converter.sqlite-wal' 2>/dev/null | tr -d '[:space:]')"
  if [ -z "$WAL_BYTES" ] || [ "$WAL_BYTES" -le 0 ]; then
    fail "expected uncheckpointed WAL work, got '${WAL_BYTES}' bytes"
  fi
  printf -- '- PASS uncheckpointed WAL at kill time: `%s` bytes\n' "$WAL_BYTES" >> "$EVIDENCE"

  log "Restarting without the hold layer"
  compose "${SMOKE[@]}" up -d --wait --wait-timeout 300 --force-recreate
  wait_ready 120
  poll_until "$JOB_B" succeeded 180

  ATTEMPT_B2="$(jq -r '.data.activeAttemptId' < "${RUN_DIR}/poll.json")"
  [ "$ATTEMPT_B2" != "$ATTEMPT_B1" ] || fail "recovery reused the interrupted attempt ${ATTEMPT_B1}"
  printf -- '- PASS recovery minted a fresh attempt: `%s` -> `%s`\n' "$ATTEMPT_B1" "$ATTEMPT_B2" >> "$EVIDENCE"
  assert_json "recovered route" '.data.route.kind' "${RUN_DIR}/poll.json" "local_pdf"

  verify_bundle "$JOB_B" "${RUN_DIR}/download-3"
  assert_json "recovered manifest attemptId" '.attemptId' "${RUN_DIR}/download-3/manifest.json" "$ATTEMPT_B2"

  if [ -n "$MD_SHA" ]; then
    assert_eq "markdown is identical across an unclean kill" \
      "$(sha256 "${RUN_DIR}/download-3/result.md")" "$MD_SHA"
    # Startup recovery revalidates every stored success and demotes a corrupt
    # one, so this proves the kill damaged no finished conversion.
    verify_bundle "${JOB_A}" "${RUN_DIR}/download-4"
    cmp "${RUN_DIR}/download-1/result.md" "${RUN_DIR}/download-4/result.md" \
      || fail "the SIGKILL changed an already-completed job's markdown"
    printf -- '- PASS the earlier completed job survived the SIGKILL intact\n' >> "$EVIDENCE"
  fi

  submit "smoke-sigkill-0001" "$(newuuid)" "${RUN_DIR}/b-conflict.json" "${RUN_DIR}/b-conflict.h"
  assert_eq "conflict after forced kill" "$(status_of "${RUN_DIR}/b-conflict.h")" "409"
fi

printf '\nRESULT: PASS\n'
printf -- '\n**RESULT: PASS**\n' >> "$EVIDENCE"
