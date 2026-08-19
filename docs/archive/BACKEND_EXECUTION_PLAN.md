# M2 execution plan: durable SQLite jobs and artifacts

**Status:** Complete
**Epic tickets:** CVR-020 through CVR-029
**Latest verified checkpoint:** `01f1bf1`. Increments 0-6 complete and
committed on `main`; M2 is closed.
**Complexity:** Medium-high
**Estimated implementation shape:** One baseline increment plus six bounded
implementation increments, each independently testable
**Last updated:** 2026-08-18
**Session handoff:** [`STATUS.md`](../STATUS.md)

## Objective

Make the existing authenticated PDF conversion slice survive process and
container restarts without changing its engine behavior or expanding supported
formats.

At the end of M2:

- a successfully accepted upload has an immutable source file and committed
  SQLite job record;
- idempotency survives restart;
- one durable worker claims queued work and resumes after restart;
- every execution has an attempt-scoped staging/publication directory;
- `succeeded` always means the Markdown and manifest are present and
  hash-verified; and
- Docker Compose persists `/data` without adding PostgreSQL, Redis, or another
  service.

## Scope

Included:

- embedded SQLite configuration, migrations, repository, and readiness;
- persistent job source and attempt artifact layout under `/data`;
- durable idempotency and current job status;
- one bounded claim/execute loop;
- startup reconciliation and graceful shutdown;
- restart, crash-window, corruption, and disk-write tests; and
- Compose persistence plus a real stop/start smoke test.

Excluded from this unit:

- AnyDoc and all non-PDF input;
- Datalab or any outbound document transfer;
- routing-policy changes;
- cancellation, manual retry, purge, retention policy, or batch endpoints;
- Caddy, LAN exposure, per-device credentials, backup automation, or desktop
  integration;
- multiple converter replicas or distributed locks; and
- GPU, OCR, transcription, or summarization work.

## Live progress snapshot

| Increment | Status | Notes |
|---|---|---|
| 0: Contract freeze | Complete | Baseline and generated-client parity recorded |
| 1: Persistence foundation | Complete | SQLite repository, migration, transactions, recovery queries |
| 2: Durable ingest/artifacts | Complete | Persistent source/artifact ownership and restart-visible API reads |
| 3: Single durable worker | Complete | One FIFO runner; adversarial lifecycle and integrity fixes verified |
| 4: Startup reconciliation | Complete | State-specific recovery and bounded bundle validation verified |
| 5: Runtime and Compose | Complete | Live readiness, persistent capabilities/OpenAPI, `/data` volume |
| 6: Adversarial/release gates | Complete | Fault barriers, integrity matrices, and graceful/forced restart smokes on the built image |

The settled Increment 5 checkpoint passed format, check, Clippy with warnings
denied, `git diff --check`, `docker compose config`, `docker build`, and 96
backend tests (56 library, 1 server, 3 parser worker, 36 HTTP contract). That
image build is the first to succeed since `e8e8ca5` added
`sqlx::migrate!("./migrations")`, because the builder stage copied only
`Cargo.toml`, `Cargo.lock`, and `src`, and the macro reads the SQL at compile
time. The checkpoint is `2f3158d`, merged as `b70ce44`.

Increment 6 then closed M2. It added a deterministic fault barrier at four crash
windows, a ten-case source and artifact integrity matrix, the missing failure
modes, and `backend/scripts/container-smoke.sh`, which runs the graceful and
forced-kill restart gates against the built image. Backend tests went from 96 to
119 and both smokes pass.

**One box in Increment 3 had been ticked without the code.** "Add a test-only
deterministic fault barrier after transactional claim" was marked done, and no
barrier existed anywhere in the crate. The consequence was not cosmetic: every
recovery test injected a stored state directly, so nothing proved the service
writes `finalizing` before it renames published files, or renames before it
commits `succeeded`. That ordering is the reason the `finalizing` state exists.
`src/faults.rs` and `tests/crash_recovery.rs` now prove it.

## Requirements and acceptance rules

