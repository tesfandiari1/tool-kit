# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

**Read [`docs/STATUS.md`](docs/STATUS.md) first.** It is the single source of
truth for session state, the critical path, milestones, traps, and verify
commands. This file is the desktop architecture contract.

## What this is

Tool-Kit is a Tauri 2 + React/TypeScript **macOS-only** desktop app that consolidates the "turn a file into AI-ready text" tools the author uses daily into one window. You drop **files and/or folders** and hit Run. The job is picked from what you dropped and the output folder defaults to sit alongside the input, so the common path is drop → Run.

| Job | Service | In → Out |
|---|---|---|
| Convert | Datalab (`/api/v1/convert`, or a pinned pipeline) | PDF / DOCX / images / … → markdown |
| Transcribe | Rev.ai (async) | audio / video → text |

macOS is the only target: the bundle builds `app` + `dmg` only, and the app uses
`titleBarStyle: Overlay` + `underWindowBackground` vibrancy (which requires
`macOSPrivateApi`, ruling out Mac App Store submission. fine for a signed DMG).

## Repository layout

Paths in this file are repo-relative. Inside the Frontend and Design system
sections below, a bare `src/...` means `apps/desktop/src/...`.

```text
apps/
  desktop/                  the .app: frontend + Tauri host
    src/                    app/ domains/ platform/ shell/ ui/
    src-tauri/              name kept: tauri.conf.json, .cargo/config.toml, and
                            dev-run.sh all resolve against this exact depth
      binaries/             staged sidecars, gitignored. `pnpm sidecars` fills it
      resources/            staged pdf-inspector bcmaps, gitignored, same script
  converter/                the HTTP service, container only
crates/
  worker-protocol/          wire contracts for every spawned worker
workers/
  vision/                   Swift, macOS 26+, cannot ship in the Linux image
contract/http/              OpenAPI spec, Spectral ruleset, Schemathesis config
deploy/docker/              Dockerfile, compose.yaml, secrets/
docs/
```

The build context for the image is the **repo root**, not `apps/converter`,
because the converter has a path dependency on `crates/worker-protocol` and a
context cannot reach outside itself. The root `.dockerignore` denies everything
and adds back only what the Dockerfile copies, which keeps the context under a
megabyte against a tree that carries gigabytes of `target/`.

`src/ui` is not a package. It is written so it could become one, and `UI.md`
names the trigger: the day a second client appears. Until then the boundary is
an ESLint rule, and that rule is load-bearing rather than decorative.

## Commands

Run from the repo root.

```bash
pnpm sidecars         # build + stage the three bundled binaries and the bcmaps. See below
pnpm tauri dev        # run the app with HMR (first run is slow: it compiles the Rust backend)
pnpm build            # frontend only: tsc + vite build
pnpm lint             # ESLint, type-aware (recommended + stylistic + react-hooks); kept clean
pnpm lint:fix

cargo check  --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path apps/converter/Cargo.toml --all-targets -- -D warnings
```

**`pnpm sidecars` is a prerequisite for every one of those cargo commands**,
not a release step. `tauri-build` resolves `bundle.externalBin` at build-script
time, so a missing binary fails `cargo check`, `cargo clippy`, `cargo test` and
`pnpm tauri dev` alike with `ResourcePathNotFound`, long before anything tries
to bundle. `pnpm tauri` and `pnpm verify` run it for you. A bare `cargo` command
does not, which is the whole reason it is named here. The staged artifacts are
gitignored, so a fresh clone needs one run before the desktop crate compiles.

Clippy is the Rust linter. Both crates deny a small extra set in
`Cargo.toml` `[lints.clippy]` (todo/dbg/unwrap in non-test code, unused
async, non-exhaustive single-variant matches). Policy knobs live in each
crate’s `clippy.toml`. Do not enable `clippy::pedantic` as a group.

### Suites written outside this repo

```bash
pnpm lint:api         # Spectral over contract/http/openapi.yaml (also in verify:backend)
pnpm verify:api-drift # schema.ts still matches the contract
pnpm verify:deps      # cargo-deny: RustSec advisories, licenses, sources
pnpm verify:contract  # Schemathesis against a live converter, needs uv
```

These three grade work whose assertions nobody here wrote, which is the one
thing the rest of the tree cannot do: every other suite was written by whoever
wrote the code under it. Schemathesis found three undocumented admission rules
on its first run.

- **`contract/http/.spectral.yaml` no longer overrides `array-items`.** It
  used to, because `inputFormats` and `engines` were fixed-length tuples that
  `prefixItems` plus a matching `maxItems` described exactly, and `items` would
  have described nothing. The Vision engine ended that: both arrays now vary in
  length with whether it initialized, 18 media types without it and 24 with it,
  so they are variable-length arrays and `items` is the honest description.
  Do not re-add `prefixItems` to either one.
- **`contract/http/schemathesis.toml`.** `positive_data_acceptance` is off
  for `createConversion` alone. Admission checks the container signature of the
  uploaded bytes, and generated bytes never carry one, so the check is
  unsatisfiable there and says nothing anywhere else.
