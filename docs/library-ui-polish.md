# Library UI polish (2026-08-20)

Impeccable craft pass on the workspace shell after first-launch onboarding and
library home landed. These were UX/UI issues identified in the code review
status check, not north-star product gaps (import pipeline, real document tabs,
index counts).

## Issues and fixes

| # | Issue | Decision | Files |
|---|-------|----------|-------|
| 1 | Title bar nav used ghost `Button`s with custom `.bar-nav__item.is-active` CSS instead of the design-system `Segmented` primitive (onboarding and Run already use it) | Replaced with `Segmented` (`size="sm"`), four options: Library, Run, Settings, History | `WorkspaceViewNav.tsx`, `App.css` |
| 2 | History appeared in both centre nav and top-right corner icon | Removed corner History in `libraryMode`; launcher keeps History + Settings icons | `App.tsx` |
| 3 | No active state when `view === "library"` (none of Run \| Settings \| History selected) | Library is now a fourth segment; selecting it matches library home | `WorkspaceViewNav.tsx` |
| 4 | Welcome header used cobalt primary "Convert" while title bar pushes Run (two conversion paths, different weight) | Changed to link-style **Open Run** beside Reveal | `ProjectWorkspace.tsx` |
| 5 | Workspace head in sidebar had no active treatment on library home | `libraryHome` prop threads from `App.tsx` → `LibraryShell` → `ProjectSidebar`; `.lib-side__workspace.is-active` uses `--surface-raised` like project rows | `App.tsx`, `LibraryShell.tsx`, `ProjectSidebar.tsx`, `App.css` |
| 6 | Embedded Settings used 34rem measure + `--s5` gutters inside a narrow library column at min window width (960px) | Unlock `max-width` on settings body; reduce horizontal padding to `--s3` for embedded settings and history in `.library-main__panel` | `App.css`, `HistoryPanel.tsx` (`history-panel` class hook) |
| 7 | Redundant back-to-Library arrow after Library entered the Segmented nav | Removed corner arrow; Library segment is the return path | `App.tsx` |

## CSS notes

- **`.bar-nav .ui-seg__item`**: `flex: none` + `padding-inline: var(--s3)` because
  `Segmented` defaults to stretching items across a panel; the title bar centre
  column is content-sized.
- **`html:not(.fit-window) .bar-status { min-height: var(--s6) }`**: the sm
  Segmented control is 30px; the bar previously reserved `--s5` (24px), which
  reflowed the workspace when swapping nav for a document name.
- **Embedded panel padding**: scoped under `.library-main__panel` so launcher
  Settings/History keep their existing measure.

## Explicitly not in this pass

Product work from `docs/north-star.md` / `docs/STATUS.md`:

- Import into `{project}/_sources/` and real document tabs from `index.db`
- Sidebar document counts (ghost slots only today)
- `welcome.md` on disk (in-memory pinned tab only)
- Security-scoped workspace bookmark
- Launcher-phase dead code removal (harmless until workspace-less path is deleted)

## Verify

```bash
pnpm check
pnpm tauri dev   # exercise: Library segment, Run/Settings/History swap, embedded Settings at min width
```
