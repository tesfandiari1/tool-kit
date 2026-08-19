# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

**Read [`docs/HANDOFF.md`](docs/HANDOFF.md) first.** It is the parallel-session
handoff: current branch, next increment, ownership boundaries, and dependency
pins. This file is the desktop architecture contract. Conversion-backend plans
live in `docs/`.

## What this is

Tool-Kit is a Tauri 2 + React/TypeScript **macOS-only** desktop app that consolidates the "turn a file into AI-ready text" tools the author uses daily into one window. You drop **files and/or folders** and hit Run. The job is picked from what you dropped and the output folder defaults to sit alongside the input, so the common path is drop → Run.

| Job | Service | In → Out |
|---|---|---|
| Convert | Datalab (`/api/v1/convert`, or a pinned pipeline) | PDF / DOCX / images / … → markdown |
| Transcribe | Rev.ai (async) | audio / video → text |

macOS is the only target: the bundle builds `app` + `dmg` only, and the app uses
`titleBarStyle: Overlay` + `underWindowBackground` vibrancy (which requires
`macOSPrivateApi`, ruling out Mac App Store submission. fine for a signed DMG).

## Commands

Run from the repo root.

```bash
pnpm tauri dev        # run the app with HMR (first run is slow: it compiles the Rust backend)
pnpm build            # frontend only: tsc + vite build
pnpm lint             # ESLint, type-aware (recommended + stylistic + react-hooks); kept clean
pnpm lint:fix

cargo check  --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
```

Clippy is the Rust linter. Both crates deny a small extra set in
`Cargo.toml` `[lints.clippy]` (todo/dbg/unwrap in non-test code, unused
async, non-exhaustive single-variant matches). Policy knobs live in each
crate’s `clippy.toml`. Do not enable `clippy::pedantic` as a group.

### Live API smoke tests

`src-tauri/src/live_smoke.rs` hits the real Datalab + Rev.ai endpoints. They are `#[ignore]`d (they spend API credits) and read keys from the environment. Sample inputs are generated under `/tmp/toolkit-test/` (override with `TEST_PDF` / `TEST_AUDIO`).

```bash
# both; keys may come from .env.local: note REV_AI_API_KEY must be exported as REVAI_API_KEY
DATALAB_API_KEY=… REVAI_API_KEY=… \
  cargo test --manifest-path src-tauri/Cargo.toml --lib live_smoke -- --ignored --nocapture

# a single test
… cargo test --manifest-path src-tauri/Cargo.toml --lib live_smoke::datalab_convert_live -- --ignored --nocapture
```

### Release / signing / notarization (macOS)

```bash
# sign-only (fast; OK for local installs: locally-built apps aren't quarantined, so Gatekeeper allows them)
APPLE_SIGNING_IDENTITY="Developer ID Application: … (92MA44797J)" pnpm tauri build

# sign + notarize + staple (for the distributable DMG): additionally export APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID
```

Artifacts: `src-tauri/target/release/bundle/{macos/Tool-Kit.app, dmg/*.dmg}`. The Developer ID cert needs Apple's **G2 intermediate** in the keychain (`certs.apple.com/devidg2.der`) or the signing identity reads as untrusted/invalid. Signing + Apple creds live in `.env.local` (gitignored).

## Architecture

**One pipeline, many tools.** Datalab, Rev.ai, and Claude are different services but share the same shape, so this is a single job queue with pluggable providers, not three separate integrations. The entire UI reduces to: inputs → output folder → `JobType` → Run.

**The backend does everything; the frontend is a thin shell.** All network calls, secrets, and file IO are in Rust. The frontend only calls Tauri commands and stays in sync by listening to `job-updated` events: there is no business logic in it. `src/shell/App.tsx` owns all state and renders one of the views in `src/domains/`; presentation comes from the `@ui` design system (`src/ui`).

### Rust (`src-tauri/src/`)

- **`providers.rs`**: the integration layer and the main extension point. Both services are `submit → poll → fetch` (`datalab_*`, `revai_*`). `send_retrying()` wraps submits with backoff, but **only retries failures that prove the server never started work** (429/502/503/504/529 and connect errors) so a retry can't double-bill; multipart bodies are rebuilt per attempt. Timeouts are per-request: `UPLOAD_TIMEOUT` (30 min, submits carry whole files) vs `POLL_TIMEOUT` (60s). Every poll checks the HTTP status through `terminal_poll_error()` before parsing, so a 401/404 fails the job instead of reading as "still pending". Results flow through `PollResult` (Pending/Done/Failed).
  - **`Err` vs `PollResult::Failed` is the whole transient/terminal distinction.** `Err` sends the caller's poll loop round again (it tolerates ~1 min of unbroken failure). `Failed` kills the job. The *result fetch* paths (the Rev.ai transcript and the Datalab pipeline step result) must follow the same 5xx/429-is-transient rule as the status poll. By then the work is finished and already billed, so treating a blip as terminal throws away a paid result and the only recovery is a full re-upload.
