> **Superseded by [`../STATUS.md`](../STATUS.md) as of 2026-08-19.** Historical copy only.

# Epic: Rust conversion backend

**Status:** M0-M3 closed. M6 implementation landed, gate open. M4, M5, M7, M8
unbuilt.
**Current work:** Sprint 0 repairs are landed. Sprint 1 (M4) is next.
**Target:** CPU-only Rust/Axum modular monolith
**Architecture:** [`BACKEND_SERVICE_PLAN.md`](BACKEND_SERVICE_PLAN.md)
**Immediate plan:** [`CLOSEOUT_EXECUTION_PLAN.md`](CLOSEOUT_EXECUTION_PLAN.md)
**Session handoff:** [`HANDOFF.md`](HANDOFF.md)
**Last updated:** 2026-08-19

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
- One worker loop, strictly sequential: it awaits each claimed job before
  claiming the next, pinned by
  `single_runner_keeps_waiting_jobs_queued_and_execution_serial`. Each engine
  also holds its own permit, which matters only because a timed-out AnyDoc
  parse detaches still holding one. Two jobs never parse concurrently.
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
| M4 | Corpus-calibrated routing and quality policy | Complete (CVR-043 open by design) | M3 |
| M5 | Restart-safe Datalab fallback and privacy policy | Planned | M4 |
| M6 | Desktop app uses the backend | In progress | M2 (M5 for one CVR-067 scenario) |
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
- [ ] **CVR-035:** Persist engine/version, warnings, fallback reason, output
  hash, and engine-specific diagnostics without leaking document content.
  AnyDoc attempts persist `classification: structured_document` and
  diagnostics of `{format, processingTimeMs}` only; the family matrix asserts
  both, and the docx round-trip asserts the client filename never reaches the
  manifest.
  **Reopened 2026-08-18:** everything except `warnings` is genuinely stored.
  `warnings` is hardcoded empty at both write sites (`finalize_success` and
  `local_analysis`), so nothing an engine reports as a caveat reaches the user.
  Populating it is S1.2 in
  [`CLOSEOUT_EXECUTION_PLAN.md`](CLOSEOUT_EXECUTION_PLAN.md).
- [x] **CVR-036:** Add licensed or synthetic success and safe-failure fixtures
  for each advertised format family. Done 2026-08-18 across the full AnyDoc
  format set: PDF on its own engine plus 18 AnyDoc extensions, each with a
  round-trip fixture and each family with a bounded-failure fixture. Licences
  and derivations in `backend/tests/fixtures/SOURCES.md`.

  **Scope correction, 2026-08-18 (supersedes the narrowing earlier that day).**
  The first pass closed this ticket by refusing rtf, odt, ods, odp, and csv on
  the grounds that the desktop cannot send them. That was the wrong test. The
  backend is a document-conversion service and AnyDoc is its engine, so the
  question is what the engine can do, not what today's UI happens to offer.
  The narrowing also missed extension variants entirely.

  **The advertised set is now every AnyDoc format a fixture proves: 19
  extensions over 18 media types**, plus PDF on its own engine.

  | Family | Extensions | AnyDoc format |
  |---|---|---|
  | Word | `doc`, `docx`, `docm` | Doc, Docx |
  | PowerPoint | `ppt`, `pps`, `pot`, `pptx`, `pptm`, `ppsx`, `ppsm` | Ppt, Pptx |
  | Excel | `xls`, `xlsx`, `xlsm` | Excel |
  | OpenDocument | `odt`, `ods`, `odp` | Odt, Ods, Odp |
  | Other | `epub`, `rtf`, `csv` | Epub, Rtf, Csv |

  `pps` and `pot` share `application/vnd.ms-powerpoint` with `ppt`, so the
  media-type list is 18 while the extension list is 19. Migration 0003 widens
  the CHECK once for all of them; the per-format cost after that is one
  admission row, one fixture, and one test case.

  **`xlsb` is the one deliberate omission.** AnyDoc maps it to the Excel parser
  and `calamine` has binary-workbook support compiled in, but no upstream
  fixture exists and a binary workbook cannot be honestly derived from an XML
  one. Advertising it would break the rule that earns this table its trust: a
  format appears only with a fixture that proves it.

  **Still absent, and not AnyDoc's to solve:** the ten desktop extensions with
  no local engine. png, jpg, jpeg, webp, tiff, tif, gif, bmp, html, htm.
  AnyDoc's `Format` enum has no variant for any of them, so they belong to the
  remote route (M5) or the desktop's per-file direct-path fallback (M6).

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

**M3 gate: met 2026-08-18.** All 19 advertised extensions have a passing
conversion fixture, and every format family has a bounded failure fixture. Each source is parsed by
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

**Scope discipline:** the M4 gate is a **fixture regression suite**, not a
rate-calibration program.

