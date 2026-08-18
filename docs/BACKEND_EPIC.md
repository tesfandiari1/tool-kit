# Epic: Rust conversion backend

**Status:** In progress — M2 closed, M3 AnyDoc next; M6 may run in parallel
**Current milestone:** M3 — AnyDoc and local format routing
**Latest verified checkpoint:** `5c626e1` — M3 Increments 0-3, contract 0.4.0
**Target:** CPU-only Rust/Axum modular monolith
**Architecture:** [`BACKEND_SERVICE_PLAN.md`](BACKEND_SERVICE_PLAN.md)
**Immediate plan:** [`BACKEND_EXECUTION_PLAN.md`](BACKEND_EXECUTION_PLAN.md)
**Session handoff:** [`HANDOFF.md`](HANDOFF.md)
**Last updated:** 2026-08-18

## Outcome

Deploy one small Dockerized backend on the local network. The desktop app
uploads documents, polls durable jobs, and downloads Markdown. The backend uses
`pdf-inspector` for PDFs, AnyDoc for supported non-PDF formats, and Datalab for
policy-approved difficult-document fallback.

The existing Rust service is the production foundation. The immediate work is
to make its already-working PDF vertical slice durable, then add one engine or
integration boundary at a time.

## Invariants

- Extend the existing Rust/Axum backend; do not replace it with FastAPI.
- One converter container and one in-process durable worker initially.
- SQLite and job artifacts live under one host-local `/data` dataset.
- Worker and parser concurrency start at one.
- Keep the PDF parser behind its bounded child-process boundary.
- PDF -> `pdf-inspector`; proven non-PDF formats -> AnyDoc; approved hard cases
  -> Datalab.
- CPU-only. No GPU, local OCR, or local model runtime.
- No PostgreSQL, Redis, RabbitMQ, Celery, Taskiq, generic queue framework, or
  distributed scheduler.
- Preserve the existing HTTP/OpenAPI contract unless a versioned change is
  necessary.
- Keep existing desktop changes untouched until the desktop-integration
  milestone.
- Do not extract a service without an observed runtime, resource, scaling,
  release, failure-isolation, security, or ownership reason.

## Milestones

| Milestone | Outcome | Status | Depends on |
|---|---|---|---|
| M0 | Architecture, ownership boundaries, and scaffold | Complete | — |
| M1 | Authenticated loopback PDF conversion vertical slice | Complete | M0 |
| M2 | Durable SQLite jobs, sources, and artifacts | Complete | M1 |
| M3 | AnyDoc and proven non-PDF local conversion | Planned | M2 |
| M4 | Corpus-calibrated routing and quality policy | Planned | M3 |
| M5 | Restart-safe Datalab fallback and privacy policy | Planned | M4 |
| M6 | Desktop app uses the backend | Planned | M2 (M5 for one CVR-067 scenario) |
| M7 | LAN deployment, operations, and recovery | Planned | M6 |
| M8 | Evaluation, reversible cutover, and cleanup | Planned | M7 |

## M0 — Architecture and boundaries

- [x] **CVR-001:** Record the dirty-worktree baseline and backend-owned paths
  without modifying concurrent desktop work.
- [x] **CVR-002:** Create an independent `backend/` Rust crate rather than a
  root Cargo workspace.
- [x] **CVR-003:** Select the Rust/Axum service as the production backend
  foundation.
- [x] **CVR-004:** Select a modular monolith with one converter container, one
  in-process worker, and parser child-process isolation.
- [x] **CVR-005:** Select embedded SQLite plus a host-local `/data` dataset for
  durable metadata and artifacts.
- [x] **CVR-006:** Keep `pdf-inspector` as the PDF engine, add AnyDoc for
  supported non-PDFs, and retain Datalab as the difficult-document fallback.
- [x] **CVR-007:** Remove FastAPI, PostgreSQL, Redis, GPU, and speculative
  microservices from the baseline.
- [x] **CVR-008:** Define extraction triggers for future capabilities instead
  of scaffolding unused services.

**M0 gate:** Backend documents and the scaffold describe one consistent
Rust/SQLite, CPU-only direction, and the desktop worktree boundary is recorded.

## M1 — Existing loopback PDF vertical slice

- [x] **CVR-010:** Add the standalone Rust/Axum crate with typed fail-fast
  configuration, structured logs, generated request IDs, and stable errors.
- [x] **CVR-011:** Add liveness, startup readiness, capabilities,
  submission/status/artifact routes, and the OpenAPI contract.
- [x] **CVR-012:** Add bootstrap bearer authentication with secret-file loading
  and constant-time comparison.
- [x] **CVR-013:** Add bounded multipart PDF streaming, signature/field
  validation, process-local idempotency, and upload backpressure.
- [x] **CVR-014:** Pin `pdf-inspector = 1.15.0` with default features disabled
  and wrap it in the short-lived `tool-kit-pdf-worker` process.
- [x] **CVR-015:** Enforce worker identity/CMap checks, environment clearing,
  parser timeout, output limits, protocol validation, and one-parser
  concurrency.
- [x] **CVR-016:** Return `needs_remote` for scanned, image-based, mixed,
  encoding-damaged, OCR-required, empty, or oversized local output.
- [x] **CVR-017:** Validate and atomically publish non-empty Markdown plus a
  versioned provenance manifest.
- [x] **CVR-018:** Add the hardened multi-stage image and loopback-only Compose
  service with non-root execution, a read-only root, dropped capabilities, and
  resource/log limits.
