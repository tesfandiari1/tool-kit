> **Superseded by [`../STATUS.md`](../STATUS.md) as of 2026-08-19.** Historical copy only.

# Session handoff

**Last updated:** 2026-08-19
**Branch:** `backend/m4-routing-policy`, off `main` at `d6fc6eb`. Two commits:
`5fe6463` (backend Sprint 0 repairs plus M4) and `a7b4976` (desktop Sprint 0
repairs). Nothing is pushed.

**Two things about the tree you need before you touch it.**

A concurrent desktop session has uncommitted work here: the title bar, page
zoom, `Path`/`splitLayout` primitives, `geometry.ts`, `useZoom.ts`,
`barStatus.ts`, and the `src-tauri` capability/settings/conf changes that zoom
needs. None of it is committed, all of it is green, and none of it is mine.
Leave it for that session.

Two of the Sprint 0 fixes are stranded inside it. `scanKey` including
`conversionRoute`/`backendUrl` (S0.7) and the capability probe retry (S0.8) are
in `src/shell/App.tsx`, which that session rewrote at the same time, so they
could not be staged separately. They are in the working tree and gated, just
not in either commit above.

`d6fc6eb` on `main` is titled "docs: add comprehensive backend and desktop
execution plans". It is not. It contains exactly the six `git mv` renames into
`docs/archive/`, swept up by a concurrent commit. Unpushed, so amend the message
if you care.
**Plan:** [`CLOSEOUT_EXECUTION_PLAN.md`](CLOSEOUT_EXECUTION_PLAN.md) covers
local same-machine use: backend on loopback, text-based inputs to Markdown,
corpus gate, doc prune. Sprint 0 and Sprint 1 are landed. Sprint A (desktop
remainder) is next. M5 through M8 stay in [`BACKEND_EPIC.md`](BACKEND_EPIC.md).
**Backend checkpoint:** `4590a9a` closes M3; `322d96e` is the later
OpenAPI/runtime route-kind correction consumed by the desktop. AnyDoc converts
19 extensions over 18 media types in-process under a hard timeout, PDF stays on
its isolated worker, and the container smoke converts a docx inside the image.
Contract is **0.4.1**: the manifest is engine-keyed and the multipart
`contentType` lists all 18 admitted media types.
**Recorded code state:** the live worktree has unrelated security, layout, and
UI changes plus `CLAUDE.md` metadata. Preserve them and stage explicit paths.
**Read this first** in any parallel session, then re-read the live worktree.
This file goes stale the moment someone lands a commit.

## M6 transport decisions (owner-settled 2026-08-18)

- `POST /api/v1/conversions` is a Rust host special-case that parses the source
  path from JSON and builds/streams the multipart request. File bytes and the
  bearer token stay out of the webview.
- Markdown download gets a separate Tauri command returning an output path and
  never crosses IPC as a body.

## Do this next

