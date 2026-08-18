# Tool-Kit conversion backend

This standalone Rust/Axum service is the production foundation for the Tool-Kit
desktop app's conversion backend. It accepts authenticated uploads, converts
native-text PDFs with `pdf-inspector`, and converts supported non-PDF formats
with AnyDoc. It exposes job status, Markdown, and a versioned provenance
manifest.

**Conversions are durable.** Accepted uploads, job records, idempotency
records, and published artifacts live in an embedded SQLite database and an
artifact tree under the data root, and they survive a service restart. A queued
or interrupted job is reconciled and resumed on the next start.

The service is still **loopback-only**. Do not expose it to the LAN yet.
Stronger parser isolation, per-device credentials, TLS, backups, and the TrueNAS
overlay remain release-gated milestones in
[`../docs/BACKEND_EPIC.md`](../docs/BACKEND_EPIC.md). M2 is closed. M3 is in
progress. Parallel sessions start at [`../docs/HANDOFF.md`](../docs/HANDOFF.md).
Datalab routing is not active yet.

The service is permanently CPU-only. It contains no local OCR, model-serving,
PDFium, ONNX, or accelerator runtime. PDFs that are scanned, image-based, mixed,
garbled, incomplete, or over the local output ceiling finish as `needs_remote`
without publishing partial Markdown. Datalab routing is not active yet.

## Implemented API

| Method | Path | Authentication | Purpose |
|---|---|---|---|
| `GET` | `/health/live` | Public | Process liveness, no dependencies |
| `GET` | `/health/ready` | Public | Probe the database, data root, and worker |
| `GET` | `/api/v1/capabilities` | Public | Current formats, profiles, capacity, and limits |
| `POST` | `/api/v1/conversions` | Bearer | Stream and submit one supported document |
| `GET` | `/api/v1/conversions/{id}` | Bearer | Poll one durable conversion |
| `GET` | `/api/v1/conversions/{id}/artifacts` | Bearer | List published artifacts |
| `GET` | `/api/v1/conversions/{id}/artifacts/markdown` | Bearer | Stream Markdown |
| `GET` | `/api/v1/conversions/{id}/artifacts/manifest` | Bearer | Stream manifest JSON |

The complete live contract is [`openapi/openapi.yaml`](openapi/openapi.yaml).
Collection lookup, cancellation, retries, purge, and remote conversion are
absent until their tracked milestones land.

Profiles accepted today:

- `standard` and `local_only` run the same local path. A document that
  cannot complete safely returns `needs_remote`. Nothing is transmitted.
- `best_quality` is a valid future profile but returns `409 profile_unavailable`
  until the Datalab adapter exists.

## Liveness and readiness

The two health routes answer different questions, so an orchestrator can tell a
hung process from a broken dependency.

`GET /health/live` is dependency-free. It proves the process is running and
answering, and nothing else. It always returns `200`.

`GET /health/ready` runs three checks concurrently, each bounded by a two-second
timeout:

- `database` opens a write transaction against SQLite and rolls it back.
- `dataRoot` creates and removes one probe file under `<data root>/.health/`.
  It never reads, writes, or deletes anything under `jobs/`, so readiness cannot
  damage a stored conversion.
- `worker` confirms the single job runner has neither failed nor stopped
  claiming work.

All three pass and the route returns `200` with `status: "ready"`. Any failure,
including a timed-out check, returns `503` with `status: "not_ready"` and the
per-check detail. The route is unauthenticated, so the body carries pass or fail
and nothing more. The underlying error goes to the log at `warn`.

## Run with Docker Compose

Run all commands from the repository root. Create a development token once:

```bash
mkdir -p backend/secrets
umask 077
openssl rand -hex 32 > backend/secrets/bootstrap-token.txt
docker compose -f backend/compose.yaml up --build
```

Compose publishes only `127.0.0.1:8080`, mounts the token as a read-only secret,
and runs the service as UID/GID `10001:10001` with a read-only root filesystem,
all capabilities dropped, `no-new-privileges`, bounded CPU/RAM/PIDs, and bounded
`tmpfs` scratch. The named volume `converter-data` is mounted at `/data` and is
the one writable path that outlives the container.

The image creates `/data` as `10001:10001` with mode `0700`. That line is
load-bearing: a fresh named volume takes the ownership of the image directory it
covers and falls back to `root:root` when the image has no such directory, so
without it the service cannot write its own data root on first start.

Submit and poll a clean PDF from another terminal:

```bash
TOKEN="$(tr -d '\r\n' < backend/secrets/bootstrap-token.txt)"

curl -i \
  -H "Authorization: Bearer ${TOKEN}" \
  -H 'Idempotency-Key: desktop-run-0001-file-0001' \
  -F 'clientRunId=11111111-1111-4111-8111-111111111111' \
  -F 'profile=standard' \
  -F 'source=@/absolute/path/to/document.pdf;type=application/pdf' \
  http://127.0.0.1:8080/api/v1/conversions

curl -H "Authorization: Bearer ${TOKEN}" \
  http://127.0.0.1:8080/api/v1/conversions/JOB_ID
```

When the job reaches `succeeded`, download
`.../artifacts/markdown` and `.../artifacts/manifest`. Reusing the same
`Idempotency-Key` with identical bytes and metadata returns the same job, and
that record is stored, so the replay still works after a restart. Reusing the
key for different input returns `409`.

## Data root and local reset

`/data` is the only persistent location. It holds `converter.sqlite`, the
immutable source of every accepted job, attempt staging, published Markdown and
manifests, the pre-acceptance quarantine, and the `.health` probe directory.