1. Preserve the current API paths, authentication, PDF engine behavior, and
   status values. Authorize one explicit capabilities-contract correction:
   report persistent durability and active capacity instead of
   `durability: ephemeral` and `maxEphemeralJobs`. Update the backend OpenAPI in
   M2, but generate and diff the TypeScript schema only in a temporary location;
   do not write `src/app/api/schema.ts` until M6.
2. Return `202 Accepted` only after the source and database job record are
   durable enough to recover on restart.
3. Use SQLite as the job ledger; an in-memory notification may wake the worker
   but cannot be the source of truth.
4. Allow only one application worker. The production shape remains one
   converter instance.
5. Use backend-generated relative paths beneath `/data`; reject path escape and
   symlink substitution.
6. Keep source, staging, and publication on the same host-local filesystem.
7. Preserve every execution under a distinct attempt ID. Recovery never
   overwrites prior attempt output.
8. Never commit `succeeded` before atomic publication and validation complete.
9. Make migrations forward-only, embedded, and automatically applied before
   readiness or worker startup.
10. Keep failure messages stable and safe for the client; retain detailed but
    content-free diagnostics in logs.

## Implementation decisions

### SQLite access

Use SQLx 0.9 with default features disabled and only `runtime-tokio`, `sqlite`,
`migrate`, and `macros`. It fits the existing asynchronous Axum/Tokio process
and avoids a blocking database thread or ORM. Use runtime queries for ordinary
repository operations so builds do not depend on a development database URL;
bind UUIDs and timestamps as validated strings rather than enabling additional
SQLx features. Add `backend/build.rs` with migration-directory change tracking
so embedded migrations are rebuilt when their SQL changes.

Connection behavior:

- configurable data directory, with the database filename fixed as
  `converter.sqlite` (`/data/converter.sqlite` in the container);
- WAL journal mode;
- full synchronous durability for the low-volume accepted-job write path;
- foreign keys enabled on every connection;
- bounded busy timeout;
- a small fixed pool suitable for API reads plus one writer;
- migrations run before the listener binds; and
- a write/read probe and `/data` writability check back readiness.

The live database must be on the Docker host's local dataset, not SMB/NFS.

### Minimal schema

Keep the schema application-specific; do not introduce a generic jobs product
or event-sourcing layer.

`conversions`

- server job ID and `client_run_id`;
- authentication scope, fixed to the literal `bootstrap` in M2 so token
  rotation does not change the idempotency namespace;
- hashed idempotency key and request fingerprint with a unique constraint;
- profile and current public status;
- immutable source relative path, media type, byte count, and SHA-256;
- active attempt ID;
- current route, reason codes, warnings, and stable failure fields; and
- created/updated timestamps.

`attempts`

- attempt ID, conversion ID, and monotonic attempt number;
- attempt state and recovery count;
- engine, engine version, route, classification, and fallback reason;
- start/finish timestamps and bounded diagnostics; and
- a uniqueness constraint on conversion plus attempt number.

`artifacts`

- conversion ID, attempt ID, and artifact kind;
- generated relative path and media type;
- byte count and SHA-256; and
- a uniqueness constraint on attempt plus kind.

Arrays and bounded engine metadata can begin as validated JSON text. Avoid a
separate idempotency table unless the migration implementation demonstrates a
clear correctness advantage; the uniqueness boundary belongs with conversion
creation.

### Filesystem layout

```text
/data/
  converter.sqlite
  jobs/
    {job-id}/
      source/input
      attempts/
        {attempt-id}/
          publication.staging/
          artifacts/result.md
          artifacts/manifest.json
  quarantine/
    pre-acceptance/
      {job-id}/...
```

The source is immutable and shared by attempts. Results are attempt-scoped.
All database paths are relative to `/data` so the dataset is relocatable.

### State and recovery

M2 keeps the current public state enum:

```text
queued -> converting_local -> finalizing -> succeeded
                         |                 `-> failed
                         `-> needs_remote
```

Recovery runs after migrations and before readiness/worker startup:

