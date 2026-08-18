# Tool-Kit conversion backend architecture

**Status:** Approved direction; implementation in progress
**Target:** CPU-only Rust/Axum service in Docker Compose
**Current milestone:** M2 — durable SQLite jobs and artifacts
**Session handoff:** [`HANDOFF.md`](HANDOFF.md)
**Last updated:** 2026-08-17

**Implementation snapshot:** M2 contract freeze, SQLite persistence, durable
ingest/artifacts, the single FIFO worker, and startup reconciliation are
implemented and verified. Live readiness, the M2 capabilities/OpenAPI update,
Compose persistence, and container restart smokes remain pending.

## Goal

Run document-to-Markdown conversion on the local network behind one stable API
used by the desktop app. Reuse Firecrawl's document engines rather than build
parsers, and keep the deployment small enough to operate comfortably on one
TrueNAS host.

The service will:

1. accept a document upload from the desktop app;
2. durably record the job and immutable source;
3. convert native-text PDFs with `pdf-inspector` and supported non-PDF files
   with AnyDoc;
4. send scanned, mixed, or quality-failed documents to Datalab only when the
   selected privacy policy permits it; and
5. return durable status, Markdown, warnings, and conversion provenance.

The service remains CPU-only. GPU support, local OCR, and local model serving
are intentionally excluded.

## Architecture decision

Continue the existing Rust/Axum backend as the production foundation. It is
not a disposable prototype.

This is the most pragmatic path because:

- `pdf-inspector` and AnyDoc are Rust-native, so the primary engines can be
  integrated without a Python bridge;
- the existing service already implements the API contract, authentication,
  bounded uploads, PDF worker isolation, atomic artifact publication,
  container hardening, and a tested clean-PDF vertical slice;
- embedded SQLite supplies the durability needed by one low-volume service
  without a database or broker container; and
- a modular monolith gives future capabilities clear internal boundaries
  without paying the operational cost of speculative microservices.

The earlier FastAPI/PostgreSQL proposal is superseded. It would replace useful
working code and add processes without solving a current scale or runtime
problem.

## Deployment shape

```text
Desktop app
    |
    v
Caddy (only LAN-facing service)
    |
    v
Rust/Axum converter
    |-- HTTP API, auth, limits, health
    |-- SQLite repository and durable job ledger
    |-- one bounded in-process worker loop
    |-- PDF ------------> isolated pdf-inspector Rust worker
    |-- other documents -> AnyDoc Rust integration
    `-- hard cases ------> Datalab HTTP adapter, when policy permits
    |
    v
host-local /data dataset
    |-- converter.sqlite
    `-- jobs/{job-id}/...
```

The initial production Compose stack contains two services:

- `proxy`: Caddy, internal TLS, and the only published LAN port; and
- `converter`: the Rust API and its single durable worker loop.

SQLite is embedded in `converter`; it is not a separate container. Development
continues to publish the converter on loopback and can omit Caddy.

## Fixed decisions

- Preserve the existing Rust/Axum API and OpenAPI contract except for reviewed,
  truthful capability changes required by an implemented milestone.
- Run one converter container and exactly one job worker initially.
- Store job metadata, attempts, idempotency, routing, and artifact metadata in
  SQLite.
- Use SQLx 0.9 with default features disabled and only the `runtime-tokio`,
  `sqlite`, `migrate`, and `macros` features. Track embedded migration changes
  through `backend/build.rs`.
- Store immutable source files and output artifacts on the filesystem, not as
  SQLite blobs.
- Put SQLite and artifacts under one host-local `/data` dataset. Never place
  the live SQLite database on SMB, NFS, or another network filesystem.
- Configure the data directory, but keep the database filename fixed as
  `converter.sqlite` beneath it.
- Start worker and parser concurrency at one. Raise either only after measured
  demand and corpus testing justify it.
- Send PDFs directly to `pdf-inspector`; do not parse them twice merely to
  force every format through AnyDoc.
- Use AnyDoc for only the non-PDF formats proven by fixtures in M3.
- Use Datalab only for explicit policy-approved fallback.
- Keep parser execution behind a bounded child-process boundary where
  practical.
- Do not add PostgreSQL, Redis, RabbitMQ, a generic queue product, or a second
  API implementation.