- [x] **CVR-019:** Add unit and HTTP-contract coverage for auth, uploads,
  idempotency, routing, parser failure, artifact publication, and OpenAPI.

**M1 gate:** The loopback service converts a complete native-text PDF and
safely returns Markdown; non-local PDFs finish as `needs_remote` without
partial output. All 28 Rust tests passed on 2026-08-17.

**Known M1 boundary:** `JobRegistry` is process-local, `ArtifactStore` owns a
temporary session directory, and each accepted job starts a `tokio::spawn`.
Restart durability, AnyDoc, Datalab, Caddy, retention, backups, and desktop HTTP
wiring are not implemented. M1 is not approved for LAN deployment.

## M2 — Durable SQLite jobs and artifacts

Current implementation snapshot:

| Area | State | Evidence / remaining work |
|---|---|---|
| Contract freeze | Complete | Backend OpenAPI and generated-client baseline recorded without modifying desktop schema |
| SQLite repository | Complete | File-backed migration, WAL/FULL/FK checks, transactional idempotency/capacity, FIFO claims, and recovery primitives |
| Durable ingest/artifacts | Complete | Persistent source and attempt layout, atomic publication, hashes, cleanup, quarantine primitives, and restart-visible reads |
| Single worker | Complete | One FIFO runner, Notify plus polling, validated-handle parser input, fail-closed execution, and bounded cancellation/shutdown |
| Startup reconciliation | Complete | State-specific recovery, bounded manifest/artifact validation, fresh attempts, and conservative orphan quarantine |
| Runtime/OpenAPI/Compose | Complete | Live readiness, persistent capability response, `/data` named volume, and temporary generated-schema diff |
| Release gates | Complete | Fault barriers, integrity matrices, and graceful plus forced-kill restart smokes on the built image |

- [x] **CVR-020:** Add SQLx 0.9 with default features disabled and only
  `runtime-tokio`, `sqlite`, `migrate`, and `macros`; add embedded migrations,
  `build.rs` migration tracking, foreign keys, WAL mode, full synchronous
  durability, busy timeout, and migration/startup tests.
- [x] **CVR-021:** Add fail-fast `/data` configuration and a persistent layout
  for the fixed `converter.sqlite` database filename, immutable job sources,
  attempt staging, artifacts, and pre-acceptance quarantine.
- [x] **CVR-022:** Replace the in-memory `JobRegistry` with a repository for
  conversions, attempts, artifacts, and durable idempotency using the literal
  M2 authentication scope `bootstrap`.
- [x] **CVR-023:** Make submission durable: publish the validated source and
  commit the conversion, first attempt, and idempotency decision before
  returning `202 Accepted`; remove the newly staged source on replay, conflict,
  capacity rejection, or transaction failure.
- [x] **CVR-024:** Replace per-job task spawning with one bounded worker loop
  that transactionally claims queued work. Use a notification for wake-up and
  SQLite polling for restart recovery.
- [x] **CVR-025:** Persist every state transition, active attempt, route,
  classification, warning, stable failure, engine version, artifact path, and
  hash.
- [x] **CVR-026:** Reconcile `queued`, `converting_local`, `finalizing`, and
  successful jobs at startup; verify immutable source integrity, preserve
  attempt history, transition corrupted successes to
  `failed`/`artifact_integrity_failed` without serving artifacts, and quarantine
  pre-acceptance orphan trees without deleting them.
- [x] **CVR-027:** Add live SQLite and writable-volume readiness, bounded
  graceful shutdown, truthful persistent-durability/active-capacity
  capabilities, the matching backend OpenAPI update, and a Compose `/data` mount
  without adding a service. Generate/diff the TypeScript schema only in a
  temporary location; defer the committed desktop schema update to M6.
- [x] **CVR-028:** Test durable idempotency, process restart, queued-job
  recovery, interrupted conversion/publication, corrupt or missing artifacts,
  corrupt or substituted sources, database failure, disk-write failure,
  rejection cleanup, pre-acceptance quarantine, deterministic crash windows,
  and the `artifact_integrity_failed` public contract.
- [x] **CVR-029:** Run the full Rust checks, Compose validation, image build,
  a graceful stop/start smoke, and a forced-kill recovery smoke before closing
  M2.

**M2 gate:** An accepted job, source, and idempotency record survive a full
converter restart. A recovered job cannot execute concurrently or report
success without validated, downloadable artifacts. A corrupted prior success is
reported as `failed`/`artifact_integrity_failed`, retains its audit metadata, and
serves no artifacts.

## M3 — AnyDoc and local format routing

Execution increments, mirroring the M2 style. Each is independently gated.

| Increment | Content | Exit |
|---|---|---|
| 0 — AnyDoc spike | Pin `=0.1.9`; build native and image; convert one file per family; hostile-input tests; `cargo tree` feature check; image-size delta | Done 2026-08-18: go; evidence in the verification log |
| 1 — Engine seam (CVR-031) | Engine-neutral `EngineOutcome`; de-PDF `EngineFailure` strings | Done 2026-08-18: all 119 tests green, no behavior change |
| 2 — AnyDoc adapter (CVR-032, CVR-034) | `engines/anydoc.rs` with the error mapping; refuses PDF bytes; in-process per the owner decision | Done 2026-08-18: docx and xlsx round-trip the durable path; a PDF fed to the adapter fails closed |
| 3 — Validation and capabilities (CVR-033) | Content-sniff plus the CSV extension hint; proven formats only; OpenAPI and capabilities together | Done 2026-08-18: admission table drives upload validation and `inputFormats`; `engines` array; OpenAPI 0.4.0 |
| 4 — Fixtures and gates (CVR-035, CVR-036, CVR-037) | Per-family success and bounded-failure fixtures; diagnostics without content leakage; full gate plus container smoke | M3 gate |