| Stored state | Recovery action |
|---|---|
| `queued` | Leave eligible for normal claim |
| `converting_local` | Close the interrupted attempt and create/requeue a new attempt within the recovery limit |
| `finalizing` + valid published files | Verify paths, sizes, hashes, and manifest; commit success |
| `finalizing` + only staging/invalid files | Remove only that attempt's staging and requeue within policy, otherwise fail stably |
| `succeeded` | Verify artifacts before serving; if missing or corrupt, expose `failed` with `artifact_integrity_failed`, retain audit metadata, and serve no artifacts |
| `failed` / `needs_remote` | Leave terminal |

The worker transactionally changes one eligible row from `queued` to
`converting_local`, commits, and then executes it. A Tokio `Notify` supplies
low-latency wake-up; a short periodic poll covers missed signals and restart.
No API request spawns a conversion task.

## Work sequence

### Increment 0: Freeze the M1 contract

Related: CVR-028; CVR-019 remains the completed M1 coverage ticket.

- [x] Record the current backend OpenAPI checksum and a temporary generated
  TypeScript-schema diff baseline without writing desktop source files.
- [x] Run format, check, tests, Clippy, and Compose validation before changing
  persistence.
- [x] Add characterization tests for current submission replay/conflict,
  state/status serialization, artifact lookup, and `needs_remote` behavior.
- [x] Add test helpers that can restart an `AppState` against the same temporary
  data directory without starting a real network listener.

Exit: current behavior is protected by tests, and any intentional contract
change will be visible.

### Increment 1: Persistence foundation

Related: CVR-020, CVR-021, CVR-022.

Implementation note (2026-08-17): this increment is complete. Re-read the live
files and [`STATUS.md`](../STATUS.md) before changing its invariants.

- [x] Add SQLx 0.9 with default features disabled and exactly the
  `runtime-tokio`, `sqlite`, `migrate`, and `macros` features; regenerate only
  `backend/Cargo.lock`.
- [x] Add `TOOLKIT_CONVERTER_DATA_DIR`, polling interval, and local
  recovery-limit settings with bounded validation. Always resolve the database
  as `converter.sqlite` beneath the configured data directory.
- [x] Add `backend/build.rs` to track changes under `backend/migrations/` for
  embedded migration rebuilds.
- [x] Add `backend/migrations/0001_conversion_jobs.sql` and migration tests.
- [x] Add `persistence/` with a small repository interface and SQLite
  implementation.
- [x] Implement create-or-replay, lookup, capacity check, state transition,
  claim-next, attempt creation, artifact commit, and recovery queries.
- [x] Keep timestamps and public views compatible with the current API.
- [x] Prove unique idempotency and fingerprint-conflict behavior under
  concurrent requests.

Likely files:

```text
backend/Cargo.toml
backend/Cargo.lock
backend/build.rs
backend/migrations/0001_conversion_jobs.sql
backend/src/config.rs
backend/src/app.rs
backend/src/persistence/mod.rs
backend/src/persistence/sqlite.rs
backend/src/conversion/model.rs
backend/src/conversion/registry.rs   # removed or reduced to repository types
```

Exit: the repository passes migration, CRUD, transition, idempotency, and
claim tests against a file-backed temporary SQLite database.

### Increment 2: Durable ingest and artifacts

Related: CVR-021, CVR-023, CVR-025.

- [x] Replace `TempDir` ownership with a configured persistent `ArtifactStore`.
- [x] Stream incoming PDFs to a generated source staging path under the job
  directory, retaining the current size/time/signature/hash checks.
- [x] Flush/sync and atomically place the immutable source before committing
  the accepted database record; sync its parent directory where supported.
- [x] Insert conversion, first attempt, and idempotency decision in one
  transaction.
- [x] Immediately remove the newly staged job/source tree on idempotency
  replay, fingerprint conflict, active-capacity rejection, or database
  transaction failure. A replay must still succeed when active capacity is
  full.
- [x] Add an artifact-store operation that moves a pre-acceptance job tree under
  `/data/quarantine/pre-acceptance/{job-id}/` without following symlinks. The
  startup reconciler invokes it in Increment 4; M2 never auto-deletes the
  quarantined document data.
- [x] Make staging and publication attempt-scoped and preserve the current
  symlink, size, hash, and non-empty-output checks.
- [x] Sync validated artifacts before publication rename so a committed
  `succeeded` state does not depend only on buffered writes.
