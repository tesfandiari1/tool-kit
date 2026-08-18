# Epic: Rust conversion backend

**Status:** In progress. M3 gate met; M6 is next and may run in parallel
**Current milestone:** M3. AnyDoc and local format routing (all increments done)
**Latest verified checkpoint:** `5f298f6` plus the uncommitted M3 close-out below
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
- Worker and parser concurrency start at one **per engine** (PDF and AnyDoc each
  hold a separate permit; one PDF and one non-PDF job may parse concurrently).
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
| M0 | Architecture, ownership boundaries, and scaffold | Complete | - |
| M1 | Authenticated loopback PDF conversion vertical slice | Complete | M0 |
| M2 | Durable SQLite jobs, sources, and artifacts | Complete | M1 |
| M3 | AnyDoc and proven non-PDF local conversion | Complete | M2 |
| M4 | Corpus-calibrated routing and quality policy | Planned | M3 |
| M5 | Restart-safe Datalab fallback and privacy policy | Planned | M4 |
| M6 | Desktop app uses the backend | Planned | M2 (M5 for one CVR-067 scenario) |
| M7 | LAN deployment, operations, and recovery | Planned | M6 |
| M8 | Evaluation, reversible cutover, and cleanup | Planned | M7 |

## M0: Architecture and boundaries

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

## M1: Existing loopback PDF vertical slice

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

## M2: Durable SQLite jobs and artifacts

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

## M3: AnyDoc and local format routing

Execution increments, mirroring the M2 style. Each is independently gated.

| Increment | Content | Exit |
|---|---|---|
| 0: AnyDoc spike | Pin `=0.1.9`; build native and image; convert one file per family; hostile-input tests; `cargo tree` feature check; image-size delta | Done 2026-08-18: go; evidence in the verification log |
| 1: Engine seam (CVR-031) | Engine-neutral `EngineOutcome`; de-PDF `EngineFailure` strings | Done 2026-08-18: all 119 tests green, no behavior change |
| 2: AnyDoc adapter (CVR-032, CVR-034) | `engines/anydoc.rs` with the error mapping; refuses PDF bytes; in-process per the owner decision | Done 2026-08-18: docx and xlsx round-trip the durable path; a PDF fed to the adapter fails closed |
| 3: Validation and capabilities (CVR-033) | Content-sniff plus the CSV extension hint; proven formats only; OpenAPI and capabilities together | Done 2026-08-18: admission table drives upload validation and `inputFormats`; `engines` array; OpenAPI 0.4.0 |
| 4: Fixtures and gates (CVR-035–039) | Container smoke first; fixtures for every advertised format; AnyDoc timeout | Done 2026-08-18: eight formats proven both ways, AnyDoc smoke in-container, restart blocker fixed |
| 5: Parse-path optimization (CVR-039) | Skip redundant source re-read/hash at parse when immutable source unchanged | Optional M3.5; measure after Inc 4 |

**Architecture review (2026-08-18):** The core shape is sound: modular
monolith, isolated PDF child worker, in-process AnyDoc crate inside the
converter container (not a separate AnyDoc service). Parse stays fast
(AnyDoc: single-digit to low tens of ms on spike fixtures); the durable job
layer adds orchestration latency, not parser slowdown. Open risks: (1) M3
container smoke has not been re-run since AnyDoc linked in (+2.68 MB image);
(2) in-process AnyDoc contradicts the spike's child-worker containment
recommendation. `catch_unwind` does not catch stack-overflow-class faults;
(3) double source read/hash at upload and parse is avoidable overhead. See
CVR-037–039 and the containment note on CVR-032.

**Performance ladder (do not ticket separately: follow in order):**

1. In-process AnyDoc, concurrency 1/engine (current).
2. AnyDoc hard timeout (CVR-037).
3. Skip redundant source re-read/hash (CVR-039, optional M3.5).
4. Raise AnyDoc concurrency after memory profiling (post-M3 gate only).
5. Child-worker fallback only if containment fails in the field (spike
   documented shape in `pdf_inspector.rs`).

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
  hostile input ever defeats AnyDoc's limits. **Containment policy (CVR-038):**
  keep in-process for M3; add a hard outer timeout in CVR-037; revisit the
  child-worker shape only if a field crash corrupts or kills the converter.
  Do not build `tool-kit-anydoc-worker` preemptively.