- [x] **CVR-030:** Verify and pin a compatible AnyDoc revision, Rust API,
  licenses, supported formats, and CPU-only container requirements. Verified
  2026-08-18: crate `anydoc` 0.1.9, MIT, `rust_version` 1.88, pure-Rust
  dependency tree, in-process `to_markdown_bytes` API with content-based
  format detection. Spike evidence is in the verification log; the pin
  `=0.1.9` is decided and lands in `Cargo.toml` with the adapter in
  Increment 2.
- [x] **CVR-031:** Add the smallest common engine outcome needed for a second
  local engine; do not create a universal document AST. `engines/outcome.rs`
  carries `EngineAnalysis` (classification plus opaque diagnostics JSON) and
  the shared `EngineOutcome`/`EngineFailure`; the PDF worker's `Inspection`
  stays inside the PDF engine.
- [x] **CVR-032:** Implement the AnyDoc adapter through its supported Rust
  integration. **Owner decision, 2026-08-18: in-process**, not a child worker.
  The spike showed every hostile fixture failing as a typed error through
  AnyDoc's internal limits, and a second worker binary plus wire protocol was
  machinery the self-hosted single-user threat model does not justify.
  `engines/anydoc.rs` runs `to_markdown_bytes` under `spawn_blocking` plus
  `catch_unwind`; a parser panic becomes `worker_crash`, recoverable like any
  interrupted attempt. The adapter refuses PDF input outright, so AnyDoc's
  embedded pdf-inspector can never bypass the isolated PDF worker. The
  child-worker shape in `pdf_inspector.rs` remains the documented fallback if
  hostile input ever defeats AnyDoc's limits.
- [x] **CVR-033:** Expand upload validation and capabilities to only the file
  types proven by fixtures. One admission table (`SOURCE_FORMATS`) drives the
  extension/media-type/magic checks at upload, engine selection at execution,
  and the capabilities `inputFormats` list. Detection is content-based inside
  the engine; admission checks the container magic. The advertised set is
  PDF, DOCX, and XLSX — the formats with passing round-trip fixtures.
  Remaining families join the table with their fixtures in Increment 4.
- [x] **CVR-034:** Route PDFs exactly once through `pdf-inspector` and supported
  non-PDF documents exactly once through AnyDoc. Engine selection keys on the
  stored source media type; the claim transaction stamps the matching engine
  identity, and a queued row with an engine-less media type fails closed
  instead of poisoning the queue.
- [ ] **CVR-035:** Persist engine/version, warnings, fallback reason, output
  hash, and engine-specific diagnostics without leaking document content.
- [ ] **CVR-036:** Add licensed or synthetic success and safe-failure fixtures
  for each advertised format family.
- [ ] **CVR-037:** Re-run the container and contract gates with AnyDoc present,
  including timeout, output ceiling, and malformed-input behavior.

**M3 gate:** Every advertised local format has a passing conversion fixture and
a bounded failure fixture. PDFs are never parsed twice, and incomplete output
never silently succeeds.

## M4 — Routing and quality policy

- [ ] **CVR-040:** Populate the labeled corpus with native, scanned, image,
  mixed, dense-table, multi-column, form, encoding-damaged, encrypted,
  malformed, and over-limit PDFs.
- [ ] **CVR-041:** Define stable quality signals and fallback reason codes from
  real engine output; avoid filename and page-count heuristics.
- [ ] **CVR-042:** Implement explicit `standard`, `local_only`, and
  `best_quality` routing policy as a pure, table-driven decision boundary.
- [ ] **CVR-043:** Calibrate dense/complex routing against the corpus and record
  false-local-success and unnecessary-remote-routing rates. Sized as a fixture
  regression suite first; add rate calibration only when real volume exists.
- [ ] **CVR-044:** Expose route, reason codes, warnings, and policy decisions in
  status and provenance without exposing document content.
- [ ] **CVR-045:** Add regression tests that prevent scanned, mixed, damaged,
  empty, or incomplete Markdown from silently succeeding.

**M4 gate:** Routing is deterministic, explainable, corpus-backed, and privacy
aware. Complexity metadata alone does not cause external transmission.

## M5 — Datalab fallback

- [ ] **CVR-050:** Add typed Datalab configuration and load its credential only
  inside the backend.
- [ ] **CVR-051:** Implement a bounded submit/poll/result HTTP adapter with
  stable errors, timeouts, retry classification, and redacted logs.
- [ ] **CVR-052:** Extend the durable state model for remote submission,
  polling, finalization, and uncertain acceptance through a contract-reviewed
  migration.
- [ ] **CVR-053:** Persist the Datalab request ID before polling and resume a
  known request after restart.
- [ ] **CVR-054:** Represent uncertain billable submission explicitly and never
  blindly resubmit a request whose remote acceptance is unknown.
- [ ] **CVR-055:** Apply the M4 policy to scanned, mixed, incomplete, empty, and
  approved recoverable local failures; enforce that `local_only` never sends
  bytes externally.
- [ ] **CVR-056:** Validate remote Markdown and publish it through the same
  attempt-scoped artifact boundary.
- [ ] **CVR-057:** Test success, privacy denial, rate limiting, outage, timeout,
  malformed output, interrupted polling, and uncertain submission.