**Why this milestone is not optional.** Measured against the real worker
binary: pdf-inspector's `confidence` on a text-based PDF is exactly
`text_pages / total_pages`, and it returns 1.0 for every all-native document at
1, 2, 4, and 10 pages. A document with nine native pages and one scanned page
returns 0.9, converts, and before M4 published as a plain success with that
page's content absent from the Markdown. `warnings` was hardcoded empty at
every write site, so nothing said otherwise. `isComplex`, `pagesWithTables`,
and `pagesWithColumns` reached the manifest and were read by nothing.

`standard` and `local_only` stop being behaviourally identical here: a
partly-scanned document routes remote under `standard` and publishes with
`pages_missing_native_text` under `local_only`.

- [x] **CVR-040:** Populate the labeled corpus with native, scanned, image,
  mixed, dense-table, multi-column, form, encoding-damaged, encrypted,
  malformed, and over-limit PDFs. PDF-only; AnyDoc families are gated in M3.
  `tests/support/corpus.rs` generates every case in process, so no binary PDF
  enters the repository, and `evals/corpus-manifest.yaml` records the expected
  status, route, reason codes, warnings, and artifact flag for 17 of them. Two
  gaps stay open: `form_pdf` produces an inspection identical to the same page
  with no AcroForm, and three of the seven engine fallback reasons have no
  case, including `garbled_text`, which is unreachable because
  `hasEncodingIssues` never sets.
- [x] **CVR-041:** Define stable quality signals and fallback reason codes from
  real engine output; avoid filename and page-count heuristics.
  `engines::QualitySignals` is engine-neutral and carries only measurements:
  `native_text_ratio` (`None` when the engine cannot measure it),
  `pages_needing_ocr`, `has_tables`, `has_columns`. The PDF engine fills it
  from `Inspection`; AnyDoc reports `unmeasured()` because it genuinely cannot
  measure completeness. No filename or page-count heuristic exists.
- [x] **CVR-042:** Implement explicit `standard`, `local_only`, and
  `best_quality` routing policy as a pure, table-driven decision boundary.
  `conversion::policy::decide(profile, route, LocalResult) -> PolicyDecision`
  is pure: no IO, no clock, no engine type. `best_quality` stays rejected at
  the API until M5 gives it a remote leg to mean.
- [ ] **CVR-043:** Calibrate dense/complex routing against the corpus and record
  false-local-success and unnecessary-remote-routing rates. **Defer rate
  calibration to M8 or until real volume exists.** M4 gate passes on fixture
  pass/fail only; do not block M5 on percentage targets.
- [x] **CVR-044:** Expose route, reason codes, warnings, and policy decisions in
  status and provenance without exposing document content. The policy's reason
  codes and warnings now flow into the attempt row, the job status response,
  and the manifest. The desktop renders them verbatim through
  `jobDetailItems`, so a new code needs no frontend release.
- [x] **CVR-045:** Add regression tests that prevent scanned, mixed, damaged,
  empty, or incomplete Markdown from silently succeeding.
  `tests/routing_policy.rs` drives every corpus case through the real HTTP
  surface and the real worker, asserting status, route, reason codes,
  warnings, failure code, artifact listing, downloaded bytes, and manifest.
  **Silently is the operative word.** A partly-textless document publishes with
  `pages_without_extractable_text` rather than routing remote, because the only
  signal available fires just as readily on a report with a sparse cover page
  that lost nothing. `a_sparse_cover_page_is_not_missing_content` pins that.

**M4 gate: met 2026-08-19**, with one ticket deliberately still open.

Routing is deterministic, explainable, corpus-backed, and privacy aware. The
policy is a pure function; complexity metadata alone causes no external
transmission, and in fact no local signal does: the engine's own give-up is the
only thing that produces `needs_remote`.

CVR-043 stays open by its own terms, and the M4 work sharpened why. Calibrating
dense or complex routing needs a signal that distinguishes "this page produced
no text because it is blank" from "because it is a scan". `confidence` does
not: a ten-page report with a one-line cover page and no images anywhere
reports 0.9, exactly like a document with a scanned page. Until the worker
reports pages that reference an image XObject and yielded no text, calibration
would be tuning a coin flip.

## M5: Datalab fallback

**Highest-complexity milestone (2026-08-18 review):** uncertain billable
submission and restart-safe polling are where production bugs live. **Port the
desktop taxonomy from `src-tauri/src/providers.rs` verbatim**: especially
`send_retrying()` (retry only when the server never started work),
`terminal_poll_error()`, and the rule that result-fetch 5xx/429 stays transient
after the work is already billed. Do not redesign Datalab semantics in the
backend. M6 per-file fallback to the direct Datalab path ([`DESKTOP_EXECUTION_PLAN.md`](archive/DESKTOP_EXECUTION_PLAN.md))
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

