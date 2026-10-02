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
Stronger parser isolation, per-device credentials, TLS, and backups remain in
Phase 3 of [`../../docs/STATUS.md`](../../docs/STATUS.md). M4 is complete. Datalab
routing inside the backend is Phase 2 and is not active yet.

The service process is CPU-only. It links no OCR, model-serving, PDFium, ONNX,
or accelerator runtime. Local OCR and transcription run in the macOS Vision and
audio workers it spawns. Where the Vision worker runs, it reads images, and a
PDF with no text on any page converts through it on route `local_vision`. A
scan with a page Vision reads no text on publishes with the
`pages_without_extractable_text` warning. A scan gets the PDF timeout plus 1
second per page, capped at 10 minutes or the PDF timeout, whichever is longer.
Other scanned, image-based, mixed, garbled, incomplete, or over-ceiling PDFs
finish as `needs_remote` without publishing partial Markdown. Datalab routing
is not active yet.

Where the Vision worker runs on macOS 27, a DOCX or PPTX picture with no alt
text gains an `*Image: …*` line from Apple's on-device Foundation Models at the
picture's position. Decorative pictures add nothing, and any failure leaves
today's output.

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

The complete live contract is
[`contract/http/openapi.yaml`](../contract/http/openapi.yaml).
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
mkdir -p deploy/docker/secrets
umask 077
openssl rand -hex 32 > deploy/docker/secrets/bootstrap-token.txt
docker compose -f deploy/docker/compose.yaml up --build
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
TOKEN="$(tr -d '\r\n' < deploy/docker/secrets/bootstrap-token.txt)"

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
immutable source of every unfinished, failed, or `needs_remote` job, attempt
staging, published Markdown and manifests, the pre-acceptance quarantine, and
the `.health` probe directory.

`/tmp` is scratch, not storage. Compose mounts it as a `tmpfs` because the PDF
worker calls `tempfile::tempfile()`. Nothing there is expected to survive.

**`/data` must be host-local.** Never point it at an SMB or NFS mount. SQLite
WAL locking is same-host only, and a network filesystem corrupts the database.

**The service never deletes the data root or an accepted conversion.** It has no
retention, purge, or cleanup job, and it quarantines unowned job trees rather
than removing them. It removes only its own staging and one copy no path reads
again: a rejected submission's tree, superseded attempt staging, and the source
of a succeeded job, once its artifacts publish. Startup also removes the
sources older builds kept for succeeded jobs. Resetting local development is
a deliberate, manual act:

```bash
docker compose -f deploy/docker/compose.yaml down -v
```

`-v` destroys the `converter-data` volume and every real conversion in it. There
is no undo and no backup. Drop the `-v` to stop the service and keep the data.

## Run natively

Rust 1.97 or newer is required. Build both sibling binaries before starting the
API so startup can verify the worker identity:

