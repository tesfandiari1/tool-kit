# Job progress and operator monitoring

**Status:** recommendation — not a ticket, not scheduled work  
**Last updated:** 2026-08-18  
**Implements nothing.** M6 still owns the poll loop; M7 still owns metrics.

Two problems that look similar and must stay separate:

| Track | Audience | Channel | Milestone |
|---|---|---|---|
| Job UX | the person watching a file row | `GET /api/v1/conversions/{id}` from Tauri, then `job-updated` | M6 Increment 4 ([`DESKTOP_EXECUTION_PLAN.md`](DESKTOP_EXECUTION_PLAN.md)) |
| Operator monitoring | you, on the host | JSON stdout, `/health/*`, later an internal scrape | M7 CVR-074 ([`BACKEND_EPIC.md`](BACKEND_EPIC.md)) |

A Prometheus scrape is not how the Run view learns a PDF is converting. A
`job-updated` event is not how you learn `/data` is full.

The architecture already chose polling as the progress mechanism and listed
SSE, WebSockets, and a workflow dashboard as non-goals
([`BACKEND_SERVICE_PLAN.md`](BACKEND_SERVICE_PLAN.md) “Public API”,
[`BACKEND_EPIC.md`](BACKEND_EPIC.md) “Explicit non-goals”). This note records
how to use that choice, and which files already implement the pieces.

## Track A — job UX

### What already exists

The instrument face is closer to done than the converter contract is.

