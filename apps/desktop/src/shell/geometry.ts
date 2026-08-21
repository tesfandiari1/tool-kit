/// Every window number lives here. `App.tsx`, `useZoom.ts` and
/// `tauri.conf.json` agree with this module. Nothing else hardcodes a window
/// size.

/// The size the window opens at, before the settings load says where the
/// workspace is. Not a phase: a beat later the window grows to `WORKSPACE` or
/// to `ONBOARDING` and never comes back. `tauri.conf.json` carries the same
/// three numbers, so change both together.
export const INITIAL = { width: 560, minWidth: 480, minHeight: 460 } as const;

/// The workspace is resizable inside these bounds. `minWidth` is derived, not
/// picked: `SPLIT.start`% of it has to clear `SPLIT.minStart`, or the pixel
/// floor beats the ratio at every width. 33% of 960 is 317px, which clears the
/// 300px floor and still leaves the document 643px, about 68 characters.
export const WORKSPACE = { minWidth: 960, minHeight: 560, width: 1180, height: 780 } as const;

/// First run. Deliberately between the two: wide enough that one sentence and
/// one control read as the whole window, and small enough that the workspace
/// arriving afterwards is a visible change rather than nothing happening.
export const ONBOARDING = { width: 720, height: 520, minWidth: 640, minHeight: 480 } as const;

/// The document owns two thirds. `minEnd` is a floor, not a preference: the
/// pane exists to be read.
///
/// The start pane holds one view at a time at its full width, so `minStart` is
/// the larger of the two things that go in it.
///
/// The run column is the binding one. Its job segmented sets CONVERT and
/// TRANSCRIBE in tracked mono uppercase, about 185px together, plus the panel's
/// --s4 padding and the column's --s5 inset. Below 300px that control clips,
/// which is what a narrower floor produced.
///
/// The tree stays under that, and the indent cap is what keeps it there: four
/// levels of --s3 is 48px, which leaves about 200px of name column at this
/// floor. Uncapped, a deep folder would walk its label off the edge instead.
export const SPLIT = { start: 33, minStart: "300px", minEnd: "50%" } as const;

/// Ten points a step. `useZoom` walks these as a ladder of fixed rungs.
export const ZOOM = { min: 0.5, max: 2, step: 0.1, default: 1 } as const;