```bash
cargo build --locked --manifest-path apps/converter/Cargo.toml --bins
TOOLKIT_CONVERTER_TOKEN_FILE=/absolute/path/to/bootstrap-token \
TOOLKIT_CONVERTER_DATA_DIR="$PWD/apps/converter/target/dev-data" \
  cargo run --locked --manifest-path apps/converter/Cargo.toml --bin tool-kit-converter
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
| `TOOLKIT_CONVERTER_VISION_WORKER_PATH` | sibling binary where it exists | Absolute macOS Vision worker override |
| `TOOLKIT_CONVERTER_AUDIO_WORKER_PATH` | sibling binary where it exists | Absolute macOS audio worker override |
| `TOOLKIT_CONVERTER_AUDIO_DIARIZER_DIR` | unset | Parent of the staged `speaker-diarization/` CoreML set. Required whenever the audio worker is present |
| `TOOLKIT_CONVERTER_AUDIO_TIMEOUT_SECS` | `1800` | Audio worker wall deadline |
| `TOOLKIT_CONVERTER_MAX_AUDIO_UPLOAD_BYTES` | `1073741824` | Per-recording streaming ceiling. Documents keep `MAX_UPLOAD_BYTES` |
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
| `TOOLKIT_CONVERTER_SHUTDOWN_ON_STDIN_EOF` | `0` | Shut down when stdin closes; `0` or `1` |
| `RUST_LOG` | `tool_kit_converter=info` | Structured tracing filter |

The Swift worker overrides differ from the PDF one in one way. A configured
path is a promise, so the engine reports what it finds there whether or not the
file exists. Only the implicit sibling probe is allowed to come back empty, and
an absent Vision or audio worker means the engine is simply absent, which is
the normal case off macOS. A present audio worker with no diarizer directory,
or one missing any model file FluidAudio loads, refuses to start: that is a
packaging bug, not a host without the engine. The audio worker's last stderr
line reaches `converter.log` when it fails.

`TOOLKIT_CONVERTER_BIND_ADDR` accepts port `0`. The kernel then picks a free
port and the `conversion service listening` log line carries the bound address,
so that line is the only place the real port appears.

`TOOLKIT_CONVERTER_SHUTDOWN_ON_STDIN_EOF=1` is for a parent process that spawns
the service with a pipe on stdin. The write end closes when that parent dies,
including a kill it could not handle, so EOF is a parent-death signal no
handler can deliver. Leave it off wherever stdin is a terminal, a closed
descriptor, or `/dev/null`, because EOF there says nothing about a parent. The
container leaves it off and shuts down on SIGTERM alone.

`TOOLKIT_CONVERTER_SCRATCH_PARENT` does not work. The config loader parses and
validates it, and no other code reads it. It stays until a separate decision on
the environment surface, so setting it changes nothing.

## Traps

- **The Dockerfile must keep copying `migrations/` and `build.rs`.**
  `sqlx::migrate!` reads the SQL at compile time, so both are build inputs.
- **Keep `RUN install -d -o 10001 -g 10001 -m 0700 /data`.** Without it a fresh
  named volume lands as `root:root` while the service runs as `10001`.
- **No `VOLUME` instruction.** It hands a plain `docker run` an anonymous volume
  nobody prunes.
- **`TOOLKIT_CONVERTER_TOKEN_FILE` means two different things in
  `compose.yaml`.** Under `environment:` it is the container path. Under
  `secrets:` it is a host-side substitution. Leave it unset on the host.
- **`stop_grace_period` is 45s on purpose.** Docker's default 10s kills the
  service 20s before its own 30s drain can finish, so every stop reads as a
  crash.
- **Run the container smoke through a CPU-capped builder.**
  `TOOLKIT_SMOKE_BUILDER` names one. The default builder lives inside the Docker
  VM, where `docker update` cannot reach it.
- **A PDF-shaped assumption in an engine-neutral layer crash-looped the
  service.** When you add an engine, grep the shared layers for the other
  engine's vocabulary.
- **`src/faults.rs` is production code that production never arms.** Only a
  test holding an `AppState` reaches it. Do not add a flag, a variable, or a
  route that arms it.
- **A format is advertised only with a passing round-trip fixture.** Two tests
  pin the admission table to the migration CHECK and the upload contract. Drift
  turns a clean 415 into an INSERT constraint error at runtime.
- **A no-transaction migration must wrap its rebuild in one transaction.**
  `every_no_transaction_migration_wraps_its_rebuild_in_one_transaction` pins
  every one of them. After the first deployment, a broken migration needs a new
  file.
- **`execute_claimed` bounds its engine-permit wait and honors shutdown.** A
  detached AnyDoc parse still holds its permit, so the wait is real.
- **`TOOLKIT_CONVERTER_SCRATCH_PARENT` is dead config.** `config.rs` parses and
  validates it, and nothing reads it. Do not wire it to anything.
- **`/health/ready` is unauthenticated** and opens a `BEGIN IMMEDIATE`
  transaction per request against a four-connection pool. Loopback only today.

## Verify

```bash
cargo fmt --manifest-path apps/converter/Cargo.toml --all -- --check
cargo check --locked --offline --manifest-path apps/converter/Cargo.toml --all-targets
cargo test --locked --offline --manifest-path apps/converter/Cargo.toml
cargo clippy --locked --offline --manifest-path apps/converter/Cargo.toml --all-targets -- -D warnings
# Extra deny-set lives in Cargo.toml [lints]; keep in sync with apps/desktop/src-tauri.
docker compose -f deploy/docker/compose.yaml config --quiet
docker build -f deploy/docker/Dockerfile -t tool-kit-converter:m2 .
```

The image healthcheck uses `--start-interval`, which needs Docker 25 or newer.
On an older engine the container still runs. It just takes longer to report
healthy.

`cargo clippy` on macOS never compiles the `#[cfg(target_os = "linux")]` blocks
in `src/bin/tool-kit-pdf-worker.rs`, so a local pass is necessary and not
sufficient. CI is the only gate that lints them.

The exact `pdf-inspector` pin, bundled CMap and license requirements, the
dependency exception register and the container-smoke evidence are in
[`../../docs/archive/STATUS_2026-09-12.md`](../../docs/archive/STATUS_2026-09-12.md).
This crate stays independent from `apps/desktop/src-tauri` and keeps its own
lockfile. Do not create a root Cargo workspace.

The approved target remains one CPU-only converter container with embedded
SQLite, one bounded in-process worker, and a host-local `/data` dataset. Caddy
will be the only LAN-facing service at the deployment milestone; PostgreSQL,
Redis, and a separate queue service are not part of the initial design.
