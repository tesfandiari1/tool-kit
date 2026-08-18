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
