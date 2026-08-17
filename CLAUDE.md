# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Tool-Kit is a Tauri 2 + React/TypeScript macOS desktop app that consolidates the "turn a file into AI-ready text" tools the author uses daily into one window. You pick input **files and/or folders**, an **output folder**, a **job**, and hit Run. Results are written to the output folder and previewable in-app.

| Job | Service | In → Out |
|---|---|---|
| Convert | Datalab (`/api/v1/convert`, or a pinned pipeline) | PDF / DOCX / images / … → markdown |
| Transcribe | Rev.ai (async) | audio / video → text |
| Summarize | Claude Messages API | text / markdown → markdown notes |

## Commands

Run from the repo root.

```bash
pnpm tauri dev        # run the app with HMR (first run is slow — it compiles the Rust backend)
pnpm build            # frontend only: tsc + vite build
pnpm lint             # ESLint, type-aware (recommended + stylistic + react-hooks); kept clean
pnpm lint:fix

cargo check  --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets   # kept warning-clean
```

### Live API smoke tests

`src-tauri/src/live_smoke.rs` hits the real Datalab + Rev.ai endpoints. They are `#[ignore]`d (they spend API credits) and read keys from the environment. Sample inputs are generated under `/tmp/toolkit-test/` (override with `TEST_PDF` / `TEST_AUDIO`).

```bash
# both; keys may come from .env.local — note REV_AI_API_KEY must be exported as REVAI_API_KEY
DATALAB_API_KEY=… REVAI_API_KEY=… \
  cargo test --manifest-path src-tauri/Cargo.toml --lib live_smoke -- --ignored --nocapture

# a single test
… cargo test --manifest-path src-tauri/Cargo.toml --lib live_smoke::datalab_convert_live -- --ignored --nocapture
```

### Release / signing / notarization (macOS)

```bash
# sign-only (fast; OK for local installs — locally-built apps aren't quarantined, so Gatekeeper allows them)
APPLE_SIGNING_IDENTITY="Developer ID Application: … (92MA44797J)" pnpm tauri build

# sign + notarize + staple (for the distributable DMG): additionally export APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID
```

Artifacts: `src-tauri/target/release/bundle/{macos/Tool-Kit.app, dmg/*.dmg}`. The Developer ID cert needs Apple's **G2 intermediate** in the keychain (`certs.apple.com/devidg2.der`) or the signing identity reads as untrusted/invalid. Signing + Apple creds live in `.env.local` (gitignored).

## Architecture

**One pipeline, many tools.** Datalab, Rev.ai, and Claude are different services but share the same shape, so this is a single job queue with pluggable providers, not three separate integrations. The entire UI reduces to: inputs → output folder → `JobType` → Run.

**The backend does everything; the frontend is a thin shell.** All network calls, secrets, and file IO are in Rust. `src/App.tsx` (one component) only calls Tauri commands via `invoke()` and stays in sync by listening to `job-updated` events — there is no business logic in the frontend.

### Rust (`src-tauri/src/`)

- **`providers.rs`** — the integration layer and the main extension point. Each service is `submit → poll → fetch` (`datalab_*`, `revai_*`); Claude (`anthropic_summarize`) is one synchronous request. `send_retrying()` wraps the submit/summarize HTTP calls with backoff on 429/529/5xx (multipart bodies are rebuilt per attempt). Results flow through `PollResult` (Pending/Done/Failed).
- **`jobs.rs`** — `JobType` maps a job to its provider, accepted extensions, and output extension. `JobManager` is the in-memory queue (`Mutex<Vec<Job>>` + `AtomicU64` ids) with a `Semaphore` capping **4 concurrent jobs** to respect upstream rate limits. `run_job()` spawns the per-file lifecycle on `tauri::async_runtime` and emits a `job-updated` event on every state change; convert/transcribe poll every 5s (~60 min cap), summarize is a single call.
- **`secrets.rs`** — API keys in the **macOS Keychain** via `keyring` (service `ai.uniwise.toolkit`, accounts `datalab`/`revai`/`anthropic`). Keys never reach the webview or disk.
- **`settings.rs`** — non-secret config (input paths, output dir, job, Datalab format, optional pipeline id, summarize model) persisted as JSON in the app config dir.
- **`lib.rs`** — the Tauri command surface + `setup()` (menu-bar tray, ⌘⇧V global shortcut). `run_pipeline` expands the selected files/folders into a concrete file list (`collect_input_files`), clears the queue, and spawns one job per match. Commands return `Result<T, String>`.

### Adding a provider / job

Add a `JobType` variant and a `ProviderKind`, implement submit/poll (or a sync call) in `providers.rs`, and add the branch in `jobs.rs::run_job`. The UI, queue, events, and output handling are reused unchanged.

### Frontend (`src/App.tsx`)

Single component. Local state mirrors `Settings` and is persisted via `save_settings` on change; the job list is authoritative from `job-updated` events. Inputs are a list of paths added by drag-drop (`getCurrentWebview().onDragDropEvent`) or the Files/Folder pickers, and auto-clear when a run finishes. Custom Tauri commands need **no** capability entries — only plugin/core commands do (`src-tauri/capabilities/default.json` grants `core`, `opener`, `dialog`).

## Gotchas

- **Keychain prompts in dev.** The unsigned `tauri dev` binary's code identity changes on every rebuild, so macOS re-prompts for Keychain access on each key read. Enter keys in the **signed installed app** (stable identity, owns its own items) for silent access; don't seed keys with the `security` CLI for real use — let the app create them.
- **Claude** is raw HTTP (no official Rust SDK): `POST /v1/messages`, headers `x-api-key` + `anthropic-version: 2023-06-01`, model default `claude-sonnet-4-6` (Settings → Advanced). It branches on `stop_reason` (refusal → error; max_tokens → truncation marker) and truncates input on a char boundary at 500k chars.
- **Datalab pipeline mode**: a `pl_…` id in Settings switches Convert from `/api/v1/convert` to `/api/v1/pipelines/{id}/run` (run → poll execution → fetch the last step's result).
- **pnpm 11** gates package build scripts: esbuild is approved via `allowBuilds: { esbuild: true }` in `pnpm-workspace.yaml`.

## Reference

Local Tauri v2 docs the author keeps: `/Users/tristin/code/tauri-skills/knowledgebase/tauri-v2`.
