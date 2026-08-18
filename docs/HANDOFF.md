# Session handoff

**Last updated:** 2026-08-18
**Branch:** `main`, HEAD `b2b4d10`. Branch fresh off `main` for M6.
**Backend checkpoint:** `b2b4d10` plus an **uncommitted format-set widening**
in the worktree. M3 is complete: AnyDoc converts 18 extensions in-process
under a hard timeout, PDF stays on its isolated worker, contract is 0.4.0, and
the container smoke converts a docx inside the image.
**Recorded code state: the tree is dirty, in two unrelated piles.**
1. The format-set widening (this session): `backend/src/{conversion/model.rs,
   api/conversions.rs, engines/anydoc.rs, persistence/sqlite.rs}`,
   `backend/migrations/0003_anydoc_full_format_set.sql`, 18 new fixtures,
   `backend/openapi/openapi.yaml`, `BACKEND_EPIC.md`, and this file. Gate is
   green; ready to commit.
2. A **repo-wide prose sweep from a parallel session** (em dash to colon or
   period) across `CLAUDE.md`, `AGENTS.md`, `README.md`,
   `backend/README.md`, `backend/evals/README.md`, and four `docs/` files.
   Mostly punctuation, but it carries a handful of real content edits and it
   rewrote the generator-owned `<!-- gitnexus:start -->` block, which
   `gitnexus analyze --index-only` exists to leave alone. **Six of those files
   are outside backend ownership. Commit it separately, by whoever made it.**
   Do not fold it into a backend commit and do not revert it blind.
**Read this first** in any parallel session, then re-read the live worktree.
This file goes stale the moment someone lands a commit.

## Do this next

1. **M3 is closed. Commit the close-out, then start M6.** The M3 gate is met
   (see the 2026-08-18 close-out entry in `docs/BACKEND_EPIC.md`). M6 begins
   with `pnpm generate:api`: the committed `src/app/api/schema.ts` trails
   OpenAPI 0.4.0 by the CVR-027 and M3 deltas. Read the diff, do not
   hand-reconcile it.

   **CVR-036 now advertises the full AnyDoc format set: 19 extensions over 18
   media types.** An earlier pass this same day closed it by narrowing, on the
   grounds that the desktop cannot send rtf/odt/ods/odp/csv. That was the wrong
   test for a document-conversion service, and it also missed extension variants
   (`docm`, `xlsm`, `pptm`, `ppsx`, `ppsm`, `pps`, `pot`) that run through
   parsers already shipped. All 12 additions were probed against the real engine
   before being advertised. `xlsb` is the one deliberate omission: no fixture
   exists and a binary workbook cannot be honestly derived from an XML one.

   **The rule that keeps the admission table honest:** a format is advertised
   only with a passing round-trip fixture, and
   `advertised_media_types_match_the_migration_check` pins the table to the
   migration CHECK in both directions. Drift there turns a clean 415 into an
   INSERT constraint error at runtime.

   **The real remaining format gap is not AnyDoc's.** Ten desktop extensions
   have no local engine: png, jpg, jpeg, webp, tiff, tif, gif, bmp, html, htm.
   AnyDoc has no `Format` variant for any of them, so they belong to the remote
   route (M5) or the desktop per-file fallback (M6). CVR-065 must gate Run on
   `capabilities.inputFormats` rather than assume the backend takes all 18.

   **A PDF-shaped assumption in an engine-neutral layer crash-looped the
   service, and PDF-only tests hid it.** Startup recovery knew only the four
   PDF classifications, so the first successful AnyDoc job made the next boot
   fail with `PersistedMetadataInvariant` and exit 1. When you add an engine,
   grep for the other engine's vocabulary in shared layers, and make at least
   one restart and one container test use the new engine's input.

   CVR-039 (skip the redundant parse re-read) stays optional and unscheduled.
   Measure first. M1 and M2 are closed.

   **Run the container smokes with a CPU-capped builder.** The image build is
   the only part that saturates the machine, and the default builder lives
   inside the Docker VM where `docker update` cannot reach it:

   ```bash
   docker buildx create --name toolkit-capped --driver docker-container --bootstrap
   docker update --cpus 4 buildx_buildkit_toolkit-capped0
   TOOLKIT_SMOKE_BUILDER=toolkit-capped backend/scripts/container-smoke.sh
   ```

   On a 12-core host that holds the builder at ~405% and host load near 4. The
   running service is already capped by `cpus: 2.0` in `compose.yaml`.

   **The fault barrier is production code that production never arms.**
   `backend/src/faults.rs` is reachable only from a test holding an `AppState`.
   Do not add a configuration flag, an environment variable, or an HTTP route
   that arms it. Its four call sites sit between committed transactions on
   purpose: a parked task must hold no SQLite write lock, or a restarted
   `AppState` could not open the same database.