- **`apps/converter/scripts/contract-fuzz.sh` runs the suite twice**, with a seeded
  conversion id and then with a UUID no conversion has. One pass is a hole
  either way and the hole is silent: pinning a real id turned a removed 404
  declaration into a green run while this was being built.
- **Both `deny.toml` files pin their platforms.** `src-tauri` names macOS only,
  which drops Tauri's GTK3 stack and its ten unmaintained advisories from the
  graph rather than ignoring them by id on a platform we ship. Each remaining
  exception names its crate and its reason, so a new unmaintained dependency
  arrives as a failure rather than as noise.

### Live API smoke tests

`apps/desktop/src-tauri/src/live_smoke.rs` hits the real Datalab + Rev.ai endpoints. They are `#[ignore]`d (they spend API credits) and read keys from the environment. Sample inputs are generated under `/tmp/toolkit-test/` (override with `TEST_PDF` / `TEST_AUDIO`).

```bash
# both; keys may come from .env.local: note REV_AI_API_KEY must be exported as REVAI_API_KEY
DATALAB_API_KEY=… REVAI_API_KEY=… \
  cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib live_smoke -- --ignored --nocapture

# a single test
… cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib live_smoke::datalab_convert_live -- --ignored --nocapture
```

### Release / signing / notarization (macOS)

```bash
# sign-only (fast; OK for local installs: locally-built apps aren't quarantined, so Gatekeeper allows them)
APPLE_SIGNING_IDENTITY="Developer ID Application: … (92MA44797J)" pnpm tauri build

# sign + notarize + staple (for the distributable DMG): additionally export APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID
```

Artifacts: `apps/desktop/src-tauri/target/release/bundle/{macos/Tool-Kit.app, dmg/*.dmg}`. The Developer ID cert needs Apple's **G2 intermediate** in the keychain (`certs.apple.com/devidg2.der`) or the signing identity reads as untrusted/invalid. Signing + Apple creds live in `.env.local` (gitignored).

Two files make the keychain work and both are inputs to the signature:
`apps/desktop/src-tauri/entitlements.plist` (wired through `bundle.macOS.entitlements`) and
`apps/desktop/src-tauri/embedded.provisionprofile` (through `bundle.macOS.files`, which the
bundler copies into `Contents/` *before* it signs, so no post-build re-sign is
needed). The profile comes from the portal: an explicit App ID for
`dev.esfandiari.toolkit` with **Keychain Sharing** enabled, then Profiles → +
→ **Developer ID**. It is committed, because it ships inside every copy of the
app and is public by construction.

**Both are parked right now.** `bundle.macOS` carries only
`minimumSystemVersion`, because `files` hard-errors when the profile is absent
and that would block every release build until the portal step happens. The app
therefore signs without the entitlement and `secrets.rs` falls back to the
legacy keychain. `verify-release.sh` fails at its **Keychain entitlement**
section on such a build, which is accurate, not a bug. Restore both keys the
moment `apps/desktop/src-tauri/embedded.provisionprofile` exists:

```json
"macOS": {
  "minimumSystemVersion": "26.0",
  "entitlements": "entitlements.plist",
  "files": { "embedded.provisionprofile": "embedded.provisionprofile" }
}
```

**The bundler signs each sidecar on its own, and that is now measured.** A
signed build on 2026-08-20 stamped all three helpers with
`flags=0x10000(runtime)`, team `92MA44797J` and a secure timestamp, inner-out:
converter, PDF worker, Vision worker, `tool-kit`, then the `.app`. It replaces
the `adhoc,linker-signed` signature cargo and swiftc leave behind. Apple
notarized the result, reported `Ready for distribution` with no issues, and the
converter then started, resolved both workers and advertised its 24 media types
out of a quarantined copy of the installed app. So `build-sidecars.sh` is right
to do no signing, and a post-build re-sign would only break the nested seal.

**Restoring `entitlements` may hand entitlements.plist to all three sidecars.**
tauri-cli 2.11.4 builds one codesign argument vector per sign target and
appends `--entitlements` to every one of them, with no test for whether the
target is the outer bundle. A gate for exactly this landed in `9a30bed98` and
was removed again in `cc8c0b531`, so the behaviour is version-dependent and
worth re-reading on any CLI bump. That is read from the bundler source, not
measured here, because the key is parked and there was nothing to measure. If it holds,
each helper claims `com.apple.application-identifier` and the restricted
`keychain-access-groups` on a Mach-O with nowhere to carry a profile. Run
`codesign -d --entitlements - --xml` against each helper on the first build
after the key comes back, and run it before notarizing, because the notary is
the expensive place to learn this.

**Always check the bundle afterwards.** `pnpm tauri build` succeeds without a
signing identity and produces a working `.app`, so nothing fails at build time:

```bash
apps/desktop/src-tauri/scripts/verify-release.sh              # after a sign-only build
apps/desktop/src-tauri/scripts/verify-release.sh --notarized  # adds stapler + Gatekeeper
```

**`pnpm tauri build` notarizes the `.app` and not the DMG.** It staples the app,
then bundles it and merely *signs* the DMG, so a downloaded DMG is still
`rejected / source=Unnotarized Developer ID` and macOS refuses to open it. The
app inside is fine, which is what makes this easy to miss: local installs work
and only downloads fail. Finish the DMG by hand:

