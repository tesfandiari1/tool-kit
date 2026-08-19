import { ACTIVE, type Job } from "@/app/types";

/// What the title bar says.
///
/// The strip beside the traffic lights is the one piece of chrome that survives
/// every phase, so it reports where you are and what is running rather than the
/// app's name. The name is already in the menu bar, the Dock, and ⌘-Tab.
export type BarStatus =
  /// A run is in flight. The only state that outranks knowing where you are.
  | { kind: "run"; done: number; total: number; since: number | null }
  /// Otherwise the strip names the surface: the open document, or the panel
  /// opened over the launcher.
  | { kind: "label"; text: string; variant: "view" | "document" };

export interface BarStatusInput {
  view: "run" | "history" | "settings";
  jobs: readonly Job[];
  /// The open document's file name, when the workspace is showing one.
  documentName: string | null;
}

export function barStatus({ view, jobs, documentName }: BarStatusInput): BarStatus {
  const active = jobs.filter((job) => ACTIVE.includes(job.status));
  if (active.length > 0) {
    // Elapsed belongs to the run, not to whichever file happens to be first, so
    // it counts from the earliest start and keeps climbing as files hand over.
    const starts = jobs.map((job) => job.startedAt).filter((at): at is number => at !== null);
    return {
      kind: "run",
      done: jobs.length - active.length,
      total: jobs.length,
      since: starts.length > 0 ? Math.min(...starts) : null,
    };
  }

  if (view === "history") return { kind: "label", text: "History", variant: "view" };
  if (view === "settings") return { kind: "label", text: "Settings", variant: "view" };
  if (documentName) return { kind: "label", text: documentName, variant: "document" };
  return { kind: "label", text: "Run", variant: "view" };
}

/// `4 / 12` with the counter padded to the total's width. Unpadded, the tenth
/// completion widens the counter and shoves the timer sideways (UI.md rule 2:
/// values light up, they do not appear).
export function runCounter(done: number, total: number): string {
  return `${String(done).padStart(String(total).length, " ")} / ${total}`;
}
