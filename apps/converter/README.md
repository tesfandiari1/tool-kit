# Tool-Kit conversion backend

This standalone Rust/Axum service is the production foundation for the Tool-Kit
desktop app's conversion backend. It accepts authenticated uploads, converts
native-text PDFs with `pdf-inspector`, and converts supported non-PDF formats
with AnyDoc. It exposes job status and Markdown.

**Conversions are durable.** Accepted uploads, job records, idempotency
records, and published artifacts live in an embedded SQLite database and an
artifact tree under the data root, and they survive a service restart. A queued
or interrupted job is reconciled and resumed on the next start. A succeeded
attempt publishes `result.md` alone. The attempt records the Markdown's
`markdown_byte_length` and `markdown_sha256` at finalizing, and startup
recovery checks a publication against them.

The service is **loopback-only**. The desktop app spawns it as a sidecar on
`127.0.0.1:0` and is its only client.

The service process is CPU-only. It links no OCR, model-serving, PDFium, ONNX,
or accelerator runtime. Local OCR and transcription run in the macOS Vision and
audio workers it spawns. Where the Vision worker runs, it reads images, and a
PDF with no text on any page converts through it on route `local_vision`. A
mixed, `ocr_required` or `garbled_text` PDF takes the same route: the PDF
worker stages each native page's Markdown in `native-pages.json`, Vision reads
only the other pages, and the two are spliced in page order. OCR over a native
page loses figures the text layer holds exactly. A scan with a page Vision
reads no text on publishes with the `pages_without_extractable_text` warning.
A scan gets the PDF timeout plus 1 second per page, capped at 10 minutes or
the PDF timeout, whichever is longer. Every other incomplete or over-ceiling
PDF finishes as `failed`, with the engine's fallback reason as the failure code
and a plain message, and publishes no partial Markdown.

Where the Vision worker runs on macOS 27, a DOCX or PPTX picture with no alt
text gains an `*Image: …*` line from Apple's on-device Foundation Models at the
picture's position. Decorative pictures add nothing, and any failure leaves
today's output.

## Implemented API

| Method | Path | Authentication | Purpose |
|---|---|---|---|
| `GET` | `/health/live` | Public | Process liveness, no dependencies |
| `GET` | `/health/ready` | Public | Probe the database, data root, and worker |
| `GET` | `/api/v1/capabilities` | Public | `data.conversion.acceptingJobs` and `inputFormats` |
| `POST` | `/api/v1/conversions` | Bearer | Stream and submit one supported document |
| `GET` | `/api/v1/conversions/{id}` | Bearer | Poll one durable conversion |
| `GET` | `/api/v1/conversions/{id}/artifacts/markdown` | Bearer | Stream Markdown |

`tests/http_contract.rs` is the contract. Collection lookup, cancellation,
retries and purge do not exist.

A submission may carry a `profile` field. The service accepts and ignores it,
and the replay fingerprint hashes the constant `local_only`, so a request
stored before the profile was retired still replays.

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

## Data root and local reset

The data root (`TOOLKIT_CONVERTER_DATA_DIR`, `<app_data>/converter` under the
app) is the only persistent location. It holds `converter.sqlite`, the
immutable source of every unfinished or failed job, attempt staging, published
Markdown, the pre-acceptance quarantine, and the `.health` probe directory.

**The data root must be host-local.** Never point it at an SMB or NFS mount. SQLite
WAL locking is same-host only, and a network filesystem corrupts the database.

**The service never deletes the data root or an accepted conversion.** It has no
retention, purge, or cleanup job, and it quarantines unowned job trees rather
than removing them. It removes only its own staging and one copy no path reads
again: a rejected submission's tree, superseded attempt staging, and the source
of a succeeded job, once its Markdown publishes. Startup also removes the
sources older builds kept for succeeded jobs. Resetting local development is a
deliberate, manual act: delete the data root. There is no undo and no backup.

## Run natively

Rust 1.97 or newer is required. Build both sibling binaries before starting the
API so startup can verify the worker identity:

```bash
cargo build --locked --manifest-path apps/converter/Cargo.toml --bins
TOOLKIT_CONVERTER_TOKEN_FILE=/absolute/path/to/bootstrap-token \
TOOLKIT_CONVERTER_DATA_DIR="$PWD/apps/converter/target/dev-data" \
  cargo run --locked --manifest-path apps/converter/Cargo.toml --bin tool-kit-converter
```

Set `TOOLKIT_CONVERTER_DATA_DIR` on every start. The default is `/data`, so a
start without it fails at boot with the path in the error.

Submit and poll a clean PDF from another terminal:

```bash
TOKEN="$(tr -d '\r\n' < /absolute/path/to/bootstrap-token)"

curl -i \
  -H "Authorization: Bearer ${TOKEN}" \
  -H 'Idempotency-Key: desktop-run-0001-file-0001' \
  -F 'clientRunId=11111111-1111-4111-8111-111111111111' \
  -F 'source=@/absolute/path/to/document.pdf;type=application/pdf' \
  http://127.0.0.1:8080/api/v1/conversions

curl -H "Authorization: Bearer ${TOKEN}" \
  http://127.0.0.1:8080/api/v1/conversions/JOB_ID
```