```bash
xcrun notarytool submit <dmg> --apple-id … --password … --team-id … --wait
xcrun stapler staple <dmg>
```

Prove it the way a user gets it, since an un-quarantined file is not a test:

```bash
xattr -w com.apple.quarantine "0083;$(printf %x $(date +%s));Safari;" copy.dmg
spctl -a -t open --context context:primary-signature -vv copy.dmg
```

**A failed DMG step leaves a mounted image that breaks every later build.**
`bundle_dmg.sh` attaches a read-write `rw.<pid>.*.dmg` under `bundle/macos/`
before laying out the window. If it dies there the volume stays attached, and
the next build fails with nothing but `failed to run bundle_dmg.sh`. Clear it:

```bash
hdiutil detach /Volumes/dmg.* -force
rm -f apps/desktop/src-tauri/target/release/bundle/macos/rw.*.dmg
```

It checks the designated requirement first, then `codesign --verify`, the four
Info.plist keys, the bundled font notices, and that no gallery chunk shipped.
The requirement check is the one that earns the script: see the Gotcha below.

## Architecture

**Two apps, one repo.** `apps/desktop/` is the Tauri app (its frontend at the
root of that directory, its Rust crate one level below at
`apps/desktop/src-tauri/`), and `apps/converter/` is the conversion service.
`crates/worker-protocol/` is shared by both, `workers/vision/` is the macOS
Vision worker, `contract/http/` is the OpenAPI contract, `deploy/docker/` ships
the converter. Paths below are relative to the app named in each section
header.

**One pipeline, many tools.** Datalab, Rev.ai, and Claude are different services but share the same shape, so this is a single job queue with pluggable providers, not three separate integrations. The entire UI reduces to: inputs → output folder → `JobType` → Run.

**The backend does everything; the frontend is a thin shell.** All network calls, secrets, and file IO are in Rust. The frontend only calls Tauri commands and stays in sync by listening to `job-updated` events: there is no business logic in it. `src/shell/App.tsx` owns all state and renders one of the views in `src/domains/`; presentation comes from the `@ui` design system (`src/ui`).

**The converter ships inside the app.** `bundle.externalBin` puts three binaries
in `Contents/MacOS/` beside `tool-kit`, and `bundle.resources` puts the
pdf-inspector bcmaps in `Contents/Resources/pdf-inspector/bcmaps`:

```text
Tool-Kit.app/Contents/
  MacOS/       tool-kit  tool-kit-converter  tool-kit-pdf-worker  tool-kit-vision-worker
  Resources/   pdf-inspector/bcmaps/  (169 files)
```

That layout is not a convention, it is the contract. `config.rs` in the
converter finds both of its workers through `current_exe().with_file_name(…)`,
so putting them side by side is what makes the sibling probe resolve, in
`target/debug` and in the bundle alike, with no env var and no branch.
`backend_host.rs` reaches the converter the same way.

`backend_host::Deployment` says where the service is, and **it is not a
setting**. **Sidecar** is what the app does: it spawns the converter on
`127.0.0.1:0`, reads the real port off the service's own listening line, mints a
Keychain token for it, and stops it again on exit. **Manual** happens only when
a deployment has dropped `backend-override.json` beside `settings.json`, which
is how the Docker deployment is reached and which nothing in the app writes.
`pnpm backend:docker`, `pnpm backend:sidecar` and `pnpm backend:where` are that
deployment act. Every read of the backend origin and token goes through
`backend_host::backend_origin` and `backend_host::backend_token`, so the two
cannot drift apart.

**Why a file and not a preference.** The service ships in the bundle, so
running one elsewhere is a deployment decision, not a taste. Leaving it in
Settings meant a URL field that did nothing in the mode everyone was actually
in: `backend_origin` returned the sidecar's port and never read
`settings.backend_url`, so the box accepted a URL, saved it, and changed
nothing. A file nobody surfaces cannot be reached by a stray click, and a
malformed one is an error rather than a silent fall back to Sidecar, because
converting on this Mac while the container someone deployed sits idle is the
one outcome they did not ask for.

The sidecar advertises **more** than the container can. `workers/vision/` is
Swift against Apple's Vision framework, so it cannot ship in the Linux image:
`capabilities.inputFormats` is 24 media types in the app and 18 in Docker, and
images convert locally only in Sidecar mode.

### Rust (`apps/desktop/src-tauri/src/`)

- **`providers.rs`**: the integration layer and the main extension point. Both services are `submit → poll → fetch` (`datalab_*`, `revai_*`). `send_retrying()` wraps submits with backoff, but **only retries failures that prove the server never started work** (429/502/503/504/529 and connect errors) so a retry can't double-bill; multipart bodies are rebuilt per attempt. Timeouts are per-request: `UPLOAD_TIMEOUT` (30 min, submits carry whole files) vs `POLL_TIMEOUT` (60s). Every poll checks the HTTP status through `terminal_poll_error()` before parsing, so a 401/404 fails the job instead of reading as "still pending". Results flow through `PollResult` (Pending/Done/Failed).
  - **`Err` vs `PollResult::Failed` is the whole transient/terminal distinction.** `Err` sends the caller's poll loop round again (it tolerates ~1 min of unbroken failure). `Failed` kills the job. The *result fetch* paths (the Rev.ai transcript and the Datalab pipeline step result) must follow the same 5xx/429-is-transient rule as the status poll. By then the work is finished and already billed, so treating a blip as terminal throws away a paid result and the only recovery is a full re-upload.