2. **M1 and M2 are closed and merged; do not redo them.** The blow-by-blow
   lives in the `docs/BACKEND_EPIC.md` verification log. Two traps from that
   work are still live and still easy to reintroduce: the Dockerfile must
   keep copying `migrations/` and `build.rs` (or `sqlx::migrate!` cannot
   compile in the image), and the runtime stage must keep
   `RUN install -d -o 10001 -g 10001 -m 0700 /data` (a fresh named volume
   otherwise lands as `root:root` and the service runs as `10001`). No
   `VOLUME` instruction. it hands plain `docker run` an anonymous volume
   nobody prunes.

   **One reviewed finding stays open on purpose.** `/health/ready` is
   unauthenticated and opens a `BEGIN IMMEDIATE` transaction against a
   four-connection pool per request, so a local process can flood it and
   contend the write lock with the job runner. Exposure is loopback-only
   today. Give it a real rate limit when Caddy and LAN exposure land (M7).

3. **Desktop OpenAPI prep is done; M6 still owns the wire-up.** The user
   authorized shaping the frontend to the contract ahead of M6. `src/app/api/`
   is a standard openapi-typescript/openapi-fetch layer, and
   `commands.serviceRequest` is its typed IPC door. Remaining M6 work: the
   Rust `service_request` handler, backend URL + Keychain token settings, and
   wiring the client into the run view. **`src/app/api/schema.ts` is now stale
   on purpose.** Increment 5 landed the CVR-027 OpenAPI delta, so the committed
   schema no longer matches `backend/openapi/openapi.yaml`. The differences are
   the new `ReadinessResponse` schema plus both `/health/ready` responses,
   `durability: "persistent"`, `maxActiveJobs` for `maxEphemeralJobs`, and the
   reworded summaries. Run `pnpm generate:api` as the first step of M6, not
   before.

   **The dual-pane workspace is closed.** Compact is a `Panel` launcher;
   opening a result splits the window; Edit writes through `write_document`.
   Do not re-do that work. Record: `docs/WORKSPACE_HANDOFF.md`. M6 still owns
   `service_request`; do not treat it as available. `src/app/api/schema.ts`
   stays M6's.
4. **Do not** create a root Cargo workspace, bump TypeScript 7, migrate
   `keyring` 4, unpin `pdf-inspector`, or take `libc` 1.0 (still alpha).

## Parallel session ownership

| Session | May edit | Must not edit |
|---|---|---|
| Backend / M2 | `backend/**`, `docs/BACKEND_*.md`, this file | `src/**`, `src-tauri/**` |
| Desktop | `src/**`, `src-tauri/**` | `backend/**` persistence/worker/OpenAPI |
| Shared docs | `README.md`, `CLAUDE.md`: append or reconcile | Reverting the other session's section |

Planning docs live in `docs/`, not the repo root. The baseline owned-path list
is in `docs/BACKEND_BASELINE.md`. Shared repo files (`.github/`, `LICENSE`,
`rust-toolchain.toml`, `.node-version`, `.gitignore`, `package.json`) are not
backend-owned; do not revert them as part of an M2 patch.

**`pnpm verify` is deliberately desktop-scoped** so in-flight backend work
cannot fail a desktop change. Use `pnpm verify:backend` or `pnpm verify:all`
when you want the converter checked too.

## Recent desktop commit

The frontend was reorganised and cleaned up. One concern per folder:

```
src/
  app/        types, commands (the IPC door), format, api/ (OpenAPI client)
  platform/   host.ts - dialogs, window, drag-drop, clipboard
  domains/    run/ history/ settings/ thread/
  shell/      App.tsx, App.css, useToast, useDocuments, useDocumentSave, useHostWindow
  ui/         design system, imported as @ui, depends on nothing of ours
  main.tsx
```

What the next session needs to know:

- **The shell is a dual-pane workspace (closed).** Compact is the `Panel`
  launcher. A document opens `SplitPane` + `DocumentPane`; Settings and History
  replace the left pane only. `View` has no `"thread"`. Edit autosaves via
  `write_document` (mtime guard). See `docs/WORKSPACE_HANDOFF.md`. Do not start
  another weave.
- **`App.tsx` moved to `src/shell/`** and shed its non-run concerns into hooks
  beside it. It still owns run state, settings, and the view switch.
- **`src/domains/viewer/` folded into `src/domains/thread/`.** Thread was its
  only consumer, so it was never a peer domain.
- **`tsconfig.node.json` no longer emits into the repo root.** It is a
  composite project and so may not set `noEmit` (TS6310); its output is
  redirected to `node_modules/.tmp`. Before this, every `pnpm build` wrote
  `vite.config.js` and `vite.config.d.ts` into the root, untracked and
  ungitignored. Both names are now in `.gitignore` as a backstop. **If those
  files reappear, that redirect was reverted.**
- **`hostFetch` honours a whole `Request`.** It is typed `typeof fetch`, so
  reading only `init` silently turned `new Request(url, {method, body})` into
  a GET with no headers and no body. `init` still wins on conflict, matching
  real `fetch`. Covered by `src/app/api/transport.test.ts`.
- **`transport.ts` no longer calls `invoke` directly**; it goes through
  `commands.serviceRequest`. **Exactly two modules may import
  `@tauri-apps/*`:** `src/app/commands.ts` (IPC) and `src/platform/host.ts`
  (dialogs, window, drag-drop). A third is the thing to reject in review.