- [x] Persist only generated relative paths and resolve them through one
  containment-checked path helper.
- [x] Test that replay, conflict, capacity rejection, and transaction failure
  leave no unowned document bytes or job directory.

Likely files:

```text
backend/src/artifacts.rs
backend/src/api/conversions.rs
backend/src/conversion/service.rs
backend/src/conversion/model.rs
backend/src/persistence/sqlite.rs
```

Exit: a returned `202` can be recovered from a new process using only the same
database and `/data` tree; a failed submission leaves no accepted job.

### Increment 3: Single durable worker

Related: CVR-024, CVR-025.

- [x] Add one `JobRunner` started during application bootstrap.
- [x] Replace `ConversionService::submit`'s per-job `tokio::spawn` with a
  repository commit followed by `Notify`.
- [x] Claim one queued job transactionally and preserve parser concurrency one.
- [x] Add a test-only deterministic fault barrier after transactional claim and
  before engine execution so claim-recovery tests do not depend on timing.
- [x] Before every initial or recovered execution, require the immutable source
  to be contained under the job root, a regular non-symlink file, and an exact
  match for its stored byte count and SHA-256.
- [x] Persist state before and after engine execution, finalization, and
  terminal outcome.
- [x] Keep the engine's public outcome behavior while hardening its source
  binding to the validated open handle, stored byte length, and stored SHA-256.
- [x] Stop claiming on shutdown; allow a bounded drain, terminate safely at the
  deadline, and leave recoverable state when interrupted.
- [x] Ensure a panic or parser crash cannot silently remove the durable row.

Likely files:

```text
backend/src/jobs/mod.rs
backend/src/app.rs
backend/src/main.rs
backend/src/conversion/service.rs
backend/src/engines/pdf_inspector.rs
```

Exit: submissions never create conversion tasks directly, queue order is
deterministic, and one worker processes queued jobs across wake-up and polling.

### Increment 4: Startup reconciliation

Related: CVR-026.

- [x] Implement the state-specific recovery table above in one explicit
  reconciler.
- [x] Run recovery before readiness turns healthy and before the worker claims
  new jobs.
- [x] Validate published artifact containment, file type, byte count, hash, and
  manifest identity during reconciliation and download.
- [x] If a previously successful job has a missing or invalid required artifact,
  expose `failed` with the stable code `artifact_integrity_failed`, retain its
  attempt and artifact audit metadata, and return no downloadable artifacts.
- [x] Fail safely when the immutable source is missing, truncated,
  hash-mismatched, non-regular, or replaced by a symlink; never run a recovered
  attempt on unverified bytes.
- [x] Give each retry a new attempt; never reuse or overwrite an interrupted
  attempt directory.
- [x] Add a conservative orphan scan that only touches generated job paths,
  never follows symlinks, and moves pre-acceptance orphan trees into quarantine
  without deleting them.
- [x] Add deterministic persisted-state fixtures for interruption, publication,
  and success-commit crash windows.
- [x] Emit structured recovery counts and reason codes without document
  content.

Likely files:

```text
backend/src/jobs/recovery.rs
backend/src/artifacts.rs
backend/src/persistence/sqlite.rs
backend/src/app.rs
```

Exit: every crash window has a deterministic next state, and restart cannot
manufacture success or start concurrent attempts for one job.

### Increment 5: Runtime and Compose integration

Related: CVR-027.

- [x] Make `/health/ready` query SQLite and verify the configured data root is
  usable without mutating user artifacts.
- [x] Change capabilities truthfully from `durability: ephemeral` to
  persistent durability and replace `maxEphemeralJobs` with `maxActiveJobs`.
  Update the backend OpenAPI, implementation, and contract assertions together;
  generate and diff the TypeScript schema in a temporary location, but leave
  `src/app/api/schema.ts` untouched until M6. This is the only planned M2
  response-shape change.
- [x] Add a named development volume or explicit bind-mount example for
  `/data`; preserve loopback-only publication and current hardening.
- [x] Keep only `/data` writable in the eventual production shape; retain a
  bounded scratch `tmpfs` only if the parser needs it.
- [x] Add healthcheck timing that allows migrations and reconciliation to
  finish.