**M5 gate:** Difficult fixtures route correctly, `local_only` inputs never
leave the network, and a known or uncertain remote request is never duplicated.

## M6 — Desktop integration

- [ ] **CVR-060:** Add backend URL and device-token settings outside React
  state, keeping the token in the macOS Keychain.
- [ ] **CVR-061:** Generate or validate and commit the Tauri HTTP client/schema
  against the backend OpenAPI contract, including the capability correction
  temporarily diffed during M2.
- [ ] **CVR-062:** Stream selected files from Tauri to the backend with an
  idempotency key and stable client run ID.
- [ ] **CVR-063:** Poll durable job state and recover an in-progress desktop run
  after app restart.
- [ ] **CVR-064:** Download Markdown through Tauri and preserve current
  collision-safe output naming.
- [ ] **CVR-065:** Show selected route, warnings, privacy decisions, and
  actionable failures without exposing provider credentials to the webview.
- [ ] **CVR-066:** Keep the existing direct-provider path behind a reversible
  switch until the deployment gate passes.
- [ ] **CVR-067:** Add end-to-end desktop tests for local success, Datalab
  fallback, restart recovery, backend unavailability, and retry-safe replay.

**M6 gate:** The desktop completes representative local and remote conversions,
recovers active jobs, and never exposes backend or provider credentials to the
webview.

## M7 — LAN deployment and operations

- [ ] **CVR-070:** Add Caddy with internal HTTPS and expose only the proxy on
  the selected LAN interface.
- [ ] **CVR-071:** Replace the bootstrap deployment credential with rotatable
  per-device credentials and document client certificate/token provisioning.
- [ ] **CVR-072:** Keep the converter on an internal network and preserve
  non-root execution, narrow mounts, dropped capabilities, read-only root,
  resource limits, and bounded logs.
- [ ] **CVR-073:** Place SQLite, artifacts, and Caddy state on explicit
  host-local persistent datasets; keep the live database off SMB/NFS.
- [ ] **CVR-074:** Add retention, conservative orphan cleanup, disk-space
  readiness, and redacted operational metrics.
- [ ] **CVR-075:** Document and prove coordinated backup and restore for the
  SQLite database plus artifacts.
- [ ] **CVR-076:** Generate an SBOM and complete transitive license, image,
  dependency-advisory, and pinned-version review.
- [ ] **CVR-077:** Add operator runbooks for install, certificate trust,
  upgrade, rollback, token rotation, backup, restore, and common failures.
- [ ] **CVR-078:** Run restart, overload, timeout, disk-full, Datalab-outage,
  TLS, auth, and restore tests on the target host.

**M7 gate:** The target host passes security-boundary, recovery, overload, and
backup/restore tests, and only Caddy is reachable on the LAN.

## M8 — Evaluation, cutover, and cleanup

- [ ] **CVR-080:** Freeze the labeled corpus and expected routing/results for
  release comparison.
- [ ] **CVR-081:** Capture current desktop-path baselines and compare output
  completeness, routing, latency, and failure behavior.
- [ ] **CVR-082:** Run the full desktop-to-backend acceptance suite and a real
  multi-document soak on the deployed host.
- [ ] **CVR-083:** Make the backend route the desktop default through a
  reversible configuration change.
- [ ] **CVR-084:** Exercise rollback and verify unfinished jobs and user files
  remain understandable and recoverable.
- [ ] **CVR-085:** Record the release evidence, known limitations, restore
  point, and operator sign-off.
- [ ] **CVR-086:** Remove the obsolete direct conversion path only in a
  separately approved cleanup after the rollback window.

**M8 gate:** The backend is the verified default, rollback has been exercised,
and legacy code is removed only after a separate approval.

## Future capability rule

Transcription, summarization, and other capabilities are outside this epic.
Start each as a Rust provider adapter or bounded module when that fits. Extract
a separate internal service only for a demonstrated incompatible runtime,
materially different resource profile, independent scaling/release need,
failure/security boundary, or independent consumer. Reconsider PostgreSQL or a
broker only if multiple service instances or hosts must coordinate durable
work.

## Explicit non-goals

- FastAPI or another parallel document API
- PostgreSQL, Redis, or a message broker at initial scale
- GPU or accelerator support
- Local OCR or model serving
- Multiple converter replicas or distributed orchestration
- A generic job/workflow platform
- A universal document AST
- A workflow dashboard, SSE, or WebSockets
- Per-page local/remote merging
- Public-internet deployment
- Transcription or summarization implementation in this epic

## Next execution sequence

Execute M3 in the backend session, in the increment order in the M3 section
above: spike, engine seam, adapter, validation and capabilities, fixtures and
gates. M6 desktop integration may run in parallel in a second session under
the ownership table in [`HANDOFF.md`](HANDOFF.md). The one coordination point
is `pnpm generate:api` after CVR-033 widens `inputFormats`, run by whichever
session lands second; the M3 session never writes `src/app/api/schema.ts`.

The M2 plan ([`BACKEND_EXECUTION_PLAN.md`](BACKEND_EXECUTION_PLAN.md)) is
complete. The M6 plan, its increment order, and the parallel-session reasoning
are in [`DESKTOP_EXECUTION_PLAN.md`](DESKTOP_EXECUTION_PLAN.md). See
[`HANDOFF.md`](HANDOFF.md) before touching files.

## Verification log

### 2026-08-17 — Foundation and PDF vertical slice