When the job reaches `succeeded`, download `.../artifacts/markdown`. Reusing the
same `Idempotency-Key` with identical bytes and metadata returns the same job,
and that record is stored, so the replay still works after a restart. Reusing
the key for different input returns `409`.

The application does not load dotenv files. Configuration is fail-fast: invalid
values or unavailable startup dependencies prevent the listener from binding.

| Variable | Default | Purpose |
|---|---:|---|
| `TOOLKIT_CONVERTER_BIND_ADDR` | `127.0.0.1:8080` | Listener socket |
| `TOOLKIT_CONVERTER_TOKEN_FILE` | `/run/secrets/bootstrap_token` | Bootstrap-token file. The app writes it 0600 at each launch |
| `TOOLKIT_CONVERTER_DATA_DIR` | `/data` | Persistent data root: database and every artifact |
| `TOOLKIT_CONVERTER_PDF_WORKER_PATH` | sibling binary | Absolute worker override |
| `TOOLKIT_CONVERTER_AUDIO_DIARIZER_DIR` | unset | Parent of the staged `speaker-diarization/` CoreML set. Required whenever the audio worker is present |
| `TOOLKIT_CONVERTER_AUDIO_TIMEOUT_SECS` | `1800` | Audio worker wall deadline |
| `TOOLKIT_CONVERTER_MAX_AUDIO_UPLOAD_BYTES` | `1073741824` | Per-recording streaming ceiling. Documents stop at 25 MiB |
| `TOOLKIT_CONVERTER_PDF_BCMAPS_DIR` | crate fallback | Runtime CMap directory. The app sets it to the bundled bcmaps |
| `TOOLKIT_CONVERTER_MAX_JOBS` | `32` | Active-job ceiling, counted from durable state |
| `TOOLKIT_CONVERTER_MAX_CONCURRENT_UPLOADS` | `2` | Concurrent staging permits before immediate `429` |
| `TOOLKIT_CONVERTER_UPLOAD_TIMEOUT_SECS` | `120` | Whole-upload deadline |
| `TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS` | `60` | Per-worker wall deadline |
| `TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS` | `30` | Shared HTTP and worker shutdown deadline |
| `TOOLKIT_CONVERTER_SHUTDOWN_ON_STDIN_EOF` | `0` | Shut down when stdin closes; `0` or `1` |
| `RUST_LOG` | `tool_kit_converter=info` | Structured tracing filter |

The Vision and audio workers are found only by the sibling probe. An absent
worker means the engine is simply absent, which is the normal case off macOS. A present audio worker with no diarizer directory,
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
desktop app turns it on.

## Traps

- **`sqlx::migrate!` reads `migrations/` at compile time**, so the SQL and
  `build.rs` are build inputs.
- **A PDF-shaped assumption in an engine-neutral layer crash-looped the
  service.** When you add an engine, grep the shared layers for the other
  engine's vocabulary.
- **`src/faults.rs` is production code that production never arms.** Only a
  test holding an `AppState` reaches it. Do not add a flag, a variable, or a
  route that arms it.
- **A format is advertised only with a passing round-trip fixture.**
  `advertised_media_types_match_the_migration_check` pins the admission table
  to the migration CHECK. Drift turns a clean 415 into an INSERT constraint
  error at runtime.
- **A no-transaction migration must wrap its rebuild in one transaction.**
  `every_no_transaction_migration_wraps_its_rebuild_in_one_transaction` pins
  every one of them. After the first deployment, a broken migration needs a new
  file.
- **`/health/ready` is unauthenticated** and opens a `BEGIN IMMEDIATE`
  transaction per request against a four-connection pool. Loopback only today.

## Verify

```bash
cargo fmt --manifest-path apps/converter/Cargo.toml --all -- --check
cargo check --locked --offline --manifest-path apps/converter/Cargo.toml --all-targets
cargo test --locked --offline --manifest-path apps/converter/Cargo.toml
cargo clippy --locked --offline --manifest-path apps/converter/Cargo.toml --all-targets -- -D warnings
# Extra deny-set lives in Cargo.toml [lints]; keep in sync with apps/desktop/src-tauri.
```

`cargo clippy` on macOS never compiles the `#[cfg(target_os = "linux")]` blocks
in `src/bin/tool-kit-pdf-worker.rs`, so a local pass is necessary and not
sufficient. CI is the only gate that lints them.

The exact `pdf-inspector` pin, bundled CMap and license requirements, the
dependency exception register and the old container-smoke evidence are in
[`../../docs/archive/STATUS_2026-09-12.md`](../../docs/archive/STATUS_2026-09-12.md).
This crate stays independent from `apps/desktop/src-tauri` and keeps its own
lockfile. Do not create a root Cargo workspace.
