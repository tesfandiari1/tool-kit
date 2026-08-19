# Backend scaffold worktree baseline

**Captured:** 2026-08-17  
**Branch:** `main`  
**Starting commit:** `bd23d91`  
**Purpose:** Preserve ownership boundaries while the conversion service is
built alongside an already-modified desktop application.

## Repository conditions at scaffold start

- No `.codegraph/` index was present.
- No root Cargo workspace or existing backend/service directory was present.
- `src-tauri/` was and remains an independent Rust crate with its own lockfile.
- The worktree already contained 27 tracked desktop changes plus untracked
  research and tray-icon files.
- The scaffold did not modify any `src/` or `src-tauri/` file.

## Pre-existing tracked desktop paths

```text
M  src-tauri/Cargo.lock
M  src-tauri/Cargo.toml
M  src-tauri/capabilities/default.json
M  src-tauri/icons/128x128.png
M  src-tauri/icons/128x128@2x.png
M  src-tauri/icons/32x32.png
D  src-tauri/icons/Square107x107Logo.png
D  src-tauri/icons/Square142x142Logo.png
D  src-tauri/icons/Square150x150Logo.png
D  src-tauri/icons/Square284x284Logo.png
D  src-tauri/icons/Square30x30Logo.png
D  src-tauri/icons/Square310x310Logo.png
D  src-tauri/icons/Square44x44Logo.png
D  src-tauri/icons/Square71x71Logo.png
D  src-tauri/icons/Square89x89Logo.png
D  src-tauri/icons/StoreLogo.png
M  src-tauri/icons/icon.icns
D  src-tauri/icons/icon.ico
M  src-tauri/icons/icon.png
M  src-tauri/src/jobs.rs
M  src-tauri/src/lib.rs
M  src-tauri/src/live_smoke.rs
M  src-tauri/src/providers.rs
M  src-tauri/src/settings.rs
M  src-tauri/tauri.conf.json
M  src/App.css
M  src/App.tsx
```

The current diff hash for those `src/` and `src-tauri/` paths after scaffold
verification was:

```text
sha256 8c972ba22737984d1be6b4d55d3bbafc85d7ac5be948d85132179e45e70e1ba0
```

This hash is evidence, not a lock: concurrent user edits remain authoritative.
Re-read the live worktree before every future integration patch.

## Concurrent root-document changes

`README.md` and `CLAUDE.md` changed after the initial status capture. This
scaffold appended only the conversion-backend section to `README.md` and did
not edit `CLAUDE.md`. Do not revert, reformat, or claim ownership over the
remaining root-document changes without first reconciling them with the user.

## Files owned by the scaffold unit

```text
docs/archive/BACKEND_BASELINE.md
docs/archive/BACKEND_EPIC.md
docs/archive/BACKEND_EXECUTION_PLAN.md
docs/archive/BACKEND_SERVICE_PLAN.md
backend/**
README.md             # conversion-backend section only
```

Planning documents later moved from the repository root into `docs/`. Session
state is [`STATUS.md`](../STATUS.md). Shared repo files added after
scaffold (`.github/`, `LICENSE`, `rust-toolchain.toml`, `.node-version`,
`.gitignore`, `package.json`) are not backend-owned; do not revert them.

When staging later, use these exact paths. Never use `git add .` or `git add -A`
in this worktree.
