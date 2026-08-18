# Session handoff

**Last updated:** 2026-08-17  
**Branch:** `main`. `codex/backend-m2` merged in PR #1 and PR #3 and was
deleted. Branch fresh off `main` for the next increment.  
**Backend checkpoint:** `449d7cb` — M2 Increments 0-4 verified. Increment 5 is
implemented and gate-green on top of `a50ca54`, with no checkpoint SHA yet.
**Recorded code state:** Increment 5 backend and doc changes are uncommitted.
Review and stage explicit paths under `backend/` and `docs/`.
**Read this first** in any parallel session, then re-read the live worktree.
This file goes stale the moment someone lands a commit.

## Do this next

1. **Implement M2 Increment 6, the failure and release gates.** The full
   checklist is in `docs/BACKEND_EXECUTION_PLAN.md`. It is the last unit before
   M2 closes: deterministic crash barriers, restart and replay tests, corrupt
   and missing source/artifact cases, an image build, and a real submit ->
   stop -> start -> poll -> download sequence plus a `SIGKILL` recovery smoke.
2. **Increment 5 is done and needs review, not redoing.** Readiness probes
   SQLite, the data root, and the runner; capabilities report
   `durability: "persistent"` and `maxActiveJobs`; OpenAPI and `Cargo.toml` are
   both `0.3.0`; Compose mounts the named volume `converter-data` at `/data`.
   The TypeScript delta was generated and diffed under `/private/tmp` only, so
   `src/app/api/schema.ts` is still byte-identical and belongs to M6.

   **Two latent breaks were fixed there; do not reintroduce them.**
   - The Dockerfile builder copied only `Cargo.toml Cargo.lock src`, so
     `sqlx::migrate!("./migrations")` had no SQL to read at compile time and the
     image had not built since `e8e8ca5` introduced that macro. `migrations/`
     and `build.rs` are now build inputs.
   - Compose never mounted `/data`, so every container restart lost the
     database and every artifact while the contract claimed durability.

   Both fixes were checked against a real build, not read off the Dockerfile.
   `docker build -t tool-kit-converter:m2 backend` succeeds, and `ls -ldn /data`
   inside that image reports mode `drwx------` owned by `10001:10001`.

   **Gotcha: a fresh named volume takes the ownership of the image directory it
   covers, and falls back to `root:root` when the image has no such directory.**
   The service runs as `10001:10001`, so the runtime stage must keep
   `RUN install -d -o 10001 -g 10001 -m 0700 /data` or the first
   `docker compose up` cannot write. No `VOLUME` instruction: that hands plain
   `docker run` an anonymous volume nobody prunes.

   **One reviewed finding was left open on purpose.** `/health/ready` is
   unauthenticated and now opens a `BEGIN IMMEDIATE` transaction against a
   four-connection pool on every request, so a local process can flood it and
   contend the SQLite write lock with the job runner. Per-request work is
   bounded at two seconds, aggregate concurrency is not. It was not fixed
   because neither obvious fix is free: caching a passing result reintroduces
   the stale readiness this increment removed, and a concurrency cap changes
   latency semantics under load. Exposure is loopback-only today. Give it a
   real rate limit when Caddy and LAN exposure land, where that belongs.
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
4. **Everything through Increment 4 is merged to `main` and green.** PR #1
   (desktop restructure, CI, M1/M2 durability) and PR #3 (Increment 4 startup
   reconciliation) are merged. `main` is at `a50ca54`, a docs-only commit on top
   of the PR #3 merge `1049b96`, with CI passing on the push trigger. Increment
   5 is the only uncommitted backend work. Re-check live status before editing
   and keep staging to explicit owned paths.

   **CI is the only gate that sees Linux-only code.** PR #1 failed on a
   `needless_return` inside `#[cfg(target_os = "linux")]` in
   `backend/src/bin/tool-kit-pdf-worker.rs`, which macOS never compiles and so
   can never lint. Cross-compiling locally does not help: `libsqlite3-sys`
   needs a Linux C toolchain. Treat a local backend pass as necessary but not
   sufficient, and open a PR so the ubuntu job runs before merging.