- **`jobs.rs`**: `JobType` maps a job to its provider, accepted extensions, and output extension. `output_extension_for(jt, cfg)` is that last rule in one place, read by both writers and by the tree's pairing test, so "a sibling `.md` means already converted" cannot disagree with what a run writes. It reads `datalab_format` only. `output_format_for` folds a `pl_…` pipeline id into the history key, and an extension has no room for one. `JobManager` is the in-memory queue (`Mutex<Vec<Job>>` + `AtomicU64` ids) with a `Semaphore` capping **4 concurrent jobs** to respect upstream rate limits. `run_job()` spawns the per-file lifecycle on `tauri::async_runtime` and emits a `job-updated` event on every state change; jobs poll every 5s (~60 min cap, with a consecutive-error bail).
  - **Generation counter.** `JobManager::generation` is bumped by every new run and by Stop. Each spawned task captures it and aborts as soon as it stops matching, which is what makes Stop work and stops a second run's tasks from spending credits and writing files invisibly. **Every write-back to a job needs a `stale()` check in front of it**, not just the expensive steps: a submit can run for 30 minutes, so a finished upload that writes "processing" over a row the user already stopped leaves a job no task will ever finish, and `running` (derived from the job list) sticks on.
  - **Run config snapshot.** Settings are read once at run start into `JobManager::run_config`, so changing Settings mid-run can't split one run across two output formats.
  - `write_output()` returns `Result` and uses `create_new`, so a failed or colliding write **fails the job** instead of reporting a green success with nothing on disk.
- **`history.rs`**: persistent run history in **SQLite** (`rusqlite`, bundled), one `history.db` beside `settings.json`. Owns every statement that touches it, and nothing else opens it. `workspace.rs` owns the other database, `.toolkit/index.db` inside the workspace, and the two never meet: one is app state, the other is derived state a user may delete. Two jobs: remember where every result went, and answer "already done?" on every input scan through an indexed lookup by `source_path`, which is why it is a database and not a log file. **A storage error never fails a job**: `with_db` swallows everything, and a database that will not open degrades to "no history". A result is reusable only when the source path, job, *and* output format match, the recorded output still exists, and the source mtime is unchanged. The lookup is path-keyed rather than content-hashed, because hashing means reading every byte of every file on every scan.

  **One rule governs a run: never do the same work twice.** A reusable result already in the chosen output folder means nothing happens; one in a *different* folder is **copied** (`jobs::reuse_result`), because the source is unchanged and the format matches, so the bytes a re-run would buy are bytes already on disk; anything else goes to the provider. That is why `Scan` reports `already_here_*` and `reusable_*` separately: the first is "nothing to do", the second is free work the Run button must stay enabled for, and it relabels itself "Copy N results" when that is all that is left. Retention is capped at 5,000 rows; `PRAGMA user_version` makes a new column a migration rather than a crash.
- **`secrets.rs`**: API keys in the **macOS data protection keychain** via `security-framework` (service `dev.esfandiari.toolkit`, access group `92MA44797J.dev.esfandiari.toolkit`, accounts `datalab`/`revai`/`backend`). Keys never reach the webview or disk. Reads are memoised per process for latency, not for prompt count: see the two-keychain Gotcha for why there are no prompts left to count.
- **`settings.rs`**: non-secret config (input paths, output dir, job, Datalab format, optional pipeline id, high-accuracy toggle) persisted as JSON in the app config dir. **`#[serde(default)]` on the struct is load-bearing**: without it, adding a field makes every existing `settings.json` fail to parse, and `load()` swallows the error and silently resets the user's output folder and job. `save()` writes to a temp file and renames for the same reason: `load()` answers a torn or truncated file with `unwrap_or_default()`, so a crash mid-write would wipe the user's config.
- **`tree.rs`**: one directory level of the library, read straight from disk with `std::fs`. It owns the listing rules and nothing else: the workspace-escape check, the dotfile and `project.json` filter, Finder's case-insensitive natural sort with folders interleaved, the source-to-result pairing (a sibling stem plus `output_extension_for`, `{stem} (n).{ext}` included), the 500-entry cap, and whether a row opens in the document pane. It stays out of `workspace.rs` because that module owns project identity and `.toolkit/index.db`, and disk is authoritative while the database is a rebuildable index over it. It carries no Tauri types, so every rule is unit-testable on its own. `list_project_files` wraps it and runs the walk on the blocking pool: a sync command runs inline on the AppKit main thread, and a lazy tree calls this on every disclosure click.
- **`backend_host.rs`**: owns the sidecar process and answers "where is the backend, and what is its token". Spawns on `127.0.0.1:0` and learns the real port by parsing the service's own `conversion service listening` line off stdout, which is why that message string is load-bearing on both sides. Mints the token through `secrets::set_key` **before** writing the 0600 token file, because `secrets.rs` memoises a miss for the whole process and only `set_key` drops the memo. The data root is stable at `<app_data>/converter`: a fresh one per launch would empty the service's replay ledger, so a recovered submit carrying a stored idempotency key would create a duplicate conversion instead of replaying. Emits `backend-status` on every transition.
  - **Stopping takes two mechanisms because they fail in different places.** Closing the child's stdin needs no pid and reaches a child we never learned one for. `/bin/kill -TERM` reaches a build with the stdin knob off. Both run, then `SIGKILL`. `unsafe_code = "forbid"` rules out `libc::kill` and an inner `#[allow]` cannot lift a `forbid`, so the signal goes through `/bin/kill`.
