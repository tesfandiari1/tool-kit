/// One opened result. The live run and history both produce this so the
/// viewer does not care which door you came through.
export interface ThreadDoc {
  title: string;
  subtitle: string | null;
  text: string | null;
  revealPath: string | null;
}

/// A document open in the workspace pane.
///
/// `id` is the output path: it is already unique per document, and keying on
/// it is what stops the same result opening twice when it is clicked in both
/// the queue and the history.
export interface OpenDoc extends ThreadDoc {
  id: string;
  text: string;
  save: SaveState;
  /// The file's modification time when this text was read. A save offers it
  /// back so a file changed outside the app is refused rather than clobbered.
  mtimeMs: number;
}

/// Where an edit currently stands with the file on disk.
///
/// These map onto the design system's existing dot tones and need no new
/// vocabulary: `edited` is the hollow ghost ring the system already uses for
/// queued, `saving` is amber because writing to disk is work in flight
/// exactly as a provider call is, and `saved` is the same green as a passed
/// job. Silence — `clean` — means nothing has happened.
export type SaveState = "clean" | "edited" | "saving" | "saved" | "error";

/// Which surface the document is showing.
export type DocMode = "read" | "edit";

/// Whether the document holds work the file on disk does not.
///
/// `error` counts: a refused write means the edit is still only in memory, so
/// closing it is as destructive as closing an unsaved one. The close confirm
/// and the tab's dirty mark both ask this, so a document whose save was
/// refused is marked as unsaved rather than looking settled.
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

/// The one line that keeps `@ui` free of any knowledge of saving. Same trick
/// `RunView` uses to map five job statuses onto three status tones.
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