0. **Sprint 0 and Sprint 1 are landed.** What changed, and the invariants each
   one now carries:

   - Startup recovery contains a per-job failure instead of aborting the boot.
     Only a repository error stops the service; an unreadable row is left for
     revalidation and an untrustworthy one is quarantined `failed` /
     `recovery_state_unrecoverable`. **When you add a state or a vocabulary,
     the question is no longer "did I update every layer" but "what does this
     cost if I did not".**
   - Migrations 0002 and 0003 wrap their rebuild in one transaction, pinned by
     `every_no_transaction_migration_wraps_its_rebuild_in_one_transaction`.
     Editing an applied migration is only safe because nothing is deployed.
     **After the first deployment, a broken migration needs a new file.**
   - `execute_claimed` bounds its engine-permit wait and honours shutdown. A
     detached AnyDoc parse still holds its permit on purpose, so the wait is
     real; past the bound the runner exits and startup recovery requeues.
   - The desktop ledger is schema 4. **A remote fallback is recorded before its
     request is sent.** Provider set with no request id means the outcome is
     unknown, and recovery refuses to resubmit rather than risk a second
     charge. Before this, every restart mid-fallback billed the file twice.
   - Recovery refuses to resume against a backend URL that is no longer
     configured, because the keychain holds exactly one backend token.
   - Direct-path `fail`/`finish` take a generation, like their backend
     counterparts. Nothing writes back to a stopped row.
   - `retry_job` and `retry_failed` re-persist the durable row Stop deleted.
   - **Both writes happen before a billable Datalab request, and both are
     required.** The ledger survives a crash, the in-memory `FallbackContext`
     survives a same-process Retry. Recording only one left Retry able to pay
     twice with no restart involved.
   - Recovery rows decode independently (`RecoveryCandidate`). Containing
     per-job reconciliation errors is worthless if the listing that produces
     the jobs still fails wholesale on one bad column.
   - `set_status` is the guarded setter for both routes. There is no unguarded
     write-back left in `jobs.rs`.

   Two things are accepted rather than fixed, on the record: a timed-out AnyDoc
   parse ends in a supervised restart rather than in-process containment, and
   AnyDoc publishes with no completeness warning because it cannot measure one.
   Both are argued in [`CLOSEOUT_EXECUTION_PLAN.md`](CLOSEOUT_EXECUTION_PLAN.md).

   **The built `Tool-Kit.app` bundle is not rebuilt by any gate.** `pnpm
   verify:all` runs `tsc`, ESLint, Vitest, `vite build`, and both Clippy/test
   suites. None of them produce an app. Opening
   `src-tauri/target/release/bundle/macos/Tool-Kit.app` runs whatever was last
   bundled. Use `pnpm tauri dev` to exercise current source.

1. **M4 is landed.** `conversion/policy.rs` is a pure
   `decide(profile, route, LocalResult) -> PolicyDecision`. Read it before
   changing any routing behaviour, and read the correction note in the closeout
   plan before trusting `confidence`.

   **The one thing to carry forward:** `QualitySignals::native_text_ratio` is
   the share of pages carrying extractable text. It is **not** a completeness
   measure and the policy must never route on it. A ten-page report with a
   one-line cover page and no images reports 0.9 with nothing missing; a page
   holding both text and a full-page scan reports 1.0 with the scan lost. It
   warns, and `a_sparse_cover_page_is_not_missing_content` stops anyone turning
   it back into a route.

   Three PDF-fixture traps, all found the hard way: body text must not begin
   with the word "Page" (pdf-inspector strips those lines and the sparse
   branch then flags every page for OCR); `form_pdf` produces an inspection
   identical to the same page with no AcroForm; `garbled_font_pdf` never
   reaches the garbled branch because `hasEncodingIssues` never sets.

2. **M1, M2, M3, and the M6 implementation are closed. Do not redo them.**
   Evidence is in
   [`archive/BACKEND_VERIFICATION_LOG.md`](archive/BACKEND_VERIFICATION_LOG.md).
   Local same-machine work is in
   [`CLOSEOUT_EXECUTION_PLAN.md`](CLOSEOUT_EXECUTION_PLAN.md).
   LAN deploy and cutover stay in [`BACKEND_EPIC.md`](BACKEND_EPIC.md).

   Traps from that work that are still live and still easy to reintroduce:

   - The Dockerfile must keep copying `migrations/` and `build.rs`, or
     `sqlx::migrate!` cannot compile in the image. The runtime stage must keep
     `RUN install -d -o 10001 -g 10001 -m 0700 /data`, or a fresh named volume
     lands as `root:root` while the service runs as `10001`. No `VOLUME`
     instruction: it hands plain `docker run` an anonymous volume nobody prunes.
   - **A PDF-shaped assumption in an engine-neutral layer crash-looped the
     service, and PDF-only tests hid it.** When you add an engine, grep for the
     other engine's vocabulary in shared layers, and make at least one restart
     and one container test use the new engine's input.
   - **The fault barrier is production code that production never arms.**
     `backend/src/faults.rs` is reachable only from a test holding an
     `AppState`. Do not add a flag, an environment variable, or a route that
     arms it. Its four call sites sit between committed transactions on
     purpose: a parked task must hold no SQLite write lock.
   - **A format is advertised only with a passing round-trip fixture**, and
     `advertised_media_types_match_the_migration_check` plus
     `advertised_media_types_match_the_openapi_upload_contract` pin the
     admission table to the migration CHECK and the upload contract. Drift
     turns a clean 415 into an INSERT constraint error at runtime.
   - **Ten desktop extensions have no local engine**: png, jpg, jpeg, webp,
     tiff, tif, gif, bmp, html, htm. They belong to the remote route or the
     desktop's direct fallback, never to the backend.
   - **Run the container smokes with a CPU-capped builder.** The image build is
     the only part that saturates the machine, and the default builder lives
     inside the Docker VM where `docker update` cannot reach it:

     ```bash
     docker buildx create --name toolkit-capped --driver docker-container --bootstrap
     docker update --cpus 4 buildx_buildkit_toolkit-capped0
     TOOLKIT_SMOKE_BUILDER=toolkit-capped backend/scripts/container-smoke.sh
     ```

   - **One reviewed finding stays open on purpose.** `/health/ready` is
     unauthenticated and opens a `BEGIN IMMEDIATE` transaction against a
     four-connection pool per request. Exposure is loopback-only today. Give it
     a real rate limit when Caddy and LAN exposure land (M7).