`/tmp` is scratch, not storage. Compose mounts it as a `tmpfs` because the PDF
worker calls `tempfile::tempfile()`. Nothing there is expected to survive.

**`/data` must be host-local.** Never point it at an SMB or NFS mount. SQLite
WAL locking is same-host only, and a network filesystem corrupts the database.

**The service never deletes the data root or an accepted conversion.** It has no
retention, purge, or cleanup job, and it quarantines unowned job trees rather
than removing them. It removes only its own unaccepted staging: a rejected
submission's tree and superseded attempt staging. Resetting local development is
a deliberate, manual act:

```bash
docker compose -f backend/compose.yaml down -v
```

`-v` destroys the `converter-data` volume and every real conversion in it. There
is no undo and no backup. Drop the `-v` to stop the service and keep the data.

## Run natively

Rust 1.97 or newer is required. Build both sibling binaries before starting the
API so startup can verify the worker identity:

```bash
cargo build --locked --manifest-path backend/Cargo.toml --bins
TOOLKIT_CONVERTER_TOKEN_FILE=/absolute/path/to/bootstrap-token \
TOOLKIT_CONVERTER_DATA_DIR="$PWD/backend/target/dev-data" \
  cargo run --locked --manifest-path backend/Cargo.toml --bin tool-kit-converter
```

`TOOLKIT_CONVERTER_DATA_DIR` is required outside the container. The default is
`/data`, which only the image creates, so a native start without it fails at
boot with the path in the error.

The application does not load dotenv files. Configuration is fail-fast: invalid
values or unavailable startup dependencies prevent the listener from binding.

| Variable | Default | Purpose |
|---|---:|---|
| `TOOLKIT_CONVERTER_BIND_ADDR` | `127.0.0.1:8080` | Listener socket |
| `TOOLKIT_CONVERTER_TOKEN_FILE` | `/run/secrets/bootstrap_token` | Mounted bootstrap-token file |
| `TOOLKIT_CONVERTER_DATA_DIR` | `/data` | Persistent data root: database and every artifact |
| `TOOLKIT_CONVERTER_SCRATCH_PARENT` | `/tmp` | Read by the config loader and used nowhere else |
| `TOOLKIT_CONVERTER_PDF_WORKER_PATH` | sibling binary | Absolute worker override |
| `TOOLKIT_CONVERTER_PDF_BCMAPS_DIR` | crate fallback | Runtime CMap directory; container sets this explicitly |
| `TOOLKIT_CONVERTER_MAX_UPLOAD_BYTES` | `26214400` | Per-source streaming ceiling |
| `TOOLKIT_CONVERTER_MAX_OUTPUT_BYTES` | `52428800` | Markdown ceiling enforced before worker write |
| `TOOLKIT_CONVERTER_MAX_JOBS` | `32` | Active-job ceiling, counted from durable state |
| `TOOLKIT_CONVERTER_MAX_CONCURRENT_UPLOADS` | `2` | Concurrent staging permits before immediate `429` |
| `TOOLKIT_CONVERTER_UPLOAD_TIMEOUT_SECS` | `120` | Whole-upload deadline |
| `TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS` | `60` | Per-worker wall deadline |
| `TOOLKIT_CONVERTER_PDF_THREADS` | `2` | Native parser Rayon threads |
| `TOOLKIT_CONVERTER_DATABASE_BUSY_TIMEOUT_SECS` | `5` | SQLite lock wait before a busy error |
| `TOOLKIT_CONVERTER_WORKER_POLL_INTERVAL_SECS` | `1` | Queue poll that backs up the wake-up notification |
| `TOOLKIT_CONVERTER_RECOVERY_LIMIT` | `3` | Fresh attempts a job may receive across restarts |
| `TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS` | `30` | Shared HTTP and worker shutdown deadline |
| `RUST_LOG` | `tool_kit_converter=info` | Structured tracing filter |

`TOOLKIT_CONVERTER_SCRATCH_PARENT` does not work. The config loader parses and
validates it, and no other code reads it. It stays until a separate decision on
the environment surface, so setting it changes nothing.

## Verify

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo check --locked --offline --manifest-path backend/Cargo.toml --all-targets
cargo test --locked --offline --manifest-path backend/Cargo.toml
cargo clippy --locked --offline --manifest-path backend/Cargo.toml --all-targets -- -D warnings
# Extra deny-set lives in Cargo.toml [lints]; keep in sync with src-tauri.
docker compose -f backend/compose.yaml config --quiet
docker build -t tool-kit-converter:m2 backend
```

The image healthcheck uses `--start-interval`, which needs Docker 25 or newer.
On an older engine the container still runs. It just takes longer to report
healthy.

`cargo clippy` on macOS never compiles the `#[cfg(target_os = "linux")]` blocks
in `src/bin/tool-kit-pdf-worker.rs`, so a local pass is necessary and not
sufficient. CI is the only gate that lints them.

The exact `pdf-inspector` pin, bundled CMap/license requirements, dependency
exception register, and container-smoke evidence are tracked in the architecture
and epic documents. This crate remains independent from `src-tauri` and keeps
its own lockfile; do not create a root Cargo workspace without a separate
migration decision.

The approved target remains one CPU-only converter container with embedded
SQLite, one bounded in-process worker, and a host-local `/data` dataset. Caddy
will be the only LAN-facing service at the deployment milestone; PostgreSQL,
Redis, and a separate queue service are not part of the initial design.
