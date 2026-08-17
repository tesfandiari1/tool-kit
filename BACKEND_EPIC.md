# Epic: Rust conversion backend

**Status:** In progress
**Current milestone:** M2 — durable SQLite jobs and artifacts
**Target:** CPU-only Rust/Axum modular monolith
**Architecture:** [`BACKEND_SERVICE_PLAN.md`](BACKEND_SERVICE_PLAN.md)
**Immediate plan:** [`BACKEND_EXECUTION_PLAN.md`](BACKEND_EXECUTION_PLAN.md)
**Last updated:** 2026-08-17

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
| M2 | Durable SQLite jobs, sources, and artifacts | In progress | M1 |
| M3 | AnyDoc and proven non-PDF local conversion | Planned | M2 |
| M4 | Corpus-calibrated routing and quality policy | Planned | M3 |
| M5 | Restart-safe Datalab fallback and privacy policy | Planned | M4 |
| M6 | Desktop app uses the backend | Planned | M2, M5 |
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

- [ ] **CVR-020:** Add SQLx 0.9 with default features disabled and only
  `runtime-tokio`, `sqlite`, `migrate`, and `macros`; add embedded migrations,
  `build.rs` migration tracking, foreign keys, WAL mode, full synchronous
  durability, busy timeout, and migration/startup tests.
- [ ] **CVR-021:** Add fail-fast `/data` configuration and a persistent layout
  for the fixed `converter.sqlite` database filename, immutable job sources,
  attempt staging, artifacts, and pre-acceptance quarantine.
- [ ] **CVR-022:** Replace the in-memory `JobRegistry` with a repository for
  conversions, attempts, artifacts, and durable idempotency using the literal
  M2 authentication scope `bootstrap`.
- [ ] **CVR-023:** Make submission durable: publish the validated source and
  commit the conversion, first attempt, and idempotency decision before
  returning `202 Accepted`; remove the newly staged source on replay, conflict,
  capacity rejection, or transaction failure.
- [ ] **CVR-024:** Replace per-job task spawning with one bounded worker loop
  that transactionally claims queued work. Use a notification for wake-up and
  SQLite polling for restart recovery.
- [ ] **CVR-025:** Persist every state transition, active attempt, route,
  classification, warning, stable failure, engine version, artifact path, and
  hash.
- [ ] **CVR-026:** Reconcile `queued`, `converting_local`, `finalizing`, and
  successful jobs at startup; verify immutable source integrity, preserve
  attempt history, transition corrupted successes to
  `failed`/`artifact_integrity_failed` without serving artifacts, and quarantine
  pre-acceptance orphan trees without deleting them.
- [ ] **CVR-027:** Add live SQLite and writable-volume readiness, bounded
  graceful shutdown, truthful persistent-durability/active-capacity
  capabilities, the matching backend OpenAPI update, and a Compose `/data` mount
  without adding a service. Generate/diff the TypeScript schema only in a
  temporary location; defer the committed desktop schema update to M6.
- [ ] **CVR-028:** Test durable idempotency, process restart, queued-job
  recovery, interrupted conversion/publication, corrupt or missing artifacts,
  corrupt or substituted sources, database failure, disk-write failure,
  rejection cleanup, pre-acceptance quarantine, deterministic crash windows,
  and the `artifact_integrity_failed` public contract.
- [ ] **CVR-029:** Run the full Rust checks, Compose validation, image build,
  a graceful stop/start smoke, and a forced-kill recovery smoke before closing
  M2.

**M2 gate:** An accepted job, source, and idempotency record survive a full
converter restart. A recovered job cannot execute concurrently or report
success without validated, downloadable artifacts. A corrupted prior success is
reported as `failed`/`artifact_integrity_failed`, retains its audit metadata, and
serves no artifacts.

## M3 — AnyDoc and local format routing

- [ ] **CVR-030:** Verify and pin a compatible AnyDoc revision, Rust API,
  licenses, supported formats, and CPU-only container requirements.
- [ ] **CVR-031:** Add the smallest common engine outcome needed for a second
  local engine; do not create a universal document AST.
- [ ] **CVR-032:** Implement the AnyDoc adapter through its supported Rust
  integration, using a narrow subprocess only if upstream constraints require
  it.
- [ ] **CVR-033:** Expand upload validation and capabilities to only the file
  types proven by fixtures.
- [ ] **CVR-034:** Route PDFs exactly once through `pdf-inspector` and supported
  non-PDF documents exactly once through AnyDoc.
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
  false-local-success and unnecessary-remote-routing rates.
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

Execute M2 only. Do not combine persistence with AnyDoc.

1. Freeze and rerun the current M1 contract.
2. Write restart/recovery contract tests around a file-backed temporary SQLite
   database and persistent temporary artifact root.
3. Add SQLite configuration, migrations, and the repository.
4. Move source and artifact ownership from `TempDir` to generated `/data`
   paths.
5. Replace per-job task spawning with the single durable worker loop.
6. Add startup reconciliation across the database/filesystem publication
   boundary.
7. Add the Compose data volume and real readiness behavior.
8. Run all Rust, contract, Compose, image, and restart-smoke gates before
   marking M2 complete.

The detailed file and test plan is in
[`BACKEND_EXECUTION_PLAN.md`](BACKEND_EXECUTION_PLAN.md). The plan is approved;
execution starts with its bounded Increment 0 and Increment 1 scope.

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