**Implementation checkpoint (2026-08-18):** the desktop now routes per file
from the live `capabilities.inputFormats` contract; it never hardcodes the
backend's proven PDF/AnyDoc set. Image and HTML formats remain permanently on
the direct Datalab path, unsupported direct-eligible formats fall back only
under `standard`, and `local_only` never sends them to Datalab. The generated
schema matches OpenAPI 0.4.0. Increment detail and the remaining validation
work live in [`DESKTOP_EXECUTION_PLAN.md`](archive/DESKTOP_EXECUTION_PLAN.md).

- [x] **CVR-060:** Add backend URL and device-token settings outside React
  state, keeping the token in the macOS Keychain.
- [x] **CVR-061:** Generate or validate and commit the Tauri HTTP client/schema
  against the backend OpenAPI contract, including the capability correction
  temporarily diffed during M2.
- [x] **CVR-062:** Stream selected files from Tauri to the backend with an
  idempotency key and stable client run ID.
- [x] **CVR-063:** Poll durable job state and recover an in-progress desktop run
  after app restart.
- [x] **CVR-064:** Download Markdown through Tauri and preserve current
  collision-safe output naming.
- [x] **CVR-065:** Show selected route, warnings, privacy decisions, and
  actionable failures without exposing provider credentials to the webview.
  **Gate Run on `capabilities.inputFormats`:** disable or fall back to the
  direct path for extensions the backend does not advertise; never fail silently
  at upload for formats the UI allowed.
- [x] **CVR-066:** Keep the existing direct-provider path behind a reversible
  switch until the deployment gate passes.
- [ ] **CVR-067:** Add end-to-end desktop tests for local success, Datalab
  fallback, restart recovery, backend unavailability, and retry-safe replay.
  Deterministic native and loopback coverage now proves local success,
  restart/replay, unknown-status pending behavior, backend unavailability,
  `needs_remote` profile decisions, stop/retry cleanup, response identity, and
  path-only artifact IPC. The explicit M5-dependent end-to-end durability
  scenario for backend-owned Datalab fallback remains pending.

- [ ] **CVR-081:** Capture direct-path baselines (~10 representative files:
  native PDF, scanned PDF, docx, xlsx, and formats still on Datalab) and
  compare output completeness, routing, and failure behavior against the
  backend path. **Moved from M8**: run before cutover, not after M7.

**M6 gate: not met yet.** The implementation completes local conversions,
recovers active jobs, keeps credentials and bytes native, and preserves the
direct fallback. The M5-dependent CVR-067 fallback-durability scenario and the
CVR-081 representative direct-path baselines remain open.

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
  [`MONITORING_AND_PROGRESS.md`](archive/MONITORING_AND_PROGRESS.md).
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

Sprints, gates, and the confirmed defect list are in
[`CLOSEOUT_EXECUTION_PLAN.md`](CLOSEOUT_EXECUTION_PLAN.md). Order: Sprint 0
repairs (done), M4 policy, M5 fallback, close M6, M7-lite, M8 cutover.

CVR-039 (parse-path optimization) stays optional and unscheduled: measure
before building it. Read [`HANDOFF.md`](HANDOFF.md) before touching files.

## Verification log

Per-increment evidence through M6 lives in
[`archive/BACKEND_VERIFICATION_LOG.md`](archive/BACKEND_VERIFICATION_LOG.md).
New entries append there.

**2026-08-19, Sprint 0 repairs.** An adversarial review of M1-M6 confirmed
ten defects; all are fixed. A second review then showed two of those repairs
had closed the reported instance and left the rest of the class open, and both
are now closed as well: the Datalab ledger write is required and the in-memory
fallback context is written before the request, so a same-process Retry can no
longer resubmit; and recovery rows decode independently, so a row that will not
parse is quarantined instead of aborting the boot. Contract moved to 0.4.1.

**2026-08-19, Sprint 1 (M4).** CVR-040, 041, 042, 044, and 045 are closed;
CVR-043 stays open by its own terms. The routing policy is a pure function, the
corpus generates in process, and the eval manifest is pinned to the strings the
service can actually emit.

The correction worth remembering: the first cut read pdf-inspector's
`confidence` as a completeness measure and routed remote on it. Three
independent skeptics rejected that and the worker confirmed them. A ten-page
report with a one-line cover page and **no images anywhere** reports the same
0.9 as a document with a scanned page, and its Markdown contains every word, so
routing on that signal would have billed Datalab for ordinary documents. The
signal now warns and never routes.

Gate: 160 backend tests, 63 desktop Rust tests, 80 frontend tests, both Clippy
gates, the production build, and container smoke run `20260819T031914Z` against
image `sha256:4a0cbdd0…` — graceful and SIGKILL, migrations rebuilding fresh in
the image, AnyDoc converting a docx in-container, and artifacts byte-identical
across a restart. Contract at 0.4.2.