- The independent Rust/Axum backend, direct `pdf-inspector` worker, OpenAPI
  contract, authentication, bounded upload path, atomic artifacts, Dockerfile,
  and loopback Compose definition are present.
- `cargo fmt --check`, `cargo check --locked --offline`,
  `cargo clippy --locked --offline --all-targets -- -D warnings`, and
  `cargo test --locked --offline` passed; the test suite contains 28 tests.
- `docker compose -f backend/compose.yaml config --quiet` passed.
- The image build and running-container smoke were not rerun during the latest
  architecture audit, so they remain explicit M2 exit checks.

### 2026-08-17 — Architecture correction

- The short-lived FastAPI/PostgreSQL draft was superseded after confirming
  that both primary Firecrawl engines are Rust-native and the existing service
  already owns the required backend boundary.
- Embedded SQLite plus one bounded worker was selected for the expected
  one-host, low-volume workload.
- GPU, local OCR, Redis, PostgreSQL, a queue product, and speculative
  microservices remain out of scope.

### 2026-08-17 — M2 Increment 0 contract freeze

- Created backend-only checkpoint `433b6c3` on `codex/backend-m2`; no desktop
  or root README path was staged.
- Recorded backend OpenAPI SHA-256
  `99e140518cc97f5be69b307918b7dd76afee28d7cede230781a678e15de82af3`.
- `openapi-typescript 7.13.0` generated the M1 client schema under `/private/tmp`;
  its SHA-256 matched the untouched desktop schema at
  `333c0250c2041df7279c6a8f5d0e1d3bf334b30dbd81423eb0b2cff10bb66612`.
- Extracted reusable HTTP test support and added restart-harness plus public
  status/profile characterization. The full backend suite now contains 30
  passing tests.
- Format, check, Clippy with warnings denied, and Compose validation passed.

### 2026-08-17 — Repo hygiene and latest-stable deps

- Planning docs moved to `docs/`. Session state lives in `docs/HANDOFF.md`.
- Added root `LICENSE` (MIT), CI, Dependabot, `rust-toolchain.toml` (1.97.1),
  `.node-version` (24), and `packageManager` / `engines` in `package.json`.
- Catch-up to latest stable except TypeScript 7, `keyring` 4, `libc` 1.0-alpha,
  and the exact `pdf-inspector =1.15.0` pin. Those exceptions are load-bearing;
  do not "finish" them in an M2 session.
- Shared repo files are not backend-owned. Do not revert them in an M2 patch.

### 2026-08-17 — M2 Increments 1-3 implementation checkpoint

- Replaced the in-memory registry with a file-backed SQLite repository and
  embedded migration. Durable idempotency, capacity precedence, FIFO claims,
  exact state transitions, restart-visible reads, and bounded recovery-attempt
  primitives are implemented.
- Replaced temporary artifact ownership with a persistent `/data` layout for
  immutable sources, attempt staging, published Markdown/manifests, and
  collision-safe pre-acceptance quarantine.
- Submission now commits the durable job before returning `202`; replay,
  conflict, and capacity paths clean their alternate staged trees.
- Replaced per-request conversion tasks with one FIFO runner using SQLite as
  the queue, `Notify` for low-latency wake-up, and polling for missed signals.
- The parser receives the already-open validated source through stdin. The
  worker independently verifies its stored byte length and SHA-256 before
  parsing, so path replacement or in-place mutation cannot change the input.
- The settled post-hardening gate passed format, check, Clippy, `git diff
  --check`, and 77 backend tests. Coverage includes concurrent FIFO claims,
  missed notifications, source path and in-place mutation, persistence
  invariant propagation, stop/claim races, stubborn task cancellation, and a
  shared HTTP/job shutdown deadline.
- Committed the backend-only durable-runner checkpoint as `f189265`; concurrent
  frontend, Tauri, dependency, Docker, and unrelated documentation changes were
  not staged with it.
- Live readiness, the M2 capabilities/OpenAPI delta, Compose persistence, and
  container restart smokes remain pending.

### 2026-08-17 — M2 Increment 4 startup-recovery checkpoint

- Startup reconciliation now runs before the durable worker starts and handles
  queued, interrupted, finalizing, and succeeded rows through explicit
  state-specific rules.
- Recovered work always receives a fresh attempt. Immutable sources are
  length/hash validated before requeue, recovery attempts are bounded, and
  corrupt sources fail without parser execution.
- Published bundles use one shared, bounded validator for finalization,
  startup, status, listing, and download. It verifies the exact two-file
  directory, strict manifest identity, output size/hash, and already-open file
  handle before success or streaming.
- A corrupted historical success becomes
  `failed`/`artifact_integrity_failed`; its artifact rows and files remain for
  audit, while downloads fail closed.
- Canonical UUID job directories without database ownership move into the
  existing collision-safe `quarantine/pre-acceptance` namespace; recovery does
  not recursively inspect or delete unknown trees.
- The settled gate passed format, check, Clippy with warnings denied,
  `git diff --check`, and 90 backend tests: 54 library, 1 server, 3 parser
  worker, and 32 HTTP-contract tests. The restart suite runs on the default
  test stack.
- The backend code, recovery tests, and this checkpoint record were committed
  together as `449d7cb`; no Increment 4 code remains uncommitted at this
  handoff.
- Live readiness, the persistent capabilities/OpenAPI delta, Compose `/data`
  wiring, and built-container restart smokes remain for Increments 5-6.

### 2026-08-17 — M2 Increment 5 runtime and Compose checkpoint

