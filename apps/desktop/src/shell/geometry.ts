/// Every window number lives here. `App.tsx`, `useFitWindow.ts`, `useZoom.ts`
/// and `tauri.conf.json` agree with this module. Nothing else hardcodes a
/// window size.

/// The launcher is one fixed width. Its height is its content's height.
export const LAUNCHER = { width: 560, minWidth: 480, minHeight: 460 } as const;

/// The workspace is resizable inside these bounds. `minWidth` is derived, not
/// picked: `SPLIT.start`% of it has to clear `SPLIT.minStart`, or the pixel
/// floor beats the ratio at every width. 33% of 960 is 317px, which clears the
/// 300px floor and still leaves the document 643px, about 68 characters.
export const WORKSPACE = { minWidth: 960, minHeight: 560, width: 1180, height: 780 } as const;

/// First run. Deliberately between the two: wide enough that one sentence and
/// one control read as the whole window, and small enough that the workspace
/// arriving afterwards is a visible change rather than nothing happening.
export const ONBOARDING = { width: 720, height: 520, minWidth: 640, minHeight: 480 } as const;

/// Breathing room so a full-height window never sits flush against the work area.
export const SCREEN_MARGIN = 48;

/// The document owns two thirds. `minEnd` is a floor, not a preference: the
/// pane exists to be read.
///
/// `minStart` is what the run column's widest row actually needs. The job
/// segmented sets CONVERT and TRANSCRIBE in tracked mono uppercase, about
/// 185px together, plus the panel's --s4 padding and the column's --s5 inset.
/// Below 300px that control clips, which is what a narrower floor produced.
export const SPLIT = { start: 33, minStart: "300px", minEnd: "50%" } as const;

/// Ten points a step. `useZoom` walks these as a ladder of fixed rungs.
export const ZOOM = { min: 0.5, max: 2, step: 0.1, default: 1 } as const;