- [x] **CVR-033:** Expand upload validation and capabilities to only the file
  types proven by fixtures. One admission table (`SOURCE_FORMATS`) drives the
  extension/media-type/magic checks at upload, engine selection at execution,
  and the capabilities `inputFormats` list. Detection is content-based inside
  the engine; admission checks the container magic. The advertised set is
  PDF, DOCX, and XLSX: the formats with passing round-trip fixtures.
  Remaining families join the table with their fixtures in Increment 4.
- [x] **CVR-034:** Route PDFs exactly once through `pdf-inspector` and supported
  non-PDF documents exactly once through AnyDoc. Engine selection keys on the
  stored source media type; the claim transaction stamps the matching engine
  identity, and a queued row with an engine-less media type fails closed
  instead of poisoning the queue.
- [x] **CVR-035:** Persist engine/version, warnings, fallback reason, output
  hash, and engine-specific diagnostics without leaking document content.
  AnyDoc attempts persist `classification: structured_document` and
  diagnostics of `{format, processingTimeMs}` only; the family matrix asserts
  both, and the docx round-trip asserts the client filename never reaches the
  manifest.
  **Audit first:** manifests and attempt rows already carry engine name/version,
  route, diagnostics JSON, and output hash. Narrow remaining work to populating
  `warnings` from engine output where applicable; close the ticket if the audit
  passes.
- [x] **CVR-036:** Add licensed or synthetic success and safe-failure fixtures
  for each advertised format family. Done 2026-08-18 for all eight advertised
  formats: pdf plus the AnyDoc seven (doc, docx, ppt, pptx, xls, xlsx, epub),
  each with a round-trip fixture and a bounded-failure fixture, licences in
  `backend/tests/fixtures/SOURCES.md`.

  **Scope decision, 2026-08-18: rtf, odt, ods, odp, and csv are not advertised
  and are not planned.** The rule is *advertise only what the desktop sends*.
  The desktop's convert list (`src-tauri/src/jobs.rs`) is pdf, png, jpg, jpeg,
  webp, tiff, tif, gif, bmp, docx, doc, pptx, ppt, xlsx, xls, html, htm, epub;
  `collect_input_files` drops anything else before upload, so an `.rtf` or
  `.csv` can never become a job. Those five would have cost five admission
  rows, a migration 0003 rebuilding two tables, five fixtures, and five
  contract tests, for zero reachable users. Reopen only when a file the user
  actually owns is rejected.

  **The real format gap is the other direction.** Ten extensions the desktop
  does accept have no local engine: png, jpg, jpeg, webp, tiff, tif, gif, bmp,
  html, and htm. AnyDoc cannot serve any of them. its `Format` enum is exactly
  doc, docx, odt, pdf, ppt, pptx, rtf, epub, excel, ods, odp, csv. They belong
  to the remote route (M5) or the desktop's per-file direct-path fallback
  (M6), never to a local-format increment.