- **`lib.rs`**: the Tauri command surface + `setup()` (menu-bar tray, ⌥⌘V global shortcut, the **Settings…** item spliced into the app submenu). `run_pipeline` expands the selected files/folders into a concrete file list (`collect_input_files`), clears the queue, and spawns one job per match. Commands return `Result<T, String>`.
  - **`convert_one` is modelled on `retry_job`, never on `run_pipeline`.** It converts one file from the tree into the folder that file already sits in, which is what keeps the pairing rule true next time. It reads the generation instead of bumping it, appends one job instead of clearing the queue, and never touches the backend ledger of a run in flight: `run_pipeline`'s opening four lines would wipe every row the user is still reading. It runs the same preflight a run does and answers every refusal as a verdict (`kind`, `reason`, `message`) rather than an `Err`, because the tree renders that answer. It refuses outright while a run is going, because `run_config` is one snapshot per manager and `log_history` reads it at finish time.

### Adding a provider / job

Add a `JobType` variant and a `ProviderKind`, implement submit/poll in `providers.rs`, and extend `JobType::accepts` plus the `Scan` struct in `lib.rs` so the new job takes part in autodetect. The UI, queue, events, and output handling are reused unchanged.

### Frontend (`apps/desktop/src/`)

`src/shell/App.tsx` owns all state and hands it to view components in `src/domains/{run,library,onboarding,history,settings,thread}`. The shell's non-run concerns are extracted as hooks beside it: `useToast` (the one channel for errors that never reach a job row), `useDocuments` (which results are open, which one the pane shows, and which file the inspector card describes) with `useDocumentSave` (the debounced write, ⌘S, and the write before a close), `useProjectTree` (the library tree's open folders, children cache, selection, and freshness), `useHostWindow` (`useDragDrop`, `useWindowFocusClass`, `useCloseConfirm`), and `useZoom` (⌘+/-/0). Local state mirrors `Settings` and is persisted via `save_settings` on change; the job list is authoritative from `job-updated` events. Inputs are a list of paths added by drag-drop (`getCurrentWebview().onDragDropEvent`) or the Files/Folder pickers.

`src/platform/host.ts` is the only module allowed to import `@tauri-apps/*` for dialogs, window, and drag-drop. `src/app/commands.ts` is the typed IPC boundary.

`src/app/api/` is the conversion-service client layer, built on the OpenAPI contract rather than custom wrappers: `schema.ts` is generated from `contract/http/openapi.yaml` by `pnpm generate:api` (ESLint blocks importing it from anywhere else), and `client.ts` is a standard `openapi-fetch` client typed by that schema. `transport.ts` plugs a Tauri-backed `fetch` into openapi-fetch's documented seam: requests go through one `service_request` command so the host can attach the Keychain token and stream multipart sources from disk (the Rust handler lands with M6). Call sites use the library's `{ data, error }` results; errors carry the contract's ErrorEnvelope.