This design follows SQLite's intended embedded/device-local use and its
same-host WAL constraint: [appropriate uses](https://www.sqlite.org/whentouse.html)
and [WAL limitations](https://www.sqlite.org/wal.html).

## Implemented boundary

The current M1 service already provides:

- an independent Rust/Axum crate and versioned OpenAPI contract;
- liveness, startup readiness, and capabilities endpoints;
- bearer-authenticated PDF submission, status polling, artifact listing, and
  Markdown/manifest downloads;
- bounded streaming uploads, validation, backpressure, and process-local
  idempotency;
- exact `pdf-inspector` 1.15.0 integration through a short-lived Rust worker;
- classification that refuses scanned, image-based, mixed, damaged, empty, or
  oversized local output rather than publishing partial Markdown;
- parser timeouts, output ceilings, environment clearing, protocol checks, and
  one-parser concurrency;
- validated, atomic Markdown and manifest publication; and
- a hardened multi-stage image plus loopback-only development Compose service.

The M1 boundary is intentionally not durable: `JobRegistry` is an in-memory
map, artifacts live in a temporary session directory, and every accepted job
starts a Tokio task. Jobs, idempotency records, uploads, and results disappear
on restart. AnyDoc, Datalab, Caddy, retention, backups, and desktop HTTP wiring
are also not implemented.

## Public API

Keep the current resource-oriented API:

| Endpoint | Purpose |
|---|---|
| `POST /api/v1/conversions` | Upload one document and return `202 Accepted`, `Location`, and an ID |
| `GET /api/v1/conversions/{id}` | Return durable status, route, warnings, and errors |
| `GET /api/v1/conversions/{id}/artifacts` | List validated published artifacts |
| `GET /api/v1/conversions/{id}/artifacts/markdown` | Stream completed Markdown |
| `GET /api/v1/conversions/{id}/artifacts/manifest` | Stream the provenance manifest |
| `GET /api/v1/capabilities` | Return enabled formats, profiles, capacity, and limits |
| `GET /health/live` | Process liveness |
| `GET /health/ready` | Database and writable-volume readiness once M2 lands |

`Idempotency-Key` remains required for submission. `clientRunId` correlates a
desktop run without replacing server-generated job and attempt IDs. Polling is
the initial progress mechanism; server-sent events and WebSockets are not
needed for the expected volume. M2 updates capabilities from ephemeral
durability/ephemeral-job capacity to persistent durability/active-job capacity;
the backend OpenAPI contract changes with the implementation. M2 generates and
diffs the TypeScript schema only in a temporary location to prove compatibility;
the committed desktop schema remains untouched until M6.

## Durable job model

M2 preserves the current public states:

```text
queued -> converting_local -> finalizing -> succeeded
                         |                 `-> failed
                         `-> needs_remote
```

M4/M5 may add remote-specific states through an explicit contract update, such
as `converting_remote` and `remote_submission_uncertain`. Cancellation and
manual retry remain out of the public API until their behavior is designed and
needed.

SQLite is the source of truth. A Tokio notification wakes the worker promptly,
while periodic SQLite polling guarantees progress after a missed notification
or restart. The notification is an optimization, not the queue.

Submission and execution follow these rules:

1. Generate backend-owned job and attempt IDs.
2. Stream the upload under `/data` with byte and time limits, then close,
   validate, hash, sync, and atomically place the immutable source.
3. Insert the conversion, first attempt, and idempotency record in one SQLite
   transaction.
4. Remove the newly staged job tree on replay, conflict, capacity rejection, or
   transaction failure; replay remains valid even when active capacity is full.
5. Return `202 Accepted` only after the durable source and database commit
   exist.
6. Let the single worker transactionally claim the oldest eligible job, then
   revalidate source containment, file type, byte count, and hash before parsing.
7. Record `finalizing`, validate staged output, and atomically rename the
   attempt publication directory.
8. Record `succeeded` only after published artifacts and hashes are verified.

The delivery model is at-least-once. Local conversion may be repeated after a
crash, but attempts never overwrite each other and only one validated attempt
becomes active. Datalab submission has stricter recovery rules because it can
be billable: persist a known remote request ID and resume polling; never blindly
repeat a submission whose acceptance is uncertain.

## Persistence layout

```text
/data/
  converter.sqlite
  converter.sqlite-wal
  converter.sqlite-shm
  jobs/
    {job-id}/
      source/
        input
      attempts/
        {attempt-id}/
          publication.staging/
          artifacts/
            result.md
            manifest.json
  quarantine/
    pre-acceptance/
      {job-id}/...
```

One immutable source belongs to the job. Every execution gets its own attempt
directory, so retries preserve provenance and cannot overwrite earlier output.
Staging and publication stay on the same filesystem so rename remains atomic.
All filesystem paths are generated by the backend and stored as paths relative
to `/data`; client filenames never become paths.

The minimal SQLite model contains:

- `conversions`: identity, client run, profile, source metadata, current state,
  active attempt, route, timestamps, and stable failure fields;
- `attempts`: execution identity, state, engine/version, classification,
  warnings, fallback reason, timing, and recovery count; and
- `artifacts`: attempt, kind, relative path, media type, byte count, and hash.

An authentication scope plus hashed idempotency key is unique on
`conversions`; the stored request fingerprint distinguishes safe replay from a
conflict. M2 uses the literal authentication scope `bootstrap`, which remains
stable when the bootstrap token changes; M7 introduces per-device scopes through
an explicit migration. Foreign keys, WAL mode, `synchronous=FULL`, a busy
timeout, embedded migrations, and bounded active-job capacity are required.
Historical terminal rows do not consume worker queue capacity; retention
removes them and their artifacts together later.

## Recovery rules

- `queued`: leave eligible for the worker.
- `converting_local`: after startup reconciliation, create a new attempt and
  requeue within the configured recovery limit only after revalidating the
  immutable source.
- `finalizing`: validate any published directory and hashes; complete success
  if publication finished, otherwise clean staging and retry within policy.
- `succeeded`: if a required artifact is missing, symlinked, or hash-mismatched,
  transition the public job to `failed` with `artifact_integrity_failed`, retain
  the attempt and artifact audit metadata, and serve no artifacts.
- `failed` and `needs_remote`: leave terminal until a future explicit retry or
  fallback operation is implemented.
- a pre-acceptance job tree with no database owner: move it under
  `/data/quarantine/pre-acceptance/` without following symlinks. M2 never
  auto-deletes quarantined document data; retention and reviewed deletion belong
  to M7.

Startup reconciliation finishes before readiness becomes healthy and before
the worker begins claiming new jobs. The first release supports one converter
instance only; multi-host coordination is not part of this architecture.

## Routing policy

| Input | Route |
|---|---|
| Native-text PDF with acceptable output | `pdf-inspector` |
| Scanned, image-based, mixed, empty, or quality-failed PDF | Datalab, or `needs_remote` under `local_only` |
| Supported non-PDF | AnyDoc |
| Approved recoverable local failure | Datalab when policy permits |
| Encrypted, malformed, unsupported, or over-limit input | Explicit rejection or failure; never upload automatically |

Profiles remain explicit:

- `local_only`: no document bytes leave the local network;
- `standard`: prefer local conversion, then use approved fallback; and
- `best_quality`: use the quality route defined and corpus-tested in M4/M5.

Dense-document routing will be calibrated against a labeled corpus. Page count
or a generic complexity flag alone is not enough to trigger remote upload.

## Internal module boundaries

Keep clear seams inside one Rust application:

```text
api/            HTTP parsing, response mapping, and OpenAPI contract
conversion/     use cases, state transitions, and routing policy
persistence/    SQLite repository and embedded migrations
jobs/           claim loop, execution, shutdown, and recovery
engines/        pdf_inspector, anydoc, and datalab adapters
artifacts/      staging, validation, publication, and retention
auth/config/error
```

These boundaries support tests and later extraction without introducing
network calls between components today. Do not create a universal document AST
or a generic workflow platform.

## Future capabilities

Transcription, summarization, and similar functions start as bounded modules or
provider adapters behind the same Caddy origin. A capability becomes a separate
service only when at least one observed trigger exists:

- an incompatible runtime or dependency stack, such as a Python-native local
  transcription model;
- materially different CPU or memory requirements;
- independent scaling or release cadence;
- a meaningful failure or security-isolation requirement; or
- consumers that need the capability independently of conversion.

A hosted transcription or summarization provider is normally just a Rust HTTP
adapter. A locally hosted Python model may justify a small internal Python
service. Either way, the desktop keeps one stable API origin and does not know
the internal topology.

## Security and operations baseline

- Caddy is the only LAN-published service; the converter stays on an internal
  Compose network.
- The converter runs non-root with a read-only root filesystem, dropped
  capabilities, `no-new-privileges`, resource limits, and one narrow writable
  `/data` mount.
- Parser children receive generated paths, a cleared environment, a hard
  timeout, an output ceiling, and no client-controlled command arguments.
- Datalab credentials remain backend-only; `local_only` content is never sent
  externally.
- Logs contain request/job/attempt IDs, routes, versions, timings, and stable
  error codes—not source content or credentials.
- Exact engine versions, output hashes, warnings, and fallback reasons are
  recorded in the database and manifest.
- Backup and restore treat SQLite plus job artifacts as one coordinated
  dataset. The release procedure quiesces writes or uses a verified SQLite
  backup/checkpoint process before the dataset snapshot.

## Explicit non-goals

- FastAPI or a Python rewrite of the document service
- PostgreSQL, Redis, RabbitMQ, or a queue framework at initial scale
- Multiple converter replicas, distributed workers, or cross-host leases
- GPU support, local OCR, or local model serving
- Initial decomposition into microservices
- Custom PDF/Office parsing or a universal document model
- Kubernetes, a service mesh, a workflow dashboard, SSE, or WebSockets
- Per-page local/remote result merging
- Public-internet deployment
- Transcription or summarization implementation inside this conversion epic

## Release success criteria

The conversion backend is ready to become the desktop default when:

- an accepted job, immutable source, and idempotency decision survive a full
  converter restart;
- native PDFs convert through `pdf-inspector` and proven non-PDF formats convert
  through AnyDoc;
- difficult fixtures follow the selected Datalab/privacy policy without
  duplicate remote submissions;
- no job reports success without downloadable, hash-verified artifacts;
- the desktop can upload, poll, recover, and save Markdown;
- only Caddy is reachable on the LAN;
- retention, disk limits, backup/restore, and rollback are demonstrated on the
  target host; and
- the labeled evaluation corpus passes before cutover.

Work is tracked in [`BACKEND_EPIC.md`](BACKEND_EPIC.md). The next bounded unit
is [`BACKEND_EXECUTION_PLAN.md`](BACKEND_EXECUTION_PLAN.md).