- **`jobs.rs`**: `JobType` maps a job to its provider, accepted extensions, and output extension. `JobManager` is the in-memory queue (`Mutex<Vec<Job>>` + `AtomicU64` ids) with a `Semaphore` capping **4 concurrent jobs** to respect upstream rate limits. `run_job()` spawns the per-file lifecycle on `tauri::async_runtime` and emits a `job-updated` event on every state change; jobs poll every 5s (~60 min cap, with a consecutive-error bail).
  - **Generation counter.** `JobManager::generation` is bumped by every new run and by Stop. Each spawned task captures it and aborts as soon as it stops matching, which is what makes Stop work and stops a second run's tasks from spending credits and writing files invisibly. **Every write-back to a job needs a `stale()` check in front of it**, not just the expensive steps: a submit can run for 30 minutes, so a finished upload that writes "processing" over a row the user already stopped leaves a job no task will ever finish, and `running` (derived from the job list) sticks on.
  - **Run config snapshot.** Settings are read once at run start into `JobManager::run_config`, so changing Settings mid-run can't split one run across two output formats.
  - `write_output()` returns `Result` and uses `create_new`, so a failed or colliding write **fails the job** instead of reporting a green success with nothing on disk.
- **`history.rs`**: persistent run history in **SQLite** (`rusqlite`, bundled), one `history.db` beside `settings.json`. Owns *all* SQL. Nothing else opens the database. Two jobs: remember where every result went, and answer "already done?" on every input scan through an indexed lookup by `source_path`, which is why it is a database and not a log file. **A storage error never fails a job**: `with_db` swallows everything, and a database that will not open degrades to "no history". A result is reusable only when the source path, job, *and* output format match, the recorded output still exists, and the source mtime is unchanged. The lookup is path-keyed rather than content-hashed, because hashing means reading every byte of every file on every scan.

  **One rule governs a run: never do the same work twice.** A reusable result already in the chosen output folder means nothing happens; one in a *different* folder is **copied** (`jobs::reuse_result`), because the source is unchanged and the format matches, so the bytes a re-run would buy are bytes already on disk; anything else goes to the provider. That is why `Scan` reports `already_here_*` and `reusable_*` separately: the first is "nothing to do", the second is free work the Run button must stay enabled for, and it relabels itself "Copy N results" when that is all that is left. Retention is capped at 5,000 rows; `PRAGMA user_version` makes a new column a migration rather than a crash.
- **`secrets.rs`**: API keys in the **macOS Keychain** via `keyring` (service `ai.uniwise.toolkit`, accounts `datalab`/`revai`). Keys never reach the webview or disk.
- **`settings.rs`**: non-secret config (input paths, output dir, job, Datalab format, optional pipeline id, high-accuracy toggle) persisted as JSON in the app config dir. **`#[serde(default)]` on the struct is load-bearing**: without it, adding a field makes every existing `settings.json` fail to parse, and `load()` swallows the error and silently resets the user's output folder and job. `save()` writes to a temp file and renames for the same reason: `load()` answers a torn or truncated file with `unwrap_or_default()`, so a crash mid-write would wipe the user's config.
- **`lib.rs`**: the Tauri command surface + `setup()` (menu-bar tray, ⌥⌘V global shortcut). `run_pipeline` expands the selected files/folders into a concrete file list (`collect_input_files`), clears the queue, and spawns one job per match. Commands return `Result<T, String>`.

### Adding a provider / job

Add a `JobType` variant and a `ProviderKind`, implement submit/poll in `providers.rs`, and extend `JobType::accepts` plus the `Scan` struct in `lib.rs` so the new job takes part in autodetect. The UI, queue, events, and output handling are reused unchanged.

### Frontend (`src/`)

`src/shell/App.tsx` owns all state and hands it to view components in `src/domains/{run,history,settings,thread}`. The shell's non-run concerns are extracted as hooks beside it: `useToast` (the one channel for errors that never reach a job row), `useThread` (opening a result from a job row or a history entry, and the return path Escape uses), and `useHostWindow` (`useDragDrop`, `useWindowFocusClass`, `useCloseConfirm`). Local state mirrors `Settings` and is persisted via `save_settings` on change; the job list is authoritative from `job-updated` events. Inputs are a list of paths added by drag-drop (`getCurrentWebview().onDragDropEvent`) or the Files/Folder pickers.