- [x] Document local development reset as a recoverable, explicit operation;
  never auto-delete the data root.

Likely files:

```text
backend/src/api/health.rs
backend/src/api/capabilities.rs
backend/src/config.rs
backend/compose.yaml
backend/Dockerfile
backend/README.md
backend/openapi/openapi.yaml
```

Exit: Compose restart preserves accepted jobs and artifacts, and readiness
reflects live persistence dependencies.

Readiness is proven by the HTTP contract suite. The persistence half of that
exit is wired, not yet demonstrated: `/data` is a named volume the image
pre-creates as `10001:10001`, and the Increment 6 smoke against the built
container is what proves a restart keeps the jobs.

### Increment 6: Failure and release verification

Related: CVR-028, CVR-029.

- [x] Test restart with a queued job. `crash_recovery.rs`. This reached the
  `ConversionState::Queued` arm of `reconcile_job` for the first time.
- [x] Use deterministic fault barriers to crash after claim, after entering
  `finalizing`, after publication rename, and before the success commit.
  `src/faults.rs` plus four tests in `crash_recovery.rs`.
- [x] Test replay and conflict after a fresh process starts. Covered in-process
  by `http_contract.rs`, and against a real restarted container by the smoke.
- [x] Test replay while active capacity is full and verify that replay,
  conflict, and capacity rejection leave no staged source. The conflict-at-
  capacity half is `failure_modes.rs`, which takes a different repository branch
  than the capacity rejection: `create_or_replay` answers Conflict before it
  ever counts active jobs.
- [x] Test that a simulated pre-acceptance crash orphan is moved into quarantine
  and is not automatically deleted. Already covered in `http_contract.rs`.
- [x] Test missing, truncated, hash-mismatched, non-regular, and symlinked
  source files before initial claim and recovered retry. `integrity_matrix.rs`.
- [x] Test missing, truncated, hash-mismatched, non-regular, and symlinked
  artifacts. `integrity_matrix.rs`.
- [x] Verify each corrupt-artifact case exposes
  `failed`/`artifact_integrity_failed`, retains internal audit metadata, and
  serves no artifacts. Asserted for all five artifact cases.
- [x] Test migration failure, locked database timeout, unwritable data root,
  and active-capacity rejection. `failure_modes.rs`. **Output-write failure is
  deferred**, with no portable way to make a write fail inside a temporary data
  root that does not also make the test lie about which call failed.
- [x] Test graceful shutdown with idle and active workers. The active-worker
  drain branch of `shutdown_until` had never run end to end before.
- [x] Run the HTTP-contract suite so routing behavior does not drift.
  **There are no M1 PDF fixtures to run.** The repository contains no `.pdf`
  file. Every PDF is synthesized in-process by `clean_pdf()`. A fixture corpus
  is CVR-040's work in M4, so this line was unbuildable as written.
- [x] Build the image and perform a real sequence: submit -> stop container ->
  start container -> poll -> download -> verify hashes.
  `backend/scripts/container-smoke.sh --phase graceful`.
- [x] Perform a separate unclean restart with `SIGKILL` while a barrier holds a
  known nonterminal state and the WAL contains uncheckpointed work; restart and
  verify deterministic recovery. `--phase sigkill`. The in-process fault barrier
  cannot reach into a container, so the hold uses the documented
  `TOOLKIT_CONVERTER_PDF_WORKER_PATH` override pointed at a stub worker that
  sleeps. The held state is polled for, not assumed.
- [x] Record commands, versions, results, and any exceptions in the epic.

The container smoke proves the graceful and forced-kill windows only. The other
two barriers, entering `finalizing` and the post-rename boundary, are held in
the Rust suite, because holding them from outside the process is not possible
without a code seam. Do not read the smoke as covering them.

Exit: every M2 gate passes with evidence; AnyDoc work may begin only afterward.

## Test matrix