- **Autodetect.** `scan_inputs` returns a per-job match count, a per-job already-done count, and a suggested output folder. An effect picks the job with more matches and defaults the output folder. It depends on **`scan.convert` / `scan.transcribe` / `scan.suggestedOutput`, never the whole `scan` object and never `settings.jobType`**: either would re-fire the effect on an unrelated refresh and undo a manual job click instantly. That is also why both jobs' already-done counts come back in one scan: switching job reads a number that is already in hand rather than triggering a new scan.
- **Already done.** The Run button promises `inputCount - skipping`, not the raw match count, and `run_pipeline` does the authoritative filtering at run time from the same `Settings` load it snapshots as the run config. `runsFinished` is bumped **once** when a run ends, to refresh the counts and an open History panel: a 200-file run emits hundreds of `job-updated` events, so reacting to those would re-scan the disk hundreds of times.
- **`starting` guards double-runs.** `running` is derived from the job list, which stays empty until the first event lands, so without the guard a double-click fires two runs.
- **Two columns, and the right one is always the document pane.** `App.tsx` picks the left pane with a switch on `view`, which is `"library" | "run" | "history"`, and each of the three takes the pane's full width. The right pane is `DocumentPane`: the open tabs, its own empty state, or the inspector card layered over either. The split still collapses to one column while nothing is open, which is the resting state on every launch, because open tabs are not restored across one.
- **The library is a tree the host lists.** `useProjectTree` holds one `DirListing` per workspace-relative path, the open set (persisted as `settings.expandedPaths`), and the selection, all keyed on that relative path and never on an index, because a refresh reorders rows. `list_project_files` reads one directory level per call, so expansion is lazy. Freshness is `runsFinished` plus a debounced window-focus reconcile that drops the answer for any folder whose own mtime did not move. There is no watcher, and nothing in the tree reacts to `job-updated`.
- **The tree asks the host for a verdict, it never plans a conversion.** Convert on a row calls `convert_one(rel)`, which answers `queued`, `copied`, or `blocked` with a reason and the sentence to show. `blocked` stages the file in Run and switches the view, because the Run hint chain is gated on a non-empty selection and would otherwise land the user on an empty column. Planning it in the webview would be a second planner over the route rules, key checks, and reuse rule `run_pipeline` already owns.
- **Settings is a sheet, not a view.** The gear in the sidebar footer, **Tool-Kit ▸ Settings…** at ⌘,, and the run hint all open it, so opening it no longer evicts the Run column mid-run. `App.tsx` renders the toast region inside the sheet while it is up and at the app root otherwise. Escape is one ordered handler: the sheet answers first, then the inspector card, then the active document.
- **The window grows once and never shrinks back.** `src/shell/geometry.ts` names every window size, and `App.tsx` and `tauri.conf.json` both read it. The window opens at `INITIAL`, then the settings load moves it once: to `ONBOARDING` on first run, or to `WORKSPACE` at the size the user last left (`settings.expandedWidth/Height`), under a ceiling from `currentMonitor().workArea` rather than `screen.availHeight`, which means nothing definite under page zoom and only ever describes one display. Binding a workspace is permanent, so there is no path back. See Gotchas for the ordering rule, which still governs that one move.
- **Zoom is the whole app, not the document.** ⌘+/-/0 step `webview.setZoom` along a fixed ladder (`ZOOM`, 10 points a step) and persist as `settings.zoom`. Stepping moves an index rather than adding to the factor: `1 + 0.1 - 0.1` is not 1 in binary floating point, so ⌘+ then ⌘- would never come home. `useZoom` publishes the factor as `--zoom` on `:root` for the chrome that has to divide by it.
- Inputs auto-clear after a run only when that run was started by `run()` *and* something succeeded, so a Retry or a wholly failed run leaves the selection alone.

### Design system (`apps/desktop/src/ui`, docs in its `UI.md`)

The design language lives in a self-contained library imported as `@ui`. **Read `src/ui/UI.md` before touching any UI.** Its four rules are the reason to reject a change:

1. **Colour is signal, never decoration.** Amber = live, green = passed, red = failed, cobalt = the control you press. Icons, the brand mark, and folder glyphs are never coloured.
2. **Values light up, they don't appear.** Counts and timers hold their slot as dim ghost glyphs so the window never reflows while jobs finish out of order.
3. **macOS first.** Real vibrancy under a scrim, SF metrics, key/inactive window states (`body.inactive`), HIG focus rings, tabular numerals, and native `<select>`/`<input type=checkbox>` under restyled shells.
4. **Whitespace is a grammar, not a feel.** `--s1`..`--s8` step from atoms of one object up to the window edge, and the gap is inversely proportional to the relationship. The outer margin is the largest gap on screen, and one element per scroll column absorbs the slack. UI.md holds the ladder.

Type is three families with non-overlapping jobs: **Instrument Serif** for display, **SF Pro** for prose, **JetBrains Mono** for every label, tab, and button (uppercase, `0.09em` tracked. the signature of the language). Both faces are self-hosted OFL; see `src/ui/fonts/THIRD_PARTY_NOTICES.md`.

- **The `@ui` boundary is enforced by ESLint.** Nothing under `src/ui` may import from `@/app`, `@/domains`, `@/platform`, or `@tauri-apps/*`. When a primitive needs something from the product it takes it as a prop: `RunView` maps its five job statuses onto three `Status` tones in one function rather than teaching the library about jobs.
- **A primitive with no specimen in `src/ui/gallery/Gallery.tsx` does not exist.** Add it in the same commit. Review it at `pnpm dev` → `http://localhost:1420/?gallery`, which runs with no Tauri bridge and has a Graphite/Paper theme toggle.
- **`App.tsx` must import `@ui` before `./App.css`.** `App.css` deliberately does not `@import` the token layer: doing it in both places shipped the whole 16KB twice, in two chunks with competing `:root` blocks.
- **`src/shell/App.css` is app composites only** (title bar, drop well, folder picker, project tree rows, job row layout, markdown prose). It carries no `:root` block at all any more, and it must not grow one: the only variables left in it are the `body.inactive` overrides. A new custom property belongs in `tokens.css`, or on the element that owns it, the way `--bar-h` sits on `.app`.

## Gotchas