3. **Do not** create a root Cargo workspace, bump TypeScript 7, migrate
   `keyring` 4, unpin `pdf-inspector`, or take `libc` 1.0 (still alpha).

## Starting local same-machine closeout

Read [`CLOSEOUT_EXECUTION_PLAN.md`](CLOSEOUT_EXECUTION_PLAN.md). Order: Sprint A
(desktop remainder) → B (Compose stack) → C (corpus gate) → D (manual
acceptance) → E (doc prune).

M5 through M8 design notes stay in [`BACKEND_EPIC.md`](BACKEND_EPIC.md). Do not
start backend Datalab work until local same-machine use passes Sprint D.

## Parallel session ownership

| Session | May edit | Must not edit |
|---|---|---|
| Backend | `backend/**`, `docs/*.md` | `src/**`, `src-tauri/**` |
| Desktop | `src/**`, `src-tauri/**` | `backend/**` persistence/worker/OpenAPI |
| Shared docs | `README.md`, `CLAUDE.md`: append or reconcile | Reverting the other session's section |

Shared repo files (`.github/`, `LICENSE`, `rust-toolchain.toml`,
`.node-version`, `.gitignore`, `package.json`) belong to neither session. Do
not revert them as part of a backend patch.

**`pnpm verify` is deliberately desktop-scoped** so in-flight backend work
cannot fail a desktop change. Use `pnpm verify:backend` or `pnpm verify:all`
when you want the converter checked too.

## Current product state

- Desktop conversion defaults to the existing direct Datalab route and
  transcription still uses Rev.ai. A user may now select the local conversion
  backend; Run is planned per file from live `capabilities.inputFormats`, while
  image/HTML formats stay permanently direct and `local_only` rejects anything
  that would require Datalab. Keys stay in the macOS Keychain.
- The desktop does **not** render in a plain browser, and that is expected.
  `useWindowFocusClass` and `useCloseConfirm` call `getCurrentWindow()` in a
  mount effect; with no `window.__TAURI_INTERNALS__` that throws and React 19
  tears the tree down. Use `pnpm tauri dev`, or `?gallery` for the
  bridge-free design review.
- The OpenAPI client at `src/app/api/` is live at contract 0.4.1. The
  capability preflight consumes it through the Tauri-backed fetch seam, and
  the registered host handler owns the base URL, bearer token, multipart file
  stream, response limits, and token redaction. Submit and poll validate UUID
  response identity; poll also requires the response ID to equal the requested
  job before any artifact can be downloaded. The webview sees neither bearer
  values nor document bytes.
- Backend jobs persist a stable client run ID, idempotency key, original
  backend URL, source mtime, profile, and optional backend job ID before
  submission. Restart recovery replays or resumes against that recorded URL.
  Generation retirement is serialized with guarded backend mutation, terminal
  bookkeeping, and `job-updated` emission; a stale download loses the guard and
  deletes only the exact collision-safe output path it created. Unknown future
  statuses stay pending through an explicit terminal allowlist.
- Backend-mode history reuse/copy is intentionally disabled. Direct and
  backend results are filed under distinct `output_format` values, but the
  matcher does not select by provenance yet. Sprint 3 of the closeout plan
  fixes it; re-running is the safe behaviour until then.
