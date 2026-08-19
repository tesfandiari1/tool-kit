# Workspace weave (closed)

**Closed:** 2026-08-18 · **Do not re-do this work.**

The desktop is a dual-pane workspace. Compact is the launcher (a stack of
`Panel`s: Input, Output, Job). Opening a finished result grows the window,
splits the body, and re-ranks the left column so the queue is navigation.
Closing the last document collapses back to compact. Edit writes to disk.

This is **not** backend M7 (LAN / ops in `BACKEND_EPIC.md`). It is the desktop
shell weave that used to live as a wiring guide. The next desktop session is
M6 (`service_request`), not another workspace pass.

Read [`STATUS.md`](../STATUS.md) first. Design brief:
[`.impeccable.md`](../../.impeccable.md). System: [`src/ui/UI.md`](../../src/ui/UI.md).

## What shipped

| Thing | Where | Notes |
|---|---|---|
| `useDocuments` | `src/shell/useDocuments.ts` | `OpenDoc[]`, `activeId`, `mode`. Both doors call `read_document`. Same path activates, does not duplicate. |
| `useDocumentSave` | `src/shell/useDocumentSave.ts` | Autosave 800ms, ⌘S, save on tab switch and close. Mtime refuse → `SaveState.error`. |
| `read_document` / `write_document` | `src-tauri/src/lib.rs`, `src/app/commands.ts` | Temp-file rename. Refuse on mtime mismatch. Do not reuse `jobs::write_output`. |
| Shell | `src/shell/App.tsx` | `SplitPane` + `DocumentPane` when `docs.length > 0`. Settings and History replace the **left pane only**. |
| Compact launcher | `src/domains/run/RunView.tsx` | Same primitives as the gallery Queue/Output specimens: `Panel`, `Well`, `StatusDot`, `Mono`, `Badge`. |
| Queue as nav | `RunView` | Finished row click opens. Selected: `--surface-raised` + `--ink`. No eye button. |
| Window | `src/platform/host.ts` | Compact min 420. Expanded min ~900. Split layout and expanded size persist in `Settings`. |
| Collision note | `src/domains/run/plan.ts` | Skip-off re-runs that would clobber are named as numbered copies. `write_output` is unchanged. |

`View` is `"run" | "settings" | "history"`. `ThreadView` is gone. Copy and
reveal still use the existing commands; the pane copies `doc.text`.

Skipped on purpose:

- `edited_at` on history (`PRAGMA user_version`). Reuse still keys on the
  source mtime, so an edited output copied to another folder is the edited
  text. Record that column later if the copy needs to say so.
- `src/domains/thread/FileViewer.tsx` is leftover from the old reader. Nothing
  imports it. Delete it in a cleanup pass; do not resurrect it as a view.

## How to verify

```bash
pnpm check                    # tsc --noEmit + eslint + vitest
pnpm verify                   # check + build + src-tauri clippy/tests
pnpm tauri dev                # the real window; macOS only
```

Compact: drop then Run. Open a done row: split + window grow. Type in Edit:
line numbers stay aligned; relaunch keeps the edit. Close the last tab:
compact. An external edit of the same file refuses the next save (red dot,
header note). Skip-off over an existing result names numbered copies under
the Run button.

`pnpm dev` still cannot render the app. `?gallery` is the Workspace primitives
(Graphite and Paper).

## Parallel-session collisions with M6

This weave is desktop-only and touches no `backend/**`. It does collide with
M6 inside `src/`:

| File | M6 wants | This weave shipped |
|---|---|---|
| `src/shell/App.tsx` | run-view wiring for the service client | the split layout and document state |
| `src/app/commands.ts` | `service_request` | `read_document` / `write_document` |
| `src-tauri/src/lib.rs` | the `service_request` handler | the document commands |
| `src-tauri/src/settings.rs` | backend URL and token | window size and split layout |
| `src/app/types.ts` | `Settings` fields | `Settings` fields, `View` |

`Settings` is the sharpest edge: both sides add fields, and `#[serde(default)]`
means a mismatch does not error, it just quietly loses a value.

`src/app/api/schema.ts` is M6's alone. This weave never touches it.
`service_request` is still M6's: it is in `commands.ts` but not in
`generate_handler!`. Calling it today fails at runtime.