- **An unsigned release build cannot ship and cannot reach the keychain.** `pnpm tauri build` with no `APPLE_SIGNING_IDENTITY` still produces a working `.app`, ad-hoc signed, whose designated requirement is `cdhash H"…"`: a literal hash of that one binary. Gatekeeper on any other Mac rejects it, notarization will not touch it, and `codesign` will not honour the restricted `keychain-access-groups` entitlement, so the app silently drops to the legacy keychain. A Developer ID build gets `identifier "dev.esfandiari.toolkit" and anchor apple generic and … subject.OU = "92MA44797J"`, stable across builds *and versions*. `apps/desktop/src-tauri/scripts/verify-release.sh` fails on every part of this by name.
- **macOS has two keychains and only one of them has no dialogs.** The legacy file-based store grants access through a per-item ACL bound to the reader's *designated requirement*, so any signature change voids every "Always Allow" and the prompts return. The data protection store has no ACLs at all: access is the `keychain-access-groups` entitlement, matched on **team id**, so no dialog exists in that path. `secrets.rs` targets the second through `security-framework`'s `use_protected_keychain()`.
  That entitlement is **restricted**, so `codesign` honours it only when `Contents/embedded.provisionprofile` authorises the claim. A bundled `.app` carries one. A bare `cargo run` Mach-O has nowhere to put it, so the dev loop gets `errSecMissingEntitlement` (-34018), falls back to the legacy keychain, and keeps its **own separate copy of every key**. Exercise the real path with `pnpm tauri build --debug`. `scripts/dev-run.sh` still signs the dev binary, now only so those legacy items stay trusted across rebuilds. Still let the app create the items: don't seed them with the `security` CLI.
- **The provisioning profile is pinned to the signing certificate**, and macOS evaluates it at install and at every launch. `apps/desktop/src-tauri/embedded.provisionprofile` lives ~18 years, but the Developer ID cert inside it expires **2031-06-18**. Rotating the cert means regenerating the profile from the portal, or the app stops launching.
- **`pnpm sidecars` gates `cargo`, not just the bundler.** `tauri-build` resolves `externalBin` in the build script, so a missing staged binary fails `cargo check`/`clippy`/`test` and `pnpm tauri dev` with `ResourcePathNotFound`. The artifacts are gitignored, so this bites on every fresh clone and after every `git clean`.
- **`Contents/MacOS` is sealed as nested code, so only executables go there.** The bcmaps are a `resource`, never an `externalBin`. A data file in `MacOS/` breaks the seal on the user's machine while every check on the build machine still passes. `verify-release.sh` sweeps that directory and fails anything that is not Mach-O.
- **`TOOLKIT_CONVERTER_PDF_BCMAPS_DIR` fails silently when unset.** `pdf-inspector` falls back to `CARGO_MANIFEST_DIR/external/bcmaps`, which exists on the build machine and on nobody else's, so CJK PDFs lose their ToUnicode mapping with no error anywhere. `backend_host.rs` treats a missing bcmaps resource as fatal to the start on purpose. Do not soften that to a warning.
- **Set `RUST_LOG` for the child, never inherit it.** The supervisor learns the sidecar's port by parsing one log line. A quieter inherited filter suppresses that line and the app hangs at Starting with nothing on screen to explain it.
- **`RunEvent::Exit`, not `ExitRequested`.** `ExitRequested` never fires on ⌘Q for this app, because the window hides instead of being destroyed. And a long blocking wait inside `Exit` stalls AppKit termination and the app reads as hung, which is why the converter's shutdown grace is 5s and the Exit path caps at 7.
- **A converter that reads stdin must not read it on tokio's blocking pool.** Dropping the runtime waits for blocking tasks that already started, so a pool read of a pipe nobody closes makes SIGTERM inert: the service drains, `main` returns, and the process then hangs forever. `stdin_eof` uses a detached `std::thread`, which does not hold up process exit.
- **Capabilities.** Custom Tauri commands need no capability entries, but **core commands do**: `core:window:default` does *not* include `hide`/`destroy`, so the close-confirm handler needs them listed explicitly in `capabilities/default.json`. A missing one fails silently at runtime. The window-sizing and zoom work added `set-max-size`, `set-resizable`, `current-monitor`, and `webview:set-webview-zoom` for the same reason. Window and app events ride `core:event:default`, which `core:default` already carries, so `onFocusChanged` and `listen("open-settings")` need nothing.
- **Do not add a `permissions/` directory under `src-tauri`.** It flips `has_app_manifest()` to true, and from that moment every custom command in the app needs an explicit ACL entry or is rejected at runtime. That is why `list_project_files` and `convert_one` appear in `generate_handler!` and nowhere else, and why adding one plugin permission file would silently break every command that has none.
- **A window focus event never reaches a `getCurrentWebview()` listener.** It is delivered only to targets of kind Window and WebviewWindow, so a webview-registered listener subscribes cleanly and then never fires, with no error and no warning to say why the tree stopped refreshing. `host.ts` wraps `getCurrentWindow().onFocusChanged`. The drag-drop subscription three functions above it uses the webview, which is what makes the wrong one the natural reach.
- **A fixed toast is invisible under a modal `<dialog>`.** `showModal()` renders in the top layer, above every z-index, so `.toast` at z-index 60 disappears the moment the Settings sheet opens, and saving an API key is the most common thing Settings does. `Sheet` takes an `overlay` slot and `App.tsx` renders the toast region into it while the sheet is up. A modal dialog also makes the rest of the document inert, which stops `data-tauri-drag-region` answering a hit test, so the sheet carries its own drag strip through `head` or the window cannot be moved while it is open.
- **An app-menu item id must not collide with the tray's.** The tray's `on_menu_event` closure is a global menu listener rather than a tray-scoped one, and it matches the bare strings `"show"` and `"quit"`. The Settings item is `toolkit:settings`, which falls to that closure's `_ => {}` arm. Splice it into the menu Tauri already installed with `Submenu::insert`, never `set_menu`: a fresh menu drops Edit, View, Window and Help without saying so.
- **Window geometry lives in `src/shell/geometry.ts` and nowhere else.** `App.tsx`, `useZoom.ts` and `tauri.conf.json` all read those names, so the config that opens the window and the code that resizes it cannot drift apart. `SplitPane`'s defaults repeat `SPLIT` by hand only because nothing under `src/ui` may import from the app. `SPLIT.minStart` is what the run column's widest control needs, and `WORKSPACE.minWidth` is then derived from it: `SPLIT.start`% of the minimum window must clear the floor, or the floor beats the ratio at every width and the seam sits at its minimum forever. Change one and redo the arithmetic on the other.
- **macOS clamps `setSize` to the min and max in force at that instant**, so the bounds move before the size. Growing into the workspace: `setResizable(true)` → `setMinSize` → `setMaxSize` → `setSize`. Backwards, the window lands at the opening size and stays there, with no error to say so. The app only ever grows, so this runs once per launch, but it still runs on the path every user takes.
- **⌘+/- is webview page zoom, and it does not move the traffic lights.** macOS draws them in logical pixels, so a 64px CSS reservation is 32 logical px at zoom 0.5 and the lights land on top of whatever sits beside them. Any chrome measured against an OS-drawn element divides by the factor instead: `calc(64px / var(--zoom, 1))`, with `--zoom` published on `:root` by `useZoom`. `--bar-h` on `.app` divides its top term for the same reason, and the Settings sheet hangs off that edge.
- **`react-resizable-panels` has two layout paths that disagree about what a key means.** `defaultLayout` is read by panel id. `setLayout` reads `Object.values(layout)` and re-keys the result by panel order, so it is an array wearing an object's clothes and key order decides which pane gets which width. Our layout arrives from a Rust `BTreeMap`, which serialises alphabetically, so `{end, start}` came back and opened the split inverted: two thirds on the left pane, one third on the document, and every drag saved the inversion back. `paneLayout` in `src/ui/primitives/splitLayout.ts` rebuilds the object in panel order and is the only thing allowed to construct one.
- **`defaultLayout` is also validated against the panels present at mount**, and the document pane renders only once a document is open. So the group starts with one panel, drops the two-id layout whole, and hands out an even split when the second pane appears. `SplitPane` re-applies imperatively on the collapsed→expanded edge, and retries because the group registers its second panel a render later than the effect runs.
- **Icons.** `icons/make-icons.py` renders every shipped size, `.icns` ladder
  included, from `icons/mark.png`. **That file is missing and has never been in
  git**, so the ladder cannot be regenerated today. Do not reconstruct it by
  upscaling a shipped icon: the mark is two flat colours with no blended pixels,
  and a 256px raster promoted to master would bake its own resampling into every
  size forever. Redraw it or find the original. `icons/make-icons.py` renders every shipped size from it, `.icns` ladder included. Colours come from the `--warm-*` ramp in `src/ui/tokens.css`, and the plate is `--surface-solid`, so the Dock icon and the window are the same charcoal. `icons/tray.png` / `tray@2x.png` derive from **the mark's** alpha as **template images** (black + alpha, used with `icon_as_template(true)`) so macOS tints them for light and dark menu bars: putting the colour app icon in the tray is a visible native-correctness bug. Take that alpha from `mark.png`, never from `icon.png`. The app icon is a filled plate, so its alpha is a black square.
