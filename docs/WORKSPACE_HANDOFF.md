# Workspace handoff (M7)

**Written:** 2026-08-18 · **Branch:** `claude/clever-goodall-x4vxag` · **Code
checkpoint:** `7892c3a`

How to weave the workspace primitives into the running Tauri app. The
primitives landed and are verified; nothing in the app reaches them yet. This
file is the wiring.

Read [`HANDOFF.md`](HANDOFF.md) first for parallel-session rules, then this.
Design rationale and the four rejected layouts live in the Tool-Kit Workspace
artifact; the design brief is [`../.impeccable.md`](../.impeccable.md).

## What is already done

Commit `7892c3a` added three `@ui` primitives, each with a gallery specimen,
plus one app composite. All of it passes `pnpm check` and `pnpm build`, and was
driven in Chromium against `?gallery` in both themes.

| Thing | Where | Contract |
|---|---|---|
| `Tabs` | `src/ui/primitives/Tabs.tsx` | `items`/`value`/`onChange`/`onClose`. A real `tablist`. **The caller owns the panel** — see the wiring note below the table |
| `SourceEditor` | `src/ui/primitives/SourceEditor.tsx` | `value`/`onChange`/`readOnly`. Lazy behind `Suspense`; the CodeMirror chunk is fetched on first render |
| `SplitPane` | `src/ui/primitives/SplitPane.tsx` | `start`/`end`/`layout`/`onLayoutChanged`. Persistence is **not** delegated — you save the layout |
| `DocumentPane` | `src/domains/thread/DocumentPane.tsx` | Arranges the three. **Not reachable from the app.** This is what you wire |
| `SaveState`, `saveTone`, `saveNote` | `src/domains/thread/model.ts` | The save-state machine and its mapping onto existing dot tones |

`Tabs` draws the strip and nothing else. Connect the panel yourself, or the
tabs announce a relationship that does not exist:

```tsx
<Tabs items={docs} value={id} onChange={setId} onClose={close} />
<div
  role="tabpanel"
  id={`ui-tabpanel-${id}`}
  aria-labelledby={`ui-tab-${id}`}
>
```

`DocumentPane` already does this; copy it if you rearrange things.

Deliberately not done: nothing that could ship an EDIT mode which silently
discards edits.

## Two facts that change the shape of the work

**`read_text_file` already exists**, as a Rust command and as
`commands.readTextFile`. Reading a document from disk is solved. Only the
write side is missing.

**`service_request` is in `commands.ts` but not in `generate_handler!`.** It is
M6's, and calling it today fails at runtime. Do not treat it as available.

## Increments

Each is landable on its own. Stop after 2 and the app is already better than it
is today: a document that opens beside the queue instead of replacing it.

### 1 — Documents become a list, not a view

`src/app/types.ts` · `src/shell/useThread.ts` · `src/domains/thread/`

- Drop `"thread"` from `View`. It becomes `"run" | "settings" | "history"`.
- `useThread` becomes `useDocuments`: `OpenDoc[]`, `activeId`, `mode`.
- **Both doors read from disk.** Today `openJobThread` uses the job's in-memory
  `outputText` and `openHistoryThread` reads the file. Once edits write to
  disk, that in-memory copy goes stale on the first save — so both paths should
  call `commands.readTextFile(outputPath)`. This deletes the asymmetry
  `useThread`'s own docstring apologises for.
- Key on `id` = output path. Clicking a row for an already-open document
  activates its tab; it must not open a second one.
- Escape currently closes the reader. Decide what it means now — closing the
  active document is the obvious reading, but **it must not discard a dirty
  document without asking**.
- Delete `ThreadView.tsx` once `DocumentPane` replaces it.

*Done when:* `View` has no `thread`, and a finished job opens in `DocumentPane`.

### 2 — Split the shell

`src/shell/App.tsx` · `src/shell/App.css`

- When `docs.length > 0`, the body becomes
  `<SplitPane start={<RunView …/>} end={<DocumentPane …/>} />`. Otherwise
  `RunView` alone, exactly as today.
- `.app` is `display:flex; flex-direction:column; height:100vh` and `.bar` is
  full-width above the panes, which is what you want — the title bar spans the
  window and the split sits under it. Give the `SplitPane` `flex:1` and
  **`min-height:0`**, or it will not scroll inside a flex column.
- **Watch `.flow`.** It is `max-width:620px; margin:0 auto`. Inside a 440px
  pane that is inert, but drag the divider wide and the run column starts
  centring itself in its own half. Decide whether that is what you want before
  someone reports it as a bug.
- Settings and History replace `RunView` today. Decide whether they replace the
  **left pane only** or the whole window. Left pane only is the better answer —
  a document should survive you changing a setting — but it is a real choice,
  not an obvious one.

*Done when:* opening a document splits the window; closing the last one
collapses it back to a single column.

### 3 — Resize the window

`src/platform/host.ts` · `src-tauri/capabilities/default.json` ·
`src-tauri/src/settings.rs` · `src/app/types.ts`

- `host.ts` is the only module allowed to import `@tauri-apps/*`. Add
  `resizeWindow()` and `windowSize()` there; it already imports
  `getCurrentWindow`.
- **Add the capabilities or it fails silently.** `core:default` does not cover
  resizing. You need roughly:
  `core:window:allow-set-size`, `core:window:allow-inner-size`,
  `core:window:allow-set-min-size`. **Verify the exact identifiers against
  `src-tauri/gen/schemas/desktop-schema.json` after a build** — that file is
  generated and is not in the repo, so the names above are the documented
  convention rather than something checked on disk. This is the same trap as
  `hide`/`destroy` already recorded in `CLAUDE.md`.