| Layer | Required evidence |
|---|---|
| Migration | Fresh database, repeat startup, unsupported/newer schema failure |
| Repository | Create, replay, conflict, capacity, ordered claim, legal/illegal transitions, concurrent submission |
| Artifact store | Containment, immutable source, attempt isolation, atomic publish, hash/size mismatch, symlink rejection |
| Worker | One claim at a time, wake and poll, engine success, `needs_remote`, timeout/crash, shutdown |
| Recovery | Every nonterminal state, both sides of the publication rename boundary, corrupted-success failure semantics, and pre-acceptance quarantine |
| HTTP contract | Existing 202/200/404/409/413/415/429 behavior, the reviewed capabilities change, and restart-visible GET/artifacts |
| Container | Persistent volume, live readiness, loopback bind, non-root/read-only/cap-drop constraints |
| End to end | Accepted job and idempotency replay survive graceful and forced-kill container restarts |

## Risks and controls

### Database/filesystem atomicity

SQLite cannot commit a filesystem rename in the same transaction. Control the
gap with an explicit `finalizing` state, publish-before-success ordering,
attempt-scoped immutable output, hashes, and startup reconciliation.

### Duplicate local execution

The delivery model is at-least-once. Claim rows transactionally, run one worker,
limit recovery attempts, and never reuse an attempt directory. Local parser
re-execution is acceptable; future remote submission will require its own
uncertain-acceptance state.

### Network filesystem corruption

SQLite WAL is same-host only. Fail deployment review if `/data` is an SMB/NFS
mount. Use a host-local TrueNAS/ZFS dataset and provide files to clients only
through the API.

### Disk exhaustion

Keep upload/output ceilings, bound active jobs, and make every cleanup or
quarantine action job-scoped. Free-space admission/readiness, retention, and
reviewed quarantine deletion are M7 work; M2 must fail individual writes safely
but does not predict available space before acceptance.

### Shutdown and parser lifetime

Stop claims first, use a bounded grace period, and preserve a recoverable
database state if the process must exit. The worker subprocess timeout remains
the hard upper bound for normal execution.

### Migration failure

Run migrations before binding the listener, keep versioned SQL in the image,
and fail closed on an unsupported schema. Do not attempt destructive automatic
downgrades.

### Current parser isolation boundary

The child process provides crash and timeout isolation but is not a separate
container security boundary. M2 remains loopback-only; M7 owns the complete LAN
deployment and supply-chain gate.

### Dirty shared worktree

Limit edits to backend-owned paths recorded in `docs/BACKEND_BASELINE.md`, reread
overlapping files before every patch, and stage explicit paths only. Never use
`git add .` or modify concurrent desktop work during M2.

## Verification commands

The final M2 run should include at least:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo check --locked --offline --manifest-path backend/Cargo.toml
cargo test --locked --offline --manifest-path backend/Cargo.toml
cargo clippy --locked --offline --manifest-path backend/Cargo.toml --all-targets -- -D warnings
docker compose -f backend/compose.yaml config --quiet
docker build -t tool-kit-converter:m2 backend
```

The restart smoke tests must use the built container and its persistent `/data`
volume. Run both a graceful stop/start and a forced `SIGKILL` at a known fault
barrier with WAL work pending; a native-process test alone does not satisfy
CVR-029.

## M2 completion gate

M2 is complete only when all of the following are true:

- all CVR-020 through CVR-029 tickets are checked with evidence;
- existing M1 API and PDF conversion tests still pass;
- no accepted job or idempotency record is lost on a full container restart;
- replay, conflict, and capacity rejection leave no unowned source data;
- pre-acceptance crash leftovers are quarantined and never auto-deleted by M2;
- queued and interrupted local work recover within a bounded attempt policy;
- no worker executes a missing, substituted, symlinked, or hash-mismatched
  source;
- no success response can reference a missing, invalid, or unverified artifact;
- a corrupted prior success becomes `failed`/`artifact_integrity_failed`, retains
  its audit metadata, and serves no artifacts;
- Compose has one converter service plus persistent `/data`, with no database
  or broker service;
- the graceful and forced-kill image restart smoke tests pass; and
- the backend remains loopback-only pending M7.

## Execution boundary

M2 is complete. Increments 0-6 are verified and committed on `main`; the
closing checkpoint is `01f1bf1`. Do not add AnyDoc or other scope here.
Parallel sessions start at [`STATUS.md`](../STATUS.md).