`src/platform/host.ts` is the only module allowed to import `@tauri-apps/*` for dialogs, window, and drag-drop. `src/app/commands.ts` is the typed IPC boundary.

`src/app/api/` is the conversion-service client layer, built on the OpenAPI contract rather than custom wrappers: `schema.ts` is generated from `backend/openapi/openapi.yaml` by `pnpm generate:api` (ESLint blocks importing it from anywhere else), and `client.ts` is a standard `openapi-fetch` client typed by that schema. `transport.ts` plugs a Tauri-backed `fetch` into openapi-fetch's documented seam: requests go through one `service_request` command so the host can attach the Keychain token and stream multipart sources from disk (the Rust handler lands with M6). Call sites use the library's `{ data, error }` results; errors carry the contract's ErrorEnvelope.

- **Autodetect.** `scan_inputs` returns a per-job match count, a per-job already-done count, and a suggested output folder. An effect picks the job with more matches and defaults the output folder. It depends on **`scan.convert` / `scan.transcribe` / `scan.suggestedOutput`, never the whole `scan` object and never `settings.jobType`**: either would re-fire the effect on an unrelated refresh and undo a manual job click instantly. That is also why both jobs' already-done counts come back in one scan: switching job reads a number that is already in hand rather than triggering a new scan.
- **Already done.** The Run button promises `inputCount - skipping`, not the raw match count, and `run_pipeline` does the authoritative filtering at run time from the same `Settings` load it snapshots as the run config. `runsFinished` is bumped **once** when a run ends, to refresh the counts and an open History panel: a 200-file run emits hundreds of `job-updated` events, so reacting to those would re-scan the disk hundreds of times.
- **`starting` guards double-runs.** `running` is derived from the job list, which stays empty until the first event lands, so without the guard a double-click fires two runs.
- Inputs auto-clear after a run only when that run was started by `run()` *and* something succeeded, so a Retry or a wholly failed run leaves the selection alone.

### Design system (`src/ui`, docs in `src/ui/UI.md`)

The design language lives in a self-contained library imported as `@ui`. **Read `src/ui/UI.md` before touching any UI.** Its three rules are the reason to reject a change:

1. **Colour is signal, never decoration.** Amber = live, green = passed, red = failed, cobalt = the control you press. Icons, the brand mark, and folder glyphs are never coloured.
2. **Values light up, they don't appear.** Counts and timers hold their slot as dim ghost glyphs so the window never reflows while jobs finish out of order.
3. **macOS first.** Real vibrancy under a scrim, SF metrics, key/inactive window states (`body.inactive`), HIG focus rings, tabular numerals, and native `<select>`/`<input type=checkbox>` under restyled shells.

Type is three families with non-overlapping jobs: **Instrument Serif** for display, **SF Pro** for prose, **JetBrains Mono** for every label, tab, and button (uppercase, `0.09em` tracked. the signature of the language). Both faces are self-hosted OFL; see `src/ui/fonts/THIRD_PARTY_NOTICES.md`.

- **The `@ui` boundary is enforced by ESLint.** Nothing under `src/ui` may import from `@/app`, `@/domains`, `@/platform`, or `@tauri-apps/*`. When a primitive needs something from the product it takes it as a prop: `RunView` maps its five job statuses onto three `Status` tones in one function rather than teaching the library about jobs.
- **A primitive with no specimen in `src/ui/gallery/Gallery.tsx` does not exist.** Add it in the same commit. Review it at `pnpm dev` → `http://localhost:1420/?gallery`, which runs with no Tauri bridge and has a Graphite/Paper theme toggle.
- **`App.tsx` must import `@ui` before `./App.css`.** `App.css` deliberately does not `@import` the token layer: doing it in both places shipped the whole 16KB twice, in two chunks with competing `:root` blocks.
- **`src/shell/App.css` is app composites only** (title bar, drop well, folder picker, job row layout, markdown prose). Its `:root` block is a shrinking bridge of legacy alias names; do not add to it.

## Gotchas