- Persist the expanded size and the split layout in `Settings`. `Settings` is
  `#[serde(default)]`-guarded so adding fields is safe, but add them in **both**
  `src-tauri/src/settings.rs` and `src/app/types.ts` + `DEFAULT_SETTINGS`, or
  the two sides disagree in silence.
- `tauri.conf.json` sets `minWidth: 420`. The split needs roughly 900 to be
  usable, so raise the minimum while expanded and drop it again on collapse.
- **Motion.** The window resize is a native animation; do not try to match it in
  CSS. The pane fades in over `--dur-slow` with `--ease-out` and a
  `scale(.99)` settle — transform and opacity only, no bounce. The
  `prefers-reduced-motion` block in `tokens.css` already zeroes the durations,
  which is why they are tokens.

*Done when:* the first document grows the window, the last one closing shrinks
it, and a size the user set by hand survives a relaunch.

### 4 — The queue becomes navigation

`src/domains/run/RunView.tsx` · `src/shell/App.css`

- The row opens the document. Retire the eye button.
- Selected state is **surface and ink only**: the row lifts to
  `--surface-raised` and the filename brightens to `--ink`. No cobalt — cobalt
  is the control you press, and a selection tint would make it the fifth
  meaning the palette does not have. (The active *tab* does use cobalt, on the
  grounds that a tab genuinely is a control you press. That is the one
  deliberate exception and it is one line in `Tabs.css` if you disagree.)
- **Do not make the row a `<button>`.** It already contains buttons, and
  nesting interactive elements is invalid. Use the pattern `Tabs` uses: a
  wrapper element with the primary control and the secondary controls as
  siblings.
- Arrow keys move the selection.

*Done when:* clicking a finished row opens it and marks the row selected.

### 5 — Write to disk

`src-tauri/src/lib.rs` · `src/app/commands.ts` · `src/shell/useDocuments.ts`

- New command. Suggested shape:
  `read_document(path) -> { text, mtime_ms }` and
  `write_document(path, text, expected_mtime_ms) -> mtime_ms`.
- **Write to a temp file and rename**, the way `settings.rs::save()` does. A
  crash mid-write must not truncate the user's document. This is the most
  important line in this increment.
- **Refuse the write if the current mtime is not the expected one.** That is
  the whole concurrent-writer story: edit in Tool-Kit and in another editor and
  last-writer-wins in silence otherwise. A refusal surfaces as the red dot and
  the reason in the header — `SaveState` already has `error`.
- **Do not reuse `jobs::write_output`.** It opens with `create_new` and will
  never overwrite; it is for provider output, not for edits.
- Autosave on a debounce (~800ms after the last keystroke), plus ⌘S, plus save
  on close and on tab switch.

*Done when:* an edit survives a relaunch, and an external edit produces a
refusal rather than a silent clobber.

### 6 — Settle the re-run semantics

The five questions in the artifact. What the code does **today**, verified:

| Question | Today | Suggested |
|---|---|---|
| Re-run over an edited output | `write_output` uses `create_new` and falls through to `report-1.md`. Your edit is safe, but you silently gain a file | Leave the Rust alone. Surface it in the scan instead — a silent duplicate is the bug, not the fallthrough |
| Reuse keying | `reusable()` keys on the **source** file's mtime, so editing an output never invalidates it. Copying to another folder copies *your edited text* | Probably right. Record `edited_at` so the copy can say so |
| `Job.outputText` staleness | Goes stale on the first save | Solved by Increment 1 |
| History honesty | A row can point at a file whose contents differ from what the provider returned | Optional `edited_at` column. `PRAGMA user_version` makes it a migration, not a crash |
| Concurrent writers | Nothing watches the file | Solved by the mtime guard in Increment 5 |

### 7 — Spacing and polish

Document-header rhythm, the empty state, and the collapse animation. `/polish`
and `/layout` both read `.impeccable.md`, so they have the context.

## Parallel-session collisions with M6

M7 is desktop-only and touches no `backend/**`. It does collide with M6 inside
`src/`, so do not run both in one session without checking:

| File | M6 wants | M7 wants |
|---|---|---|
| `src/shell/App.tsx` | run-view wiring for the service client | the split layout and document state |
| `src/app/commands.ts` | `service_request` | `read_document` / `write_document` |
| `src-tauri/src/lib.rs` | the `service_request` handler | the document commands |
| `src-tauri/src/settings.rs` | backend URL and token | window size and split layout |
| `src/app/types.ts` | `Settings` fields | `Settings` fields, `View` |

`Settings` is the sharpest edge: both sides add fields, and `#[serde(default)]`
means a mismatch does not error, it just quietly loses a value.

`src/app/api/schema.ts` is M6's alone. M7 never touches it.

## Verify

```bash
pnpm check                    # tsc --noEmit + eslint + vitest
pnpm verify                   # check + build + src-tauri clippy/tests
pnpm dev                      # then http://localhost:1420/?gallery
pnpm tauri dev                # the real window; macOS only
```

The desktop does not render in a plain browser, and `src-tauri` clippy and
tests run on `macos-latest` in CI because of `macos-private-api` and `keyring`.
`?gallery` is the bridge-free design review surface and is where the primitives
were verified; the split, the resize and the disk writes can only be checked in
`pnpm tauri dev`.