- `/health/ready` is a real probe. It runs three checks concurrently, each under
  a two-second timeout: a SQLite write transaction, a create-and-remove probe
  file under `<data root>/.health/`, and the job runner's failed/stopped flags.
  All three pass returns `200` with `status: "ready"`, any failure returns `503`
  with `status: "not_ready"` and the per-check detail. The route is
  unauthenticated, so the body carries pass or fail only and the cause goes to
  the log. `/health/live` stays dependency-free.
- The probe never touches `jobs/`. Its `.health` directory sits beside the job
  tree, so startup reconciliation and quarantine never see it, and the probe
  file is removed by a `Drop` guard so a failed write or a cancelled check
  cannot leave one behind.
- Capabilities now report `durability: "persistent"` and `maxActiveJobs`.
  OpenAPI moved to `0.3.0` with a new `ReadinessResponse` schema and both `200`
  and `503` documented on `/health/ready`. `backend/Cargo.toml` moved to `0.3.0`
  so `serviceVersion` stays in lockstep, and the contract test now pins
  `info.version` to `CARGO_PKG_VERSION`.
- The TypeScript client delta was generated and diffed under
  `/private/tmp` only. `src/app/api/schema.ts` is byte-identical, as M6 requires.
- Two latent breaks were fixed. The Dockerfile never copied `migrations/` or
  `build.rs`, so `sqlx::migrate!` could not compile in the image. Compose never
  mounted `/data`, so nothing persisted. The runtime stage now runs
  `install -d -o 10001 -g 10001 -m 0700 /data`, because a fresh named volume
  otherwise lands as `root:root` and the service runs as `10001`.
- Both fixes were checked against a real build. `docker build -t
  tool-kit-converter:m2 backend` succeeds, and `ls -ldn /data` inside that image
  reports mode `drwx------` owned by `10001:10001`.
- Compose mounts the named volume `converter-data` at `/data`, sets
  `TOOLKIT_CONVERTER_DATA_DIR` explicitly, carries a commented host bind-mount
  alternative with the SMB/NFS warning, and mirrors the image healthcheck
  timing (`interval 30s`, `timeout 5s`, `start_period 30s`, `start_interval 2s`,
  `retries 3`). The `/tmp` tmpfs stays because the PDF worker calls
  `tempfile::tempfile()`.
- The artifact-store initialization error now names the directory it could not
  create, so a native start against the container default `/data` says which
  path failed instead of a bare `Read-only file system`.
- Gate run from the repo root:
  `cargo fmt --manifest-path backend/Cargo.toml --all -- --check`,
  `cargo check --locked --offline --manifest-path backend/Cargo.toml --all-targets`,
  `cargo clippy --locked --offline --manifest-path backend/Cargo.toml --all-targets -- -D warnings`,
  `cargo test --locked --offline --manifest-path backend/Cargo.toml`,
  `git diff --check`,
  `docker compose -f backend/compose.yaml config --quiet`, and
  `docker build -t tool-kit-converter:m2 backend`. All passed with 96 backend
  tests: 56 library, 1 server, 3 parser worker, and 36 HTTP contract.
- Clippy on macOS never compiles the `#[cfg(target_os = "linux")]` blocks in
  `backend/src/bin/tool-kit-pdf-worker.rs`. This local pass is necessary and not
  sufficient. CI is the only gate that lints them.
- Increment 5 is committed as `2f3158d` and merged to `main` in PR #4 as
  `b70ce44`, with the frontend, backend, desktop, and GitGuardian CI checks
  all green. The graceful and
  forced-kill restart smokes remain for Increment 6.

### 2026-08-17 — M2 Increment 6 failure and release gates

Closes CVR-028 and CVR-029, and closes M2.

- Added `backend/src/faults.rs`, a deterministic crash barrier armed only by
  tests. Four call sites: after the claim, after the attempt enters
  `finalizing`, after the publication rename, and immediately before the success
  commit. All four sit between committed transactions, so a parked task holds no
  SQLite write lock and a restarted `AppState` opens the same database. Arming
  is unreachable from configuration, the environment, or any HTTP route, and the
  live path costs one relaxed atomic load per barrier.
- **An Increment 3 checkbox had been ticked without the code.** "Add a test-only
  deterministic fault barrier after transactional claim" was marked done and no
  barrier existed. Until now every recovery test injected a stored state
  directly, which proves recovery handles each state but never proves the code
  reaches them in order. `crash_before_success_commit_is_indistinguishable_from_after_publish`
  now asserts the database holds zero artifact rows while the published bundle
  is already complete and hash-consistent on disk. That is the publish-before-
  commit ordering, stated as a test.
- Added a ten-case integrity matrix in `tests/integrity_matrix.rs`: missing,
  truncated, hash-mismatched at equal length, non-regular, and symlinked, for
  both the immutable source and the published bundle. Nine of the ten were
  untested. The only prior source-corruption test changed the byte length too,
  so the SHA-256 comparison short-circuited and never ran.
- Added `tests/failure_modes.rs`: conflict at active capacity, which takes the
  Conflict branch before the capacity check and so has different cleanup;
  graceful shutdown draining a genuinely in-flight conversion; a locked database
  bounded by the busy timeout; an unwritable data root; and migration failure.
- Added `tests/crash_recovery.rs`, which also reaches the
  `ConversionState::Queued` arm of startup reconciliation for the first time.
- Added `backend/scripts/container-smoke.sh` and `compose.yaml`
  `stop_grace_period: 45s`. Docker's 10s default was killing the service 20s
  before its own 30s shutdown grace expired, so every `docker compose stop`
  looked like a crash and left an interrupted job for recovery.