- **The `@ui` ESLint boundary lists relative depths explicitly**
  (`../app/*`, `../../shell/*`, and so on). Moving a file changes its depth,
  so **re-probe the rule after any move**: add a violating import inside
  `src/ui` and confirm ESLint errors. A silently dead boundary rule is worse
  than none.
- Deleted the starter leftovers (`src/assets/react.svg`, `public/vite.svg`,
  `public/tauri.svg`); the favicon is now `public/icon.png`. Dropped
  `ruff.toml` and the Python `.gitignore` rules, since there is no Python here.
- Tests went from 12 to 19, and the Vitest glob is now
  `src/**/*.test.{ts,tsx}` so a future component test is actually collected.

Deliberately not done: no `apps/*` + `packages/*` monorepo migration.
`src-tauri/tauri.conf.json` sets `frontendDist: "../dist"` and
`beforeBuildCommand: "pnpm build"`, so the frontend must build from the repo
root. `index.html`, `vite.config.ts`, `tsconfig*.json`, and `package.json` at
the root is the required layout, not clutter.

## Current product state

- Desktop still converts via Datalab and transcribes via Rev.ai. Keys stay in
  the macOS Keychain. Verified under `pnpm tauri dev` on 2026-08-17: the
  window renders, autodetect and the output folder work, and a run starts and
  reaches the provider.
- The desktop does **not** render in a plain browser, and that is expected.
  `useWindowFocusClass` and `useCloseConfirm` call `getCurrentWindow()` in a
  mount effect; with no `window.__TAURI_INTERNALS__` that throws and React 19
  tears the tree down. Use `pnpm tauri dev`, or `?gallery` for the
  bridge-free design review.
- Desktop frontend now has an OpenAPI client layer at `src/app/api/`
  (user-authorized pre-M6 prep). `schema.ts` is generated from
  `backend/openapi/openapi.yaml` and now trails it by the Increment 5 delta,
  which M6 regenerates. `client.ts` is a
  standard `openapi-fetch` client typed by that schema: call sites get the
  library's `{ data, error }` results, with `error` carrying the contract's
  ErrorEnvelope. `transport.ts` plugs a Tauri-backed `fetch` into
  openapi-fetch's custom-fetch seam: every request goes through one
  `service_request` command with payload `{ method, path, headers, body? }`
  and answer `{ status, headers, body }`. The host resolves the real base
  URL, attaches the Keychain token, and for `createConversion` streams the
  file whose desktop path sits in the `source` field. `index.ts` re-exports
  the client plus the `paths`/`components`/`operations` schema type
  namespaces. ESLint blocks importing the generated schema outside
  `src/app/api/`. Nothing consumes the client yet; `service_request` has no
  Rust handler until M6.
- Backend M1, M2, and M3 are complete. M1 shipped the loopback PDF slice; M2
  made jobs, sources, and artifacts durable across restart; M3 added AnyDoc
  for the seven non-PDF desktop formats. Per-increment evidence, checkpoints,
  and test counts are in the `docs/BACKEND_EPIC.md` verification log rather
  than repeated here.
- **`TOOLKIT_CONVERTER_SCRATCH_PARENT` is dead config.** `config.rs` parses and
  validates it and nothing reads it. Left in place on purpose: removing an
  environment variable changes the public surface and deserves its own
  decision. `backend/.env.example` and `backend/README.md` both say so. Do not
  wire it to anything on the assumption it was forgotten.
- **Deferred from Increment 6, on the record:** output-write failure injection
  (no portable way to induce it), the M1 PDF fixture corpus (no `.pdf` exists in
  the repo; that is CVR-040 in M4), and a multi-process restart test in Rust
  (the container smoke covers it against the real binary).
- The service remains loopback-only. No LAN, Caddy, AnyDoc, or Datalab
  fallback until their milestones.

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
The latest run is `20260818T195919Z` against image `sha256:7b765043…`: both
phases pass and the evidence includes the in-container AnyDoc docx conversion.

## Doc map

| File | Role |
|---|---|
| `README.md` | How to run, test, and release |
| `CLAUDE.md` | Desktop architecture invariants |
| `AGENTS.md` | Learned user preferences and workspace facts |
| `src/ui/UI.md` | Design language and the rules for extending it |
| `docs/BACKEND_EPIC.md` | Milestone tracker and CVR tickets |
| `docs/BACKEND_SERVICE_PLAN.md` | Approved architecture |
| `docs/BACKEND_EXECUTION_PLAN.md` | M2 increment checklist |
| `docs/DESKTOP_EXECUTION_PLAN.md` | M6 desktop integration plan and milestone ordering |
| `docs/MONITORING_AND_PROGRESS.md` | Job UX poll vs operator logs/metrics; not a ticket |
| `docs/BACKEND_BASELINE.md` | Scaffold ownership / staging paths |
| `docs/YAAK_ARCHITECTURE_REFERENCE.md` | Yaak patterns to steal, not fork |
| `backend/README.md` | Converter setup and env vars |