- Backend M1, M2, and M3 are complete. Evidence is in
  [`archive/BACKEND_VERIFICATION_LOG.md`](archive/BACKEND_VERIFICATION_LOG.md).
- **`TOOLKIT_CONVERTER_SCRATCH_PARENT` is dead config.** `config.rs` parses and
  validates it and nothing reads it. Left in place on purpose: removing an
  environment variable changes the public surface and deserves its own
  decision. `backend/.env.example` and `backend/README.md` both say so. Do not
  wire it to anything on the assumption it was forgotten.
- **Deferred from Increment 6, on the record:** output-write failure injection
  (no portable way to induce it), the M1 PDF fixture corpus (no `.pdf` exists in
  the repo; that is CVR-040 in M4), and a multi-process restart test in Rust
  (the container smoke covers it against the real binary).
- The service remains loopback-only. No LAN or Caddy exists yet, and Datalab
  fallback inside the backend remains M5 work; the M6 `standard` profile falls
  through to the existing desktop Datalab lifecycle instead.

## Toolchain and dependencies

Pins: Rust **1.97.1** (`rust-toolchain.toml`), Node **24** (`.node-version`),
pnpm **11.22.0** (`package.json` `packageManager`).

Stay current with weekly Dependabot (`.github/dependabot.yml`). One-shot
catch-up:

```bash
pnpm outdated
pnpm update --latest
# then pin TypeScript back to 6.x: 7 breaks typescript-eslint
pnpm add -D typescript@6.0.3

cargo update --manifest-path backend/Cargo.toml
cargo update --manifest-path src-tauri/Cargo.toml
```

Bumping a Cargo major means editing `Cargo.toml` ranges, not only
`cargo update`. After a catch-up, run `pnpm lint && pnpm build` and both
crates' clippy/tests.

Deliberately not latest:

- TypeScript stays on **6.0.3** until typescript-eslint supports 7.
- `keyring` stays on **3.x** (`apple-native`). 4.x needs a `keyring-core`
  rewrite.
- `pdf-inspector` stays **`=1.15.0`** (latest, and the Dockerfile copies that
  exact crate path).
- `openapi-typescript` still peers on TypeScript 5; that is only the
  `pnpm generate:api` CLI.
- `openapi-fetch` 0.17 is the OpenAPI client runtime. Keep it paired with
  `openapi-typescript` when Dependabot bumps either.

## GitNexus

Indexed as **`tool-kit`**, currently on **1.6.9**. The graph lives in
`.gitnexus/` (gitignored). MCP tools must pass `repo: "tool-kit"`. Refresh
after large commits:

```bash
gitnexus analyze . --index-only   # incremental, ~15s on this repo
gitnexus status                   # indexed commit vs HEAD
```

`--index-only` is load-bearing, not decoration. A plain `analyze` rewrites the
`<!-- gitnexus:start -->` block in `AGENTS.md` and `CLAUDE.md` with the current
node and edge counts, so every refresh leaves two modified tracked files that
have nothing to do with the work in hand. The flag skips that injection and
leaves the committed block alone.

Two traps, both hit on 2026-08-17:

- **Upgrade the copy in Homebrew's prefix, not fnm's.** `gitnexus` is an npm
  global at `/opt/homebrew/lib/node_modules/gitnexus`, and `~/.claude.json`
  launches the MCP server as `/opt/homebrew/bin/gitnexus mcp`. The active
  `npm` is fnm's, so a plain `npm i -g` installs a *second* copy that shadows
  it on PATH and breaks again on the next `fnm use`. Use
  `/opt/homebrew/bin/npm install -g gitnexus@latest`.
- **An index written by a newer GitNexus is unreadable by an older one.** The
  symptom is `LadybugDB unavailable ... Database file version: 42, Current
  build storage version: 40`, which reads like a transient lock but is a
  version mismatch. Upgrade the binary, then restart the MCP server: a running
  stdio server keeps the old code in memory, so `/mcp` reconnect or a new
  session is required. `gitnexus doctor` confirms the native modules loaded.