- Backend tests went from 96 to 119: 59 library, 1 server, 3 parser worker, 36
  HTTP contract, 5 crash recovery, 10 integrity matrix, 5 failure modes. Three
  consecutive full runs, no flakes. Format, `cargo check`, Clippy with warnings
  denied, `git diff --check`, and `docker compose config` all pass.

Container evidence, Docker 29.5.3 and Compose v5.1.4, image
`sha256:7ce986e7…`, run from `38d3028`:

- Graceful: readiness reported `database`, `dataRoot`, and `worker` all `ok`;
  capabilities reported `durability: persistent` with `maxActiveJobs` present
  and `maxEphemeralJobs` absent; a job submitted immediately before
  `stop -t 45` completed after the restart; the container exited `0`, proving a
  real SIGTERM drain rather than an escalated kill; both artifacts matched the
  ledger hash, the `ETag`, and `Content-Length`; the manifest carried the
  host-computed source hash and no filename; a replay returned the same job and
  a changed `clientRunId` returned `409`; and a second restart returned
  byte-identical artifacts.
- Forced kill: the job was held in `converting_local` by pointing
  `TOOLKIT_CONVERTER_PDF_WORKER_PATH` at a stub worker that sleeps, and the
  state was polled for rather than assumed. `kill -s SIGKILL` gave exit `137`
  with 238,992 bytes of uncheckpointed WAL still on the volume, so the recovered
  state existed only there. Recovery minted a fresh attempt, the job succeeded,
  and its Markdown hash matched the graceful run byte for byte. The earlier
  completed job survived intact, which matters because startup recovery
  revalidates every stored success and demotes a corrupt one.

Deferred on purpose, not silently dropped:

- **Output-write failure injection.** No portable way to make a write fail
  inside a temporary data root without the test lying about which call failed.
- **The M1 PDF fixture corpus.** The repository contains no `.pdf` file. Every
  PDF is synthesized in-process. Building a corpus is CVR-040 in M4, so that
  checklist line was unbuildable as written.
- **A true multi-process restart in Rust.** The container smoke covers it
  better, against the real release binary.
- **The unauthenticated readiness flood.** Still open, still recorded in
  `HANDOFF.md`, still belongs with Caddy and LAN exposure in M7.

The container smoke proves the graceful and forced-kill windows only. The
`finalizing` and post-rename barriers are held in the Rust suite, because
holding them from outside the process needs a code seam.

Committed on `main` as `01f1bf1`, closing M2.

### 2026-08-18 — M3 Increment 0 AnyDoc spike

**Go.** AnyDoc 0.1.9 is verified as the non-PDF engine. The initial
containment call was a bounded child worker; the owner chose in-process (see
the Increment 2 entry). Closes CVR-030's verification half; the
`Cargo.toml` pin landed with the adapter.

- Crate facts: `anydoc` 0.1.9 on crates.io, MIT, `rust_version` 1.88 against
  our 1.97.1 toolchain, edition 2024, ~14.2k lines of Rust, no build script.
  Nine runtime dependencies, all pure Rust: calamine 0.36, cfb 0.14, csv 1.4,
  encoding_rs 0.8, flate2 1, log 0.4, pdf-inspector `^1.14.2`, quick-xml
  0.41, zip 8.6. No C toolchain, no network, no ML runtime.
- **pdf-inspector unification is a non-issue.** AnyDoc requires `^1.14.2` and
  the backend pins `=1.15.0`, so one copy builds. AnyDoc enables
  pdf-inspector's `default` feature, which is empty (`default = []`); the
  heavy features (OCR, pdfium rendering, model download, Python) are all
  opt-in. The compiled PDF surface is unchanged.
- Happy path, upstream fixtures (firecrawl/anydoc `tests/fixtures`, MIT):
  text.docx in 17.9ms, sheet.xlsx in 2.5ms, pres.pptx in 9.4ms, book.epub in
  2.0ms, text.rtf in 5.6ms, and sheet.csv with an explicit `Format::Csv`
  hint. `Format::from_bytes` named every signed format correctly.
- Hostile path: six of six upstream `malformed/` and `abuse/` fixtures fail
  as typed errors with zero panics under `catch_unwind`. The limits fire
  before decompression: the 197KB zipbomb dies in 107µs with
  `ResourceLimit(max_entry_bytes: ... declares 201326759 decompressed bytes)`,
  the imagebomb likewise, deep XML at `max_xml_depth` 256, the huge table
  span at `max_expansion`, and truncated or empty inputs fail detection as
  `Unsupported`.
- CSV carries no signature: detection returns `None` and conversion asks for
  an explicit format. The upload path accepts an extension hint for CSV only,
  as CVR-033 records.
- `Format::from_bytes` on a PDF header returns `Some(Pdf)` and
  `to_markdown_bytes` converts PDFs in-process through pdf-inspector. The
  adapter therefore refuses `Format::Pdf` outright, keeping the isolated PDF
  worker as the only PDF path.
- **Containment: bounded child worker** reusing the PDF worker's supervision
  pattern. In-process looked survivable (typed errors, pre-decompression
  limits, no panics), which keeps the worker protocol simple. But a
  stack-overflow-class bug in a 0.1.x parser is not catchable in-process, and
  the child boundary keeps a parser crash away from the SQLite ledger. The
  cost is one more binary and tens of milliseconds per job against
  multi-second end-to-end runs.
