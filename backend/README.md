# Tool-Kit conversion backend

This standalone Rust/Axum service is the production foundation for the Tool-Kit
desktop app's conversion backend. Its completed M1 slice accepts authenticated
PDF uploads, converts complete native-text PDFs locally with `pdf-inspector`,
and exposes job status, Markdown, and a versioned provenance manifest.

M1 is a **loopback-only development vertical slice**. Jobs, idempotency records,
uploads, and artifacts are intentionally ephemeral and disappear on restart.
Do not expose this Compose service to the LAN yet; durable storage, stronger
parser isolation, per-device credentials, TLS, backups, and the TrueNAS overlay
remain release-gated milestones in [`../BACKEND_EPIC.md`](../BACKEND_EPIC.md).
The next bounded unit is the SQLite and persistent-artifact work in
[`../BACKEND_EXECUTION_PLAN.md`](../BACKEND_EXECUTION_PLAN.md). AnyDoc and
Datalab follow only after durability is proven.

The service is permanently CPU-only. It contains no local OCR, model-serving,
PDFium, ONNX, or accelerator runtime. PDFs that are scanned, image-based, mixed,
garbled, incomplete, or over the local output ceiling finish as `needs_remote`
without publishing partial Markdown. Datalab routing is not active in M1.

## Implemented API

| Method | Path | Authentication | Purpose |
|---|---|---|---|
| `GET` | `/health/live` | Public | Process liveness |
| `GET` | `/health/ready` | Public | Validated startup dependencies are ready |
| `GET` | `/api/v1/capabilities` | Public | Current formats, profiles, capacity, and limits |
| `POST` | `/api/v1/conversions` | Bearer | Stream and submit one PDF |
| `GET` | `/api/v1/conversions/{id}` | Bearer | Poll one ephemeral job |
| `GET` | `/api/v1/conversions/{id}/artifacts` | Bearer | List published artifacts |
| `GET` | `/api/v1/conversions/{id}/artifacts/markdown` | Bearer | Stream Markdown |
| `GET` | `/api/v1/conversions/{id}/artifacts/manifest` | Bearer | Stream manifest JSON |

The complete live contract is [`openapi/openapi.yaml`](openapi/openapi.yaml).
Collection lookup, cancellation, retries, purge, non-PDF formats, and remote
conversion are absent until their tracked milestones are implemented.

Profiles accepted by M1:

- `standard` and `local_only` run the same local PDF path. A document that
  cannot complete safely returns `needs_remote`; nothing is transmitted.
- `best_quality` is a valid future profile but returns `409 profile_unavailable`
  until the Datalab adapter exists.

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
`tmpfs` scratch.

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
`Idempotency-Key` with identical bytes and metadata returns the same job during
that service process; reusing it for different input returns `409`.

## Run natively

Rust 1.95 or newer is required. Build both sibling binaries before starting the
API so startup can verify the worker identity:

```bash
cargo build --locked --manifest-path backend/Cargo.toml --bins
TOOLKIT_CONVERTER_TOKEN_FILE=/absolute/path/to/bootstrap-token \
  cargo run --locked --manifest-path backend/Cargo.toml --bin tool-kit-converter
```

The application does not load dotenv files. Configuration is fail-fast: invalid
values or unavailable startup dependencies prevent the listener from binding.

| Variable | Default | Purpose |
|---|---:|---|
| `TOOLKIT_CONVERTER_BIND_ADDR` | `127.0.0.1:8080` | Listener socket |
| `TOOLKIT_CONVERTER_TOKEN_FILE` | `/run/secrets/bootstrap_token` | Mounted bootstrap-token file |
| `TOOLKIT_CONVERTER_SCRATCH_PARENT` | `/tmp` | Ephemeral session parent |
| `TOOLKIT_CONVERTER_PDF_WORKER_PATH` | sibling binary | Absolute worker override |
| `TOOLKIT_CONVERTER_PDF_BCMAPS_DIR` | crate fallback | Runtime CMap directory; container sets this explicitly |
| `TOOLKIT_CONVERTER_MAX_UPLOAD_BYTES` | `26214400` | Per-source streaming ceiling |
| `TOOLKIT_CONVERTER_MAX_OUTPUT_BYTES` | `52428800` | Markdown ceiling enforced before worker write |
| `TOOLKIT_CONVERTER_MAX_JOBS` | `32` | Process-local job-record ceiling |
| `TOOLKIT_CONVERTER_MAX_CONCURRENT_UPLOADS` | `2` | Concurrent staging permits before immediate `429` |
| `TOOLKIT_CONVERTER_UPLOAD_TIMEOUT_SECS` | `120` | Whole-upload deadline |
| `TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS` | `60` | Per-worker wall deadline |
| `TOOLKIT_CONVERTER_PDF_THREADS` | `2` | Native parser Rayon threads |
| `RUST_LOG` | `tool_kit_converter=info` | Structured tracing filter |

## Verify

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo check --locked --offline --manifest-path backend/Cargo.toml
cargo test --locked --offline --manifest-path backend/Cargo.toml
cargo clippy --locked --offline --manifest-path backend/Cargo.toml --all-targets -- -D warnings
docker compose -f backend/compose.yaml config --quiet
docker build -t tool-kit-converter:m1 backend
```

The exact `pdf-inspector` pin, bundled CMap/license requirements, dependency
exception register, and container-smoke evidence are tracked in the architecture
and epic documents. This crate remains independent from `src-tauri` and keeps
its own lockfile; do not create a root Cargo workspace without a separate
migration decision.

The approved target remains one CPU-only converter container with embedded
SQLite, one bounded in-process worker, and a host-local `/data` dataset. Caddy
will be the only LAN-facing service at the deployment milestone; PostgreSQL,
Redis, and a separate queue service are not part of the initial design.
