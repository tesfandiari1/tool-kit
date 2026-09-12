/// A document open in the workspace pane. `id` is the output path, which is
/// what stops the same result opening twice from two doors.
export interface OpenDoc {
  id: string;
  title: string;
  subtitle: string | null;
  text: string;
  revealPath: string | null;
  save: SaveState;
  /// Offered back on save, so an outside edit is refused, not clobbered.
  mtimeMs: number;
}

/// These map onto the design system's dot tones. `clean` is silence.
export type SaveState = "clean" | "edited" | "saving" | "saved" | "error";

export type DocMode = "read" | "edit";

/// `error` counts: a refused write leaves the edit in memory.
export function isDirty(s: SaveState) {
  return s === "edited" || s === "error";
}

const TONES = {
  clean: "idle",
  edited: "queued",
  saving: "live",
  saved: "pass",
  error: "fault",
} as const;

/// The one line that keeps `@ui` free of any knowledge of saving.
export function saveTone(s: SaveState) {
  return TONES[s];
}

const NOTES: Record<SaveState, string | null> = {
  clean: null,
  edited: "Unsaved changes",
  saving: "Saving…",
  saved: "Saved",
  error: "Could not save",
};

export function saveNote(s: SaveState) {
  return NOTES[s];
}