5. **Do not** create a root Cargo workspace, bump TypeScript 7, migrate
   `keyring` 4, unpin `pdf-inspector`, or take `libc` 1.0 (still alpha).

## Parallel session ownership

| Session | May edit | Must not edit |
|---|---|---|
| Backend / M2 | `backend/**`, `docs/BACKEND_*.md`, this file | `src/**`, `src-tauri/**` |
| Desktop | `src/**`, `src-tauri/**` | `backend/**` persistence/worker/OpenAPI |
| Shared docs | `README.md`, `CLAUDE.md` — append or reconcile | Reverting the other session's section |

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
  shell/      App.tsx, App.css, useToast, useThread, useHostWindow
  ui/         design system, imported as @ui, depends on nothing of ours
  main.tsx
```

What the next session needs to know:

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
- Backend M1 (authenticated loopback PDF slice) is complete and committed.
- M2 Increment 0 (contract freeze) is complete on this branch (`433b6c3`,
  later `d8445a5`).
- M2 Increments 1-2 are complete: SQLite is the live job/idempotency store,
  sources and artifacts use the persistent data root, submissions are durable
  before `202`, and completed jobs/artifacts survive `AppState` restart.
- M2 Increment 3 is complete and verified: one FIFO
  runner claims from SQLite, wakes through `Notify`, polls as a fallback, uses
  an exact validated source handle, verifies source length/SHA-256 again inside
  the worker, fails closed on durable-state uncertainty, and performs bounded
  cancellation/shutdown under the HTTP supervisor's same deadline. The settled
  gate passed format, check, Clippy, `git diff --check`, and 77 backend tests;
  the backend-only checkpoint is `f189265`.
- M2 Increment 4 is complete and verified: startup reconciliation runs before
  the worker, preserves interrupted attempt history, bounds fresh attempts,
  validates sources and exact published bundles, fails corrupt historical
  successes closed while retaining audit data, and quarantines only canonical
  unowned job trees. The settled gate passed format, check, Clippy,
  `git diff --check`, and 90 backend tests (54 library, 1 server, 3 worker, 32
  HTTP contract) on the default test stack. The checkpoint is `449d7cb`.
- M2 Increment 5 is implemented and gate-green, uncommitted. `/health/ready`
  runs three concurrent two-second-bounded checks (SQLite write transaction, a
  create-and-remove probe file under `<data root>/.health/`, runner
  failed/stopped) and answers `503` with per-check detail when any fails, while
  `/health/live` stays dependency-free. Capabilities report persistent
  durability and `maxActiveJobs`. OpenAPI and `backend/Cargo.toml` are both
  `0.3.0`, and a contract test now pins `info.version` to `CARGO_PKG_VERSION`.
  Compose mounts `converter-data` at `/data` and mirrors the image healthcheck
  timing. The gate passed format, check, Clippy with warnings denied,
  `git diff --check`, `docker compose config`, `docker build`, and 96 backend
  tests (56 library, 1 server, 3 worker, 36 HTTP contract).
- **`TOOLKIT_CONVERTER_SCRATCH_PARENT` is dead config.** `config.rs` parses and
  validates it, and nothing else reads it. Increment 5 left it in place on
  purpose, because removing an environment variable changes the public surface
  and deserves its own decision. `backend/.env.example` and `backend/README.md`
  both say so. Do not wire it to anything on the assumption it was forgotten.
- Increment 6 built-container restart gates remain pending.
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
# then pin TypeScript back to 6.x — 7 breaks typescript-eslint
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

## Verify

```bash
pnpm check           # tsc --noEmit + eslint + vitest         (fast inner loop)
pnpm verify          # check + build + src-tauri clippy/tests (desktop scope)
pnpm verify:backend  # converter clippy/tests
pnpm verify:all      # both
```

CI (`.github/workflows/ci.yml`) runs frontend, backend, and desktop as three
jobs on push/PR. Desktop clippy/tests
use `macos-latest` because of `macos-private-api` and `keyring`.

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
| `docs/BACKEND_BASELINE.md` | Scaffold ownership / staging paths |
| `docs/YAAK_ARCHITECTURE_REFERENCE.md` | Yaak patterns to steal, not fork |
| `backend/README.md` | Converter setup and env vars |
