# Backend verification log (archived)

Per-increment evidence for M0 through M6, moved out of
[`BACKEND_EPIC.md`](../BACKEND_EPIC.md) on 2026-08-18 so the tracker stays
readable. Historical record. Nothing here is a live instruction.

### 2026-08-18: M6 desktop backend vertical slice

CVR-060 through CVR-066 are implemented. CVR-067 and CVR-081 remain open for
the explicit evidence named on their tickets.

- `b893dd3` regenerated the committed OpenAPI 0.4.0 schema and recorded the
  multipart/download command decisions. `af93c18` and `03c0ce4` added the
  durable per-file in-flight ledger and bound every recovery row to its
  original backend URL.
- `bed0ab2` added the reversible direct/backend setting, backend URL and
  profile, plus the Keychain-backed backend token surface. `1533485` registered
  the native service door, streamed multipart from the desktop path, bounded
  generic responses, and streamed Markdown directly to collision-safe disk.
- `24f8231` added capability-driven per-file planning. Permanent image/HTML
  formats stay direct even if advertised; arbitrary files never enter the
  conversion path; unavailable service capacity blocks only files that need
  it; `local_only` never falls through to Datalab.
- `6f19ee8` added durable submit/replay, five-second polling, explicit terminal
  allowlisting, restart recovery, path-only downloads, `needs_remote` profile
  decisions, and distinct backend history output. `eed27c5` and `1f69cab`
  carried route, reason codes, warnings, and failures into the enabled UI.
- `0039e47`, `c9f917c`, and `94554de` closed Stop/new-run races by serializing
  generation retirement, guarded mutation, terminal bookkeeping, and event
  emission. `3f91168` rejects invalid or redirected response IDs before an
  artifact request.
- `489b24f` added an ignored, environment-gated live compatibility smoke. It
  passed once against an ephemeral 0.4.0 service and the checked-in AnyDoc DOCX
  fixture through the actual native capabilities, submit, poll, and streamed
  download helpers.
- The final shared-tree landing gate for `1f69cab` passed 53 frontend tests,
  the production build, Clippy with warnings denied, and 60 desktop Rust tests;
  3 credentialed live smokes stayed ignored. That Rust count includes 3 tests
  from concurrent, unstaged `secrets.rs` work and is not an exact-commit count.
  The atomic-retirement landing gate similarly passed 52 frontend tests and
  the shared-tree 60-test Rust suite.
- Conservative caveat: when the backend route is selected, conversion history
  reuse/copy is disabled because direct-provider and backend results are not
  yet safely interchangeable in the reuse matcher.

Still pending: CVR-081's representative direct-path baseline and the
M5-dependent end-to-end durability scenario for backend-owned Datalab fallback.

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
  integrity matrix, 5 failure modes). Container smoke run `20260818T202130Z`
  passed graceful and SIGKILL, advertising all 18 media types. (The earlier
  `20260818T195919Z` run predates the full format set and advertised 8.)

### 2026-08-18: full AnyDoc format set

Supersedes the CVR-036 narrowing recorded earlier the same day. That decision
advertised only the formats the desktop sends and called rtf, odt, ods, odp and
csv "not planned". Wrong test: this is a document-conversion service, so the
bound is what the engine can do, not what today's UI offers. The narrowing also
never looked at extension variants.

- **Advertised set goes from 8 extensions to 19**, over 18 media types. New:
  `odt`, `ods`, `odp`, `rtf`, `csv`, and the variants `docm`, `xlsm`, `pptm`,
  `ppsx`, `ppsm`, `pps`, `pot`.
- **Every one was probed against the real engine before being advertised**, not
  assumed. All 12 convert. odt 1322 bytes, ods 396, odp 272, rtf 1321, csv 294,
  and each variant matching its base parser.
- Bounded failures are upstream where they exist: `encrypted--errors.odt` gives
  `encrypted_document`, `hugerepeat--errors.ods` gives `document_exceeds_limits`
  through the `max_expansion` budget. Derived where they do not: a truncated
  odp fails detection, and a bare `{\rtf1` or a blank csv produces empty output.
  All fail closed with no artifact.
- **Detection needed no changes**, but one subtlety is worth recording: `.docm`
  and `.pptm` main-part content types contain `ms-word` / `ms-powerpoint`, not
  `wordprocessingml` / `presentationml`, so `opc_format` returns `None` and the
  mandated root element identifies them (`detect.rs:186`). The derived macro
  fixtures are real OPC packages so they exercise that path rather than the
  content-type shortcut.
- `ContainerMagic` gains `Rtf` (`{\rtf`) and `None`. CSV carries no signature at
  all, so its admission is the extension plus the declared media type, and the
  adapter names the format from the admitted label because detection returns
  `None`.
- Migration 0003 widens the media-type CHECK once for all ten new media types,
  reusing the 0002 rebuild. The populated-database upgrade test now drives 0002
  and 0003 together and asserts an OpenDocument row inserts.
- **New guard:** `advertised_media_types_match_the_migration_check` pins the
  admission table to the migration CHECK in both directions. Drift there means
  an upload the API accepts dies at INSERT on a constraint error instead of
  returning a clean 415, and nothing caught it before.
- **`xlsb` is the one deliberate omission**: mapped by AnyDoc and supported by
  calamine, but no fixture exists and a binary workbook cannot be honestly
  derived from an XML one.
- Contract is 0.4.0 still; `inputFormats` grows from 8 to 18 entries, which is
  additive. `pnpm generate:api` at M6 picks it up.