What the graph is good for here: its community detection tracks the folder
layout, so a file spanning several communities is the signal that it holds
more than one concern. That is how `App.tsx` was split. It is unreliable for
dead-code hunting in this codebase, because object-literal methods
(`commands.*`), destructured hook returns, and default exports behind a
dynamic `import()` all read as uncalled. Verify any "unused" claim with grep
before acting on it. Graphify is not installed.

## Automation hazards

Both of these happened while implementing Increment 6, and both cost real time.
Put the prohibitions in the prompt when you fan work out to subagents here.

- **Never let an agent generate synthetic load.** One asked to prove a timing
  assertion was not flaky "on a loaded machine" built one:
  `for i in $(seq 1 $((ncpu*2))); do (while :; do :; done) & done`, cleaned up
  with `kill $(jobs -p)`. `jobs -p` returns nothing in a non-interactive
  `zsh -c`, so nothing died. The parent exited, 24 spin loops reparented to
  PID 1, and the machine sat near 960% CPU until killed by hand. Killing matched
  PIDs needs a zsh array (`PIDS=($(...))`): zsh does not word-split unquoted
  variables, so `for p in $PIDS` yields one "illegal pid" argument.
- **Scope agents to explicit paths and verify they finished before reverting.**
  A gate agent did an unrequested docs consolidation that deleted
  `BACKEND_EXECUTION_PLAN.md` and stripped its references from five files, then
  **re-applied the same change after the first revert** because it was still
  running. Check file mtimes rather than trusting a task-status report. Tracked
  files come back with `git checkout HEAD --`; an untracked file is simply gone,
  so `git add` new work early.

- **A read-only "audit and synthesize" fan-out is the wrong shape for this
  repo.** One was run on 2026-08-18 to review M3: it produced a long ranked
  report and fixed nothing, while the blocker, the missing xlsx fixture, the
  PDF-only smoke, and the dead CSV hint all came from reading the code
  directly. If you fan work out here, have the agents land patches behind a
  gate, or do not fan it out.

Cap the Docker build rather than letting it take all cores. See the capped
builder recipe under "Do this next".

## Verify

```bash
pnpm check           # tsc --noEmit + eslint + vitest         (fast inner loop)
pnpm verify          # check + build + src-tauri clippy/tests (desktop scope)
pnpm verify:backend  # converter clippy/tests
pnpm verify:all      # both
pnpm verify:container # built image + graceful and SIGKILL restart smokes
```

`verify:container` is deliberately **not** part of `verify:all`. The other three
are offline cargo/tsc/eslint runs; the container gate needs a Docker daemon,
builds an image, and takes minutes, so folding it in would break the everyday
gate on any machine without Docker running.

CI (`.github/workflows/ci.yml`) runs frontend, backend, and desktop as three
jobs on push/PR. Desktop clippy/tests
use `macos-latest` because of `macos-private-api` and `keyring`.
**CI does not run the container smoke.** It is a local release gate, so re-run
it by hand before closing any milestone that touches persistence, the worker,
the Dockerfile, or Compose. Its evidence lands in
`backend/target/container-smoke/<utc-timestamp>/evidence.md`, which is
gitignored; paste the relevant lines into the epic rather than linking the path.
The latest run is `20260819T022230Z` against image `sha256:8bb30ae5…`: both
phases pass, the migration rebuild runs fresh inside the image, and the
evidence includes the in-container AnyDoc docx conversion.

## Doc map

Four live documents. Everything closed is in
[`archive/`](archive/README.md) with a note on why.

| File | Role |
|---|---|
| `docs/HANDOFF.md` | This file. Current state and parallel-session rules |
| `docs/CLOSEOUT_EXECUTION_PLAN.md` | The active plan for M4 through M8 |
| `docs/BACKEND_EPIC.md` | Milestone tracker and CVR tickets |
| `docs/BACKEND_SERVICE_PLAN.md` | Approved architecture |
| `README.md` | How to run, test, and release |
| `CLAUDE.md` | Desktop architecture invariants |
| `AGENTS.md` | Learned user preferences and workspace facts |
| `src/ui/UI.md` | Design language and the rules for extending it |
| `backend/README.md` | Converter setup and env vars |