- **Keychain prompts in dev are fixed by code identity, not by fewer reads.** macOS authorises a keychain item against the reader's *designated requirement*, and an ad-hoc `cargo run` binary's requirement is its own cdhash, so every rebuild used to arrive as a new app and re-prompt. `src-tauri/.cargo/config.toml` sends `cargo run` through `scripts/dev-run.sh`, which signs the binary with the Developer ID and `--identifier ai.uniwise.toolkit`: the same requirement the installed app has, so dev inherits its already-trusted items and an "Always Allow" survives rebuilds. Every failure path still runs the binary, as cargo left it, and says why on stderr: a dev loop that died over an expired certificate would be worse than the prompts. `secrets.rs` memoises each read for the life of the process, which is what keeps a 200-file run from asking 200 times. Still let the app create the items: don't seed them with the `security` CLI.
- **Capabilities.** Custom Tauri commands need no capability entries, but **core commands do**: `core:window:default` does *not* include `hide`/`destroy`, so the close-confirm handler needs them listed explicitly in `capabilities/default.json`. A missing one fails silently at runtime.
- **Icons.** The app icon is the original artwork. `icons/tray.png` / `tray@2x.png` are derived from its alpha channel as **template images** (black + alpha, used with `icon_as_template(true)`) so macOS tints them for light and dark menu bars: putting the colour app icon in the tray is a visible native-correctness bug.
- **Global shortcut is ⌥⌘V**, not ⌘⇧V: that one is macOS "Paste and Match Style" and registering it hijacks the combination system-wide.
- **Datalab pipeline mode**: a `pl_…` id in Settings switches Convert from `/api/v1/convert` to `/api/v1/pipelines/{id}/run` (run → poll execution → fetch the last step's result).
- **pnpm 11** gates package build scripts: esbuild is approved via `allowBuilds: { esbuild: true }` in `pnpm-workspace.yaml`.
- **TypeScript stays on 6.x.** 7.0 has no compiler API; `typescript-eslint` crashes. `tsconfig.json` must not set `baseUrl` (deprecated in 6).
- **`keyring` stays on 3.x** with `apple-native`. 4.x dropped that feature and needs a `keyring-core` rewrite.
- **Do not create a root Cargo workspace.** `src-tauri` and `backend` keep separate lockfiles until a dedicated migration.
- **Never `git add -A`.** Desktop and backend work share one dirty tree; stage explicit paths. See `docs/BACKEND_BASELINE.md` and `docs/HANDOFF.md`.

## Reference

- [`docs/HANDOFF.md`](docs/HANDOFF.md): current work and parallel-session rules
- [`docs/BACKEND_EPIC.md`](docs/BACKEND_EPIC.md): conversion-backend milestones
- [`docs/BACKEND_EXECUTION_PLAN.md`](docs/BACKEND_EXECUTION_PLAN.md): M2 increments
- [`docs/DESKTOP_EXECUTION_PLAN.md`](docs/DESKTOP_EXECUTION_PLAN.md): M6 desktop integration and the order of the remaining milestones
- [`docs/MONITORING_AND_PROGRESS.md`](docs/MONITORING_AND_PROGRESS.md): job UX poll vs operator logs/metrics
- [`docs/WORKSPACE_HANDOFF.md`](docs/WORKSPACE_HANDOFF.md): desktop dual-pane workspace (closed; not backend M7)
- [`.impeccable.md`](.impeccable.md): design context (users, brand, aesthetic direction, principles); every `/impeccable` skill reads it
- [`docs/YAAK_ARCHITECTURE_REFERENCE.md`](docs/YAAK_ARCHITECTURE_REFERENCE.md): Yaak files to steal (host, RPC, blobs, Keychain); do not fork the product
- Local Tauri v2 docs: `/Users/tristin/code/tauri-skills/knowledgebase/tauri-v2`.

<!-- gitnexus:start -->
# GitNexus: code intelligence

This project is indexed by GitNexus as **tool-kit** (1912 symbols, 5507 relationships, 163 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> Index stale? Run `node .gitnexus/run.cjs analyze` from the project root. It auto-selects an available runner. No `.gitnexus/run.cjs` yet? `npx gitnexus analyze` (npm 11 crash → `npm i -g gitnexus`; #1939).

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows. For regression review, compare against the default branch: `detect_changes({scope: "compare", base_ref: "main"})`.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({search_query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol (callers, callees, and which execution flows it participates in), use `context({name: "symbolName"})`.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace. Use `rename`, which understands the call graph.
- NEVER commit changes without running `detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/tool-kit/context` | Codebase overview, check index freshness |
| `gitnexus://repo/tool-kit/clusters` | All functional areas |
| `gitnexus://repo/tool-kit/processes` | All execution flows |
| `gitnexus://repo/tool-kit/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