- **Global shortcut is ⌥⌘V**, not ⌘⇧V: that one is macOS "Paste and Match Style" and registering it hijacks the combination system-wide.
- **Datalab pipeline mode**: a `pl_…` id in Settings switches Convert from `/api/v1/convert` to `/api/v1/pipelines/{id}/run` (run → poll execution → fetch the last step's result).
- **pnpm 11** gates package build scripts: esbuild is approved via `allowBuilds: { esbuild: true }` in `pnpm-workspace.yaml`.
- **TypeScript stays on 6.x.** 7.0 has no compiler API; `typescript-eslint` crashes. `tsconfig.json` must not set `baseUrl` (deprecated in 6).
- **`security-framework` needs the `OSX_10_15` feature**, which is not a default. It gates `PasswordOptions::use_protected_keychain()`, and without it `secrets.rs` silently targets the legacy keychain.
- **Do not create a root Cargo workspace.** `src-tauri` and `backend` keep separate lockfiles until a dedicated migration.
- **Never `git add -A`.** Desktop and backend work share one dirty tree; stage explicit paths. See `docs/archive/BACKEND_BASELINE.md` and `docs/STATUS.md`.

## Reference

One live planning document plus the repo guides. Everything else is closed and
lives in [`docs/archive/`](docs/archive/README.md).

- [`docs/STATUS.md`](docs/STATUS.md): state, critical path, milestones, architecture, traps, verify commands
- [`.impeccable.md`](.impeccable.md): design context; every `/impeccable` skill reads it
- Local Tauri v2 docs: `/Users/tristin/code/knowledge-base/topics/tauri/raw/tauri-v2`.

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