- Image: `docker buildx build --builder toolkit-capped --load -t
  tool-kit-converter:m3-spike backend` succeeds. Exact sizes 40,229,571 bytes
  versus 40,203,627 for `m2`: a 25,944-byte floor delta, because AnyDoc is
  compiled but not yet referenced and LTO strips it. The real delta is
  Increment 4's measurement.
- The spike harness (`backend/examples/anydoc_spike.rs`) and the
  `Cargo.toml`/lockfile change were reverted after evidence capture.
  Fixtures stayed in a temp directory; curated, license-recorded fixtures
  enter the repo under Increment 4.

### 2026-08-18 — M3 Increment 1 engine seam

Closes CVR-031. No behavior change; all 119 backend tests pass.

- Added `backend/src/engines/outcome.rs`: `EngineAnalysis` carries a
  `DocumentClassification` plus an opaque diagnostics `serde_json::Value`, and
  `EngineOutcome`/`EngineFailure` moved here from `pdf_inspector.rs` as the
  shared engine vocabulary. `EngineFailure` messages now say "conversion
  worker" instead of "PDF worker"; the stable codes are unchanged.
- The PDF worker's `Inspection` stays inside the PDF engine, which maps it to
  `EngineAnalysis` at the outcome boundary. The serialized diagnostics are
  byte-identical to the previous `Inspection` JSON, so manifests and attempt
  rows written before this change still validate.
- `ConversionManifest.document` is now `serde_json::Value`. Old manifests
  parse unchanged, and the publication validator deserializes the PDF shape
  only when checking a pdf-inspector manifest, keyed on `engine.name`. That
  branch is where Increment 2 plugs the AnyDoc manifest check in.
- The completeness gate `is_complete_native_inspection` moved into
  `pdf_inspector.rs` with its test.
- The `inspection_encoding_failed` failure path collapsed into
  `EngineFailure::Protocol` inside the engine. No test asserted it, and the
  serialization cannot fail for the plain data structs involved.
- Gate: format, check, Clippy with warnings denied, and 119 tests all pass.

### 2026-08-18 — M3 Increments 2 and 3: AnyDoc adapter, validation, capabilities

Closes CVR-032, CVR-033, and CVR-034. Backend moves to 0.4.0.

- **Design change, owner-approved:** AnyDoc runs in-process behind
  `spawn_blocking` + `catch_unwind`, not behind a second child worker. The
  spike evidence (typed errors on every hostile fixture, pre-decompression
  limits, zero panics) plus the single-user loopback threat model made the
  extra binary, wire protocol, and supervision layer unjustified machinery.
  The engine still refuses PDF bytes outright. If AnyDoc's limits ever fail
  in the field, the fallback is the PDF worker's shape.
- `engines/anydoc.rs` re-verifies the open source handle (length and SHA-256)
  before parsing, writes the staged Markdown exactly like the PDF worker, and
  returns `EngineOutcome`. Rejections are engine-owned: `unsupported_document`
  (unknown container or PDF), `encrypted_document`, `invalid_document`,
  `document_exceeds_limits`. No `needs_remote` from AnyDoc in M3; remote
  policy is M4/M5.
- One admission table (`conversion::model::SOURCE_FORMATS`) is the single
  source of truth: upload validation (extension, declared media type,
  container magic), claim-time engine selection, and capabilities
  `inputFormats` all read it. The advertised set is PDF, DOCX, and XLSX —
  the formats with passing round-trip fixtures. Other AnyDoc families join
  with their fixtures in Increment 4.
- Migration `0002_non_pdf_source_formats.sql` rebuilds `conversions` and
  `attempts`: the source media-type CHECK widens to the AnyDoc universe and
  `classification` gains `structured_document`. SQLite cannot ALTER a CHECK,
  so both tables are rebuilt under `PRAGMA foreign_keys = OFF` using sqlx's
  `-- no-transaction` directive. A new test drives the upgrade over a
  populated M2 database through the real migrator path: rows survive, the new
  types are accepted, `text/plain` is still rejected, and
  `foreign_key_check` is clean.
- Claim stamps the engine identity from the job's stored media type via a
  closure inside the claim transaction; an engine-less media type fails the
  job closed with `source_integrity_failed` instead of looping forever.
- Contract: capabilities `engine` becomes `engines[]` (pdf-inspector 1.15.0,
  anydoc 0.1.9); `inputFormats` lists the three proven media types; upload
  error codes unify to `unsupported_source_extension`,
  `invalid_source_media_type`, and `invalid_source_signature` (the
  `invalid_pdf_*` codes are gone); the manifest `document` shape check is
  engine-keyed. OpenAPI and Cargo are 0.4.0. The desktop schema stays stale
  on purpose; `pnpm generate:api` belongs to M6.
- Two vendored fixtures (text.docx, sheet.xlsx) from the upstream AnyDoc
  corpus, MIT, recorded in `backend/tests/fixtures/SOURCES.md`.
- The harness' engine-selection tests use the real in-process engine; no
  worker binary or stub needed for AnyDoc.
- Gate: format, check, Clippy with warnings denied, `git diff --check`,
  `docker compose config`, and 127 backend tests all pass (64 library, 1
  server, 3 parser worker, 39 HTTP contract, 5 crash recovery, 10 integrity
  matrix, 5 failure modes). `docker build` succeeds; the image is
  42,886,364 bytes, +2.68 MB over the M2 image with AnyDoc linked in. The
  container smoke stays with Increment 4 (CVR-037).