| Piece | File | What it does |
|---|---|---|
| Desktop job row | [`src-tauri/src/jobs.rs`](../src-tauri/src/jobs.rs) | `Job.status` (`queued` / `working` / `processing` / `done` / `failed`), `progress_note`, `started_at`. `set_status` writes both and `emit`s. The Datalab/Rev.ai poll loop sleeps 5s (`Duration::from_secs(5)`, ~720 attempts). |
| Event | [`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs), [`src/app/commands.ts`](../src/app/commands.ts) | Rust `app.emit("job-updated", job)`. The webview listens with `commands.onJobUpdated`. |
| Wire type | [`src/app/types.ts`](../src/app/types.ts) | `progressNote: string`. Views do not import Tauri. |
| Row UI | [`src/domains/run/RunView.tsx`](../src/domains/run/RunView.tsx) | `Status` + `progressNote` while active; elapsed timer from `startedAt`; run-level `<Meter value={finished / total}>`. |
| Meter contract | [`src/ui/primitives/Meter.tsx`](../src/ui/primitives/Meter.tsx) | `value` is 0–1. **Omit it when the figure is unknown** — an invented percentage is a promise the job cannot keep, and the fill becomes the indeterminate sweep. |
| Converter job | [`backend/src/conversion/model.rs`](../backend/src/conversion/model.rs), [`backend/openapi/openapi.yaml`](../backend/openapi/openapi.yaml) `ConversionJob` | Durable `status` enum, `route`, `warnings`, `failure`, `createdAt`, `updatedAt`. **No progress field.** |
| Poll target | [`backend/src/api/conversions.rs`](../backend/src/api/conversions.rs) `get` | `GET /api/v1/conversions/{id}` returns that `JobView`. |
| Worker | [`backend/src/worker_protocol.rs`](../backend/src/worker_protocol.rs), [`backend/src/engines/pdf_inspector.rs`](../backend/src/engines/pdf_inspector.rs) | Child process, terminal `WorkerReport` (`Converted` / `NeedsRemote` / `Rejected`). No page stream. |
| HTTP door | [`src/app/api/transport.ts`](../src/app/api/transport.ts) | Webview never `fetch`es the box. Host attaches the token. No Rust `service_request` handler until M6. |

Yaak’s stream rule, applied here: register the listener before the command
runs so the first event is not missed
([`docs/YAAK_ARCHITECTURE_REFERENCE.md`](YAAK_ARCHITECTURE_REFERENCE.md)
pattern 2, `rpcStream`). `onJobUpdated` already does that. Do not open
`EventSource` from React to the converter — that puts the bearer token in the
webview, which is the mistake the host split exists to prevent.

### Do this in M6 Increment 4

Host polls `GET /api/v1/conversions/{id}` on the existing 5s loop. Map the
durable status onto the `Job` the UI already renders. Do not add a
free-text `progressNote` on the converter, and do not add a new transport.

| Backend `status` | Desktop `status` | `progressNote` |
|---|---|---|
| `queued` | `queued` | `Queued` |
| `converting_local` | `processing` | `Converting locally` |
| `finalizing` | `processing` | `Finalizing` |
| `succeeded` | `done` | (clear) |
| `failed` | `failed` | `failure.message` |
| `needs_remote` | fall through to the Datalab poll | `Sending to Datalab…` |

The run Meter stays files-done / files-total, which is a real number. A
per-file percent is not. Native-text PDFs often finish inside one 5s poll;
the row may jump `queued` → `done`. That is fine.

If 5s feels sticky after a real run, poll at 1s only while
`converting_local` or `finalizing`. Still GET. Four in-flight jobs at 5s is
~12 requests per minute; that is why
[`YAAK_ARCHITECTURE_REFERENCE.md`](YAAK_ARCHITECTURE_REFERENCE.md) says not
to copy `yaak-sse` / `yaak-ws` until polling actually hurts.

### Granularity ladder

Climb a rung only when an engine can fill a count.

| Rung | Signal | Cost | Do it? |
|---|---|---|---|
| 0 — phase | The `status` enum already on `ConversionJob` | Host mapper. Zero OpenAPI change | Yes. M6 Increment 4. |
| 1 — faster poll | Same GET, 1s while converting/finalizing | Trivial | Only if 5s feels dead. Measure first. |
| 2 — optional ratio | `progress.completed` / `progress.total`, omitted when unknown | Versioned OpenAPI add | Only after an engine can fill it. |
| 3 — page loop | `detect_pdf` then `extract_pages_markdown`, persist page N of M | Worker protocol change, restart semantics, partial Markdown | No. Not to feed a bar. |

Datalab’s poll is `processing` / `complete` / `failed` with no percent
([API](https://documentation.datalab.to/docs/welcome/api)). Rev.ai is
`in_progress` / `transcribed` / `failed`
([get job](https://docs.rev.ai/api/asynchronous/reference/jobs/getjobbyid)).
pdf-inspector’s `process_pdf` is one shot; `extract_pages_markdown` is a
caller-driven page loop, not a callback
([Rust API](https://github.com/firecrawl/pdf-inspector/blob/main/docs/rust-api.md)).
Using that loop would change isolation and timeouts. It is not a UI feature.

Do not:

- invent a percent from elapsed time (`Meter` forbids it)
- watch staging-file size (the worker writes a terminal report)
- add SSE, WebSockets, or `GET ?wait=` long-poll for M6
- put Prometheus in the Run view

## Track B — operator monitoring

### What already exists

| Piece | File | What it does |
|---|---|---|
| JSON logs | [`backend/src/main.rs`](../backend/src/main.rs) `init_tracing` | `tracing_subscriber` JSON to stdout, filter from `RUST_LOG`. |
| Request correlation | [`backend/src/api/mod.rs`](../backend/src/api/mod.rs) `trace_request` | `x-request-id` header, `http.request` span, `elapsed_ms` on completion. |
| Job events | [`backend/src/conversion/service.rs`](../backend/src/conversion/service.rs), [`backend/src/jobs/mod.rs`](../backend/src/jobs/mod.rs), [`backend/src/jobs/recovery.rs`](../backend/src/jobs/recovery.rs) | `job_id` / `attempt_id` / status on conversion and recovery events. Keep them content-free. |
| Liveness | [`backend/src/api/health.rs`](../backend/src/api/health.rs) `live` | No dependencies. Process is up. |
| Readiness | same file, `ready` | SQLite write, data-root probe, worker not failed. Compose healthcheck hits this ([`backend/compose.yaml`](../backend/compose.yaml)). |
| Log volume | [`backend/compose.yaml`](../backend/compose.yaml) `logging` | `json-file`, 10 MB × 3. |
| Filter | [`backend/.env.example`](../backend/.env.example), [`backend/README.md`](../backend/README.md) | `RUST_LOG=tool_kit_converter=info`. |

Day-one operations is `docker compose logs` and the healthcheck. Do not add
OpenTelemetry, Grafana, or a metrics container to this Compose file. The
stack is one converter, later Caddy
([`BACKEND_SERVICE_PLAN.md`](BACKEND_SERVICE_PLAN.md) “Deployment shape”).

Do not scrape `/health/ready` as a metric. It opens a `BEGIN IMMEDIATE` on a
four-connection pool; [`HANDOFF.md`](HANDOFF.md) already records that an
unauthenticated flood can contend the SQLite write lock. Live is the process
check. Ready is for Compose at 30s.

### M7 CVR-074, when it lands

Retention, orphan cleanup, disk-space readiness, and redacted operational
metrics. Disk-space on `/health/ready` is the readiness half. Metrics are
the other half. Cheapest first:

1. **Logs only (default).** Keep JSON events. If dashboards appear later,
   Promtail/Alloy on the NAS tails Docker logs. No new port on the converter.
2. **`GET /metrics`, internal only.** [axum-prometheus](https://github.com/ptrskay3/axum-prometheus)
   for HTTP RED (`requests_total`, duration, pending). Custom gauges: queue
   depth, active jobs, data-root bytes free. Bind on the internal network or
   a loopback port. Never publish it through Caddy on the LAN.
3. **OTLP.** Only if a collector already lives on the NAS. The OpenTelemetry
   Rust Prometheus exporter tells new projects to use OTLP instead
   ([readme](https://github.com/open-telemetry/opentelemetry-rust/blob/main/opentelemetry-prometheus/README.md)).
   Do not add a collector to this Compose file.

Allowed metric labels: HTTP method, route template, status class, terminal
job status, engine name. Forbidden: filename, path, job id (unbounded
cardinality), `clientRunId`. Job ids belong in logs, where they correlate
one incident.

Do not send converter failures to Sentry or PostHog. Document-adjacent
payloads stay on the box.

## Steal / reject

Steal the host/event split and the Meter contract. Steal Yaak’s
listen-before-command rule, not its SSE engine. Steal Datalab’s submit / poll
/ fetch shape, which [`src-tauri/src/providers.rs`](../src-tauri/src/providers.rs)
already copied and which `ConversionJob` should keep presenting to the Mac.

Reject Celery-style `update_state(PROGRESS)` (needs a broker, out of scope).
Reject a Kubernetes watch (one SQLite writer, one consumer; GET is the watch).
Reject a Grafana sidecar. Reject climbing rung 3 to animate a bar.