- [x] **CVR-037:** Re-run the container and contract gates with AnyDoc present.
  **Do this first in Increment 4**, before widening formats: graceful and
  SIGKILL restart smokes (`backend/scripts/container-smoke.sh`), full Rust gate,
  and contract tests against the built image with AnyDoc linked. Add an AnyDoc
  hard outer timeout (reuse `TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS` or a dedicated
  env) wrapping `spawn_blocking`; malformed-input and output-ceiling behavior
  must match PDF-engine expectations. Include a test that a simulated AnyDoc
  hang or panic fails the job without wedging the worker loop. Done: the smoke
  passed on 2026-08-18 at run `20260818T192759Z` against image
  `sha256:cd989ed8…` (graceful and SIGKILL); the adapter wraps the parse in
  `tokio::time::timeout` reusing the worker timeout (a timed-out task detaches,
  bounded by AnyDoc's internal limits); hang and panic wrapper tests pass;
  the failure matrix covers malformed input and the output ceiling.
- [x] **CVR-038:** Record the AnyDoc containment policy: in-process for M3;
  hard timeout required (CVR-037); child-worker fallback trigger documented on
  CVR-032. No preemptive second worker binary.
- [ ] **CVR-039:** Skip the redundant full source re-read and SHA-256
  re-computation at parse time when the immutable source path, byte length, and
  hash were validated at claim/submit and the file has not changed. Optional
  M3.5 after Increment 4 gate; measure on typical docx/xlsx before landing.

**M3 gate: met 2026-08-18.** All eight advertised formats have a passing
conversion fixture and a bounded failure fixture. Each source is parsed by
exactly one engine, chosen from the stored media type, and the AnyDoc adapter
refuses PDF bytes outright, so no PDF reaches a second parser. Empty output,
typed engine errors, the output ceiling, and a cross-family mislabel all fail
closed with no artifact published. The container smoke converts a docx inside
the image and asserts `local_anydoc`, so "smokes pass with AnyDoc present"
now means AnyDoc ran, not merely that it linked. AnyDoc conversions honor a
hard outer timeout, and the parser permit is held for the real lifetime of the
parse so a timed-out job cannot run beside the next one.

Known and accepted: AnyDoc's own part-level "skip a broken piece and continue"
recovery is silent, so a partially-degraded document can still succeed. That is
upstream behavior, not a local gate failure. quality signals are M4 (CVR-041).

## M4: Routing and quality policy

**Scope discipline (2026-08-18 review):** M4 gate is a **fixture regression
suite**, not a rate-calibration program. Keep `backend/evals/` as manifest
metadata only until this milestone; do not build harness infrastructure in M3.
`standard` and `local_only` are behaviourally identical until M4 ships. Do
not expose both as distinct user choices before then.

- [ ] **CVR-040:** Populate the labeled corpus with native, scanned, image,
  mixed, dense-table, multi-column, form, encoding-damaged, encrypted,
  malformed, and over-limit PDFs. PDF-only; AnyDoc families are gated in M3.
- [ ] **CVR-041:** Define stable quality signals and fallback reason codes from
  real engine output; avoid filename and page-count heuristics.
- [ ] **CVR-042:** Implement explicit `standard`, `local_only`, and
  `best_quality` routing policy as a pure, table-driven decision boundary.
- [ ] **CVR-043:** Calibrate dense/complex routing against the corpus and record
  false-local-success and unnecessary-remote-routing rates. **Defer rate
  calibration to M8 or until real volume exists.** M4 gate passes on fixture
  pass/fail only; do not block M5 on percentage targets.
- [ ] **CVR-044:** Expose route, reason codes, warnings, and policy decisions in
  status and provenance without exposing document content.
- [ ] **CVR-045:** Add regression tests that prevent scanned, mixed, damaged,
  empty, or incomplete Markdown from silently succeeding.

**M4 gate:** Routing is deterministic, explainable, corpus-backed, and privacy
aware. Complexity metadata alone does not cause external transmission.

## M5: Datalab fallback

**Highest-complexity milestone (2026-08-18 review):** uncertain billable
submission and restart-safe polling are where production bugs live. **Port the
desktop taxonomy from `src-tauri/src/providers.rs` verbatim**: especially
`send_retrying()` (retry only when the server never started work),
`terminal_poll_error()`, and the rule that result-fetch 5xx/429 stays transient
after the work is already billed. Do not redesign Datalab semantics in the
backend. M6 per-file fallback to the direct Datalab path ([`DESKTOP_EXECUTION_PLAN.md`](DESKTOP_EXECUTION_PLAN.md))
may ship before M5; treat backend Datalab as an optimization, not a desktop
blocker.

- [ ] **CVR-050:** Add typed Datalab configuration and load its credential only
  inside the backend.
- [ ] **CVR-051:** Implement a bounded submit/poll/result HTTP adapter with
  stable errors, timeouts, retry classification, and redacted logs. Match
  `providers.rs` retry and transient/terminal classification.
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

## M6: Desktop integration

**Format gap (2026-08-18 review):** the desktop accepts ~18 convert extensions;
the backend advertises three until M3 Increment 4 completes. M6 must not present
backend routing as all-or-nothing. Increment plan and per-file fallback live in
[`DESKTOP_EXECUTION_PLAN.md`](DESKTOP_EXECUTION_PLAN.md). Run
`pnpm generate:api` as Increment 0. The committed schema trails OpenAPI 0.4.0.

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
  **Gate Run on `capabilities.inputFormats`:** disable or fall back to the
  direct path for extensions the backend does not advertise; never fail silently
  at upload for formats the UI allowed.
- [ ] **CVR-066:** Keep the existing direct-provider path behind a reversible
  switch until the deployment gate passes.
- [ ] **CVR-067:** Add end-to-end desktop tests for local success, Datalab
  fallback, restart recovery, backend unavailability, and retry-safe replay.

- [ ] **CVR-081:** Capture direct-path baselines (~10 representative files:
  native PDF, scanned PDF, docx, xlsx, and formats still on Datalab) and
  compare output completeness, routing, and failure behavior against the
  backend path. **Moved from M8**: run before cutover, not after M7.

**M6 gate:** The desktop completes representative local and remote conversions,
recovers active jobs, never exposes backend or provider credentials to the
webview, and CVR-081 baselines are recorded.

## M7: LAN deployment and operations

**v1 LAN cutover subset (2026-08-18 review):** ship with CVR-070 (Caddy),
CVR-071 (per-device tokens), CVR-073 (data layout off SMB/NFS), and CVR-075
(backup/restore proof). **Defer without blocking cutover:** CVR-076 (full SBOM. Run `cargo auditable` once and document licenses instead), CVR-077 (full
runbooks: one README ops section until a second host exists), CVR-078 (subset
of container smoke plus a manual host checklist). Rate-limit `/health/ready`
when Caddy exposes the LAN. The unauthenticated readiness flood remains an
open finding from M2 Increment 5.

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
  readiness, and redacted operational metrics. Default ops surface is JSON logs
  plus Compose healthcheck; optional `GET /metrics` on loopback or internal
  network only. No OTel sidecar or Grafana in this Compose file. See
  [`MONITORING_AND_PROGRESS.md`](MONITORING_AND_PROGRESS.md).
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

## M8: Evaluation, cutover, and cleanup

CVR-081 moved to M6 (baseline capture before cutover). CVR-080 overlaps M4
corpus freeze. Run once when M4 fixtures are stable, not twice.

- [ ] **CVR-080:** Freeze the labeled corpus and expected routing/results for
  release comparison.
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

**Now:** M3 is closed. Next is M6 desktop integration, starting with
`pnpm generate:api` (the committed schema trails OpenAPI 0.4.0). CVR-039
(parse-path optimization) stays optional and unscheduled: measure before
building it.

M6 desktop integration may run in parallel in a second session under the
ownership table in [`HANDOFF.md`](HANDOFF.md). The one coordination point is
`pnpm generate:api` after M3 widens `inputFormats`; run by whichever session
lands second; the M3 session never writes `src/app/api/schema.ts`.

Recommended milestone order after M3: M6 (with per-file Datalab fallback) may
parallel M4; M5 after M4 fixture suite; M7-lite (CVR-070/071/073/075) before
full M7 ops debloat; M8 cutover last.

The M2 plan ([`BACKEND_EXECUTION_PLAN.md`](BACKEND_EXECUTION_PLAN.md)) is
complete. Historical reference only. Active increment detail for M6 is in
[`DESKTOP_EXECUTION_PLAN.md`](DESKTOP_EXECUTION_PLAN.md). See
[`HANDOFF.md`](HANDOFF.md) before touching files.

## Verification log

### 2026-08-17: Foundation and PDF vertical slice

- The independent Rust/Axum backend, direct `pdf-inspector` worker, OpenAPI
  contract, authentication, bounded upload path, atomic artifacts, Dockerfile,
  and loopback Compose definition are present.
- `cargo fmt --check`, `cargo check --locked --offline`,
  `cargo clippy --locked --offline --all-targets -- -D warnings`, and
  `cargo test --locked --offline` passed; the test suite contains 28 tests.
- `docker compose -f backend/compose.yaml config --quiet` passed.
- The image build and running-container smoke were not rerun during the latest
  architecture audit, so they remain explicit M2 exit checks.

### 2026-08-17: Architecture correction

- The short-lived FastAPI/PostgreSQL draft was superseded after confirming
  that both primary Firecrawl engines are Rust-native and the existing service
  already owns the required backend boundary.
- Embedded SQLite plus one bounded worker was selected for the expected
  one-host, low-volume workload.
- GPU, local OCR, Redis, PostgreSQL, a queue product, and speculative
  microservices remain out of scope.

### 2026-08-17: M2 Increment 0 contract freeze

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

### 2026-08-17: Repo hygiene and latest-stable deps

- Planning docs moved to `docs/`. Session state lives in `docs/HANDOFF.md`.
- Added root `LICENSE` (MIT), CI, Dependabot, `rust-toolchain.toml` (1.97.1),
  `.node-version` (24), and `packageManager` / `engines` in `package.json`.
- Catch-up to latest stable except TypeScript 7, `keyring` 4, `libc` 1.0-alpha,
  and the exact `pdf-inspector =1.15.0` pin. Those exceptions are load-bearing;
  do not "finish" them in an M2 session.
- Shared repo files are not backend-owned. Do not revert them in an M2 patch.

### 2026-08-17: M2 Increments 1-3 implementation checkpoint

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

### 2026-08-17: M2 Increment 4 startup-recovery checkpoint

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

### 2026-08-17: M2 Increment 5 runtime and Compose checkpoint

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

### 2026-08-17: M2 Increment 6 failure and release gates

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

### 2026-08-18: M3 Increment 0 AnyDoc spike

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

### 2026-08-18: M3 Increment 1 engine seam

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

### 2026-08-18: M3 Increments 2 and 3: AnyDoc adapter, validation, capabilities

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
  `inputFormats` all read it. The advertised set is PDF, DOCX, and XLSX:
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

### 2026-08-18: Architecture review and epic debloat

Adversarial review of M3–M8 sequencing, performance path, and plan bloat.
Woven into milestone sections above; no code changes.

- **Verdict:** core shape (monolith + SQLite + PDF child + in-process AnyDoc
  crate) is correct. Parse stays fast; orchestration adds latency, not parser
  slowdown.
- **M3 Increment 4 reordered:** CVR-037 container smoke first; CVR-036 batch
  all spike-proven AnyDoc families; added CVR-038 (containment policy) and
  CVR-039 (optional parse-path optimization).
- **Debloat:** M4 rate calibration deferred; M7 v1 subset identified;
  CVR-081 moved to M6; M5 must port `providers.rs` retry taxonomy.
- **Clarified:** parser concurrency is per engine, not global.

### 2026-08-18: M3 Increment 4 fixture matrix, timeout, and gates

Closes CVR-035, CVR-037, and CVR-038. CVR-036 is complete for the
desktop-intersection seven; rtf, odt, ods, odp, and csv remain as the batch
noted on the ticket.

- The admission table now covers the desktop seven (doc, docx, ppt, pptx,
  xls, xlsx, epub). Every family has a vendored MIT success fixture from the
  upstream AnyDoc corpus plus a bounded-failure fixture, recorded in
  `backend/tests/fixtures/SOURCES.md`; the two truncated xls/epub fixtures
  are derived locally from the vendored files. The contract suite proves all
  seven round-trips and seven fail-closed cases.
- The adapter wraps `spawn_blocking` in `tokio::time::timeout` reusing the
  worker timeout (CVR-037): a hang fails the job `worker_timeout` and the
  worker loop never waits on the parse again; the detached task stays bounded
  by AnyDoc's internal limits. Wrapper tests cover hang, panic, and the
  engine serving the next conversion immediately.
- Detection failure after admission maps to `invalid_document`, not
  `unsupported_document`. A corrupt known container is not an unknown
  format.
- The CSV extension hint exists in the adapter (media type `text/csv` names
  the format when detection returns nothing) with an engine-level test; the
  admission row lands with the remaining CVR-036 batch.
- Gates: format, check, strict Clippy deny-set, `git diff --check`,
  `docker compose config`, and 132 backend tests (67 library, 1 server, 3
  parser worker, 41 HTTP contract, 5 crash recovery, 10 integrity matrix, 5
  failure modes). Container smoke run `20260818T192759Z` passed graceful and
  SIGKILL phases against image `sha256:cd989ed8…` with AnyDoc linked in.

### 2026-08-18: M3 close-out. adversarial review, restart blocker, gate

Closes CVR-036 at the advertised eight and meets the M3 gate. A review aimed at
cutting scope found a shipping blocker instead.

- **Blocker: a succeeded AnyDoc job stopped the service from booting.**
  `jobs/recovery.rs` validated stored classifications against a hand-copied
  list of the four PDF values (`text_based`, `scanned`, `image_based`,
  `mixed`). AnyDoc persists `structured_document`, which migration 0002 added
  to the database CHECK but nobody added here. Startup reconciliation
  revalidates every stored success, so the first non-PDF conversion made the
  next restart fail with `PersistedMetadataInvariant` and exit 1. a crash loop
  whose only manual escape is deleting the database. Fixed by moving the
  vocabulary to one place: `DocumentClassification::from_stored` now sits
  beside `as_str` in `persistence/model.rs` and recovery calls it.
- **Why four increments and 132 green tests missed it: every restart test used
  a PDF.** `completed_job_idempotency_and_downloads_survive_app_restart`
  submits `clean_pdf()`, and the container smoke synthesized a PDF. The one
  input class that could trigger the bug was never restarted. The regression
  test `a_succeeded_anydoc_job_survives_restart_without_wedging_startup`
  reproduces the exact production error with the fix stashed.
- **The container smoke now converts a docx** and asserts `local_anydoc`,
  `anydoc` `0.1.9`, the source hash, a non-empty artifact, and no filename
  leak. This is what surfaced the blocker: the previous smoke proved only that
  AnyDoc was linked into the image.
- **xlsx had no bounded-failure fixture**, the one real hole in the gate's
  fixture matrix. Added `truncated.xlsx`, derived like the xls and epub ones.
- **A cross-family mislabel used to report corruption.** docx, xlsx, pptx and
  epub share one ZIP magic, so admission cannot separate them. An xlsx sent as
  `.docx` converted fine and then failed the manifest check as
  `artifact_integrity_failed`. The adapter now compares the detected family
  against the admitted one before parsing and rejects it as
  `invalid_document`.
- **The parser permit was released too early.** On timeout the blocking task
  detaches and keeps parsing, but the permit dropped when `convert` returned,
  so the next job could parse alongside it. The permit now lives inside the
  blocking closure, restoring one parse per engine.
- **Cut:** the CSV media-type hint and its test. `text/csv` had no admission
  row, so uploads were rejected before the engine saw them. it could not fire.
  The manifest route match no longer wildcards to `local_pdf`, so a third
  engine becomes a compile error rather than a mislabelled job.
- Gate: `cargo fmt --check`, `cargo check`, strict Clippy deny-set,
  `git diff --check`, `docker compose config`, and 133 backend tests (66
  library, 1 server, 3 parser worker, 43 HTTP contract, 5 crash recovery, 10
  integrity matrix, 5 failure modes). Container smoke run `20260818T195919Z`
  passed graceful and SIGKILL against image `sha256:7b765043…`, with the
  AnyDoc assertions in the evidence file.
