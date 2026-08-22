/// Every window number lives here. `App.tsx`, `useZoom.ts` and
/// `tauri.conf.json` agree with this module. Nothing else hardcodes a window
/// size.

/// The size the window opens at, before the settings load says where the
/// workspace is. Not a phase: a beat later the window grows to `WORKSPACE` or
/// to `ONBOARDING` and never comes back.
///
/// No code reads this, and none should. It is the record of what
/// `tauri.conf.json` opens the window at, kept here because JSON cannot import
/// a module and the two numbers still have to agree. Change one, change both.
export const INITIAL = { width: 560, minWidth: 480, minHeight: 460 } as const;

/// The workspace is resizable inside these bounds. `minWidth` is derived, not
/// picked: `SPLIT.start`% of it has to clear `SPLIT.minStart`, or the pixel
/// floor beats the ratio at every width. 26% of 960 is 249px, which clears the
/// 240px sidebar floor and leaves the main area 710px.
export const WORKSPACE = { minWidth: 960, minHeight: 560, width: 1180, height: 780 } as const;

/// First run. Deliberately between the two: wide enough that one sentence and
/// one control read as the whole window, and small enough that the workspace
/// arriving afterwards is a visible change rather than nothing happening.
export const ONBOARDING = { width: 720, height: 520, minWidth: 640, minHeight: 480 } as const;

/// The sidebar owns the left edge and the main area owns the rest. The split
/// never collapses: the sidebar is the app's one fixed landmark, so the nav
/// swaps the end pane and the start pane stays where the user left it.
///
/// `minStart` is what the *tree* needs, not what the run column needs. Run and
/// History live in the end pane now, so the widest thing the start pane ever
/// draws is a tree row: four levels of --s3 indent is 48px, plus the row's own
/// chrome, which leaves about 160px of name column at the floor. The indent cap
/// is what holds that. Uncapped, a deep folder walks its label off the edge.
///
/// `minEnd` is a floor, not a preference. It has to clear the run column's job
/// segmented, which sets CONVERT and TRANSCRIBE in tracked mono uppercase at
/// about 185px, plus the panel's --s4 padding and the column's --s5 inset. 50%
/// of `WORKSPACE.minWidth` is 480px, comfortably past that, and the same floor
/// is what keeps a document readable.
export const SPLIT = { start: 26, minStart: "240px", minEnd: "50%" } as const;

/// Ten points a step. `useZoom` walks these as a ladder of fixed rungs.
export const ZOOM = { min: 0.5, max: 2, step: 0.1, default: 1 } as const;
