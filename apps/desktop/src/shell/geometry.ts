/// Every window number. Nothing else hardcodes a window size.

/// The size the window opens at, hidden, and never paints. No code reads this:
/// it records what `tauri.conf.json` says, because JSON cannot import a module.
/// Change one, change both.
export const INITIAL = { width: 560, minWidth: 480, minHeight: 460 } as const;

/// `minWidth` is derived, not picked: `SPLIT.start`% of it has to clear
/// `SPLIT.minStart`, or the pixel floor beats the ratio at every width.
export const WORKSPACE = { minWidth: 960, minHeight: 560, width: 1180, height: 780 } as const;

/// First run, between the two, so the workspace arriving is a visible change.
export const ONBOARDING = { width: 720, height: 520, minWidth: 640, minHeight: 480 } as const;

/// The split never collapses: the sidebar is the app's one fixed landmark.
/// `minStart` is what a four-level tree row needs, with about 160px of name
/// left at the floor. `minEnd` has to clear the run column's job segmented.
export const SPLIT = { start: 26, minStart: "240px", minEnd: "50%" } as const;

/// Ten points a step, walked as fixed rungs. See `useZoom`.
export const ZOOM = { min: 0.5, max: 2, step: 0.1, default: 1 } as const;
