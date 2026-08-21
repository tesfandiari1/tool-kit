import { ACTIVE, type Job, type View } from "@/app/types";

/// What the title bar says.
///
/// The strip beside the traffic lights is the one piece of chrome that survives
/// every phase, so it reports where you are and what is running rather than the
/// app's name. The name is already in the menu bar, the Dock, and ⌘-Tab.
export type BarStatus =
  /// A run is in flight. The only state that outranks knowing where you are.
  | { kind: "run"; done: number; total: number; since: number | null }
  /// Otherwise the strip names the surface: the workspace you are in, the open
  /// document, or the panel opened over either.
  ///
  /// `view` sets in the label treatment; the other two are names the user
  /// chose, so they keep their own case.
  | { kind: "label"; text: string; variant: "view" | "document" | "workspace" };

export interface BarStatusInput {
  view: View;
  jobs: readonly Job[];
  /// The open document's file name, when the workspace is showing one.
  documentName: string | null;
  /// The open workspace's folder name. Null before one exists.
  workspaceName: string | null;
}

export function barStatus({ view, jobs, documentName, workspaceName }: BarStatusInput): BarStatus {
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
  // Settings has no case here on purpose. It is a sheet over the window, and a
  // real sheet does not rename the window it hangs off, so the strip keeps
  // naming the surface underneath.
  if (documentName) return { kind: "label", text: documentName, variant: "document" };
  // The library is a place rather than a panel, so the strip names the
  // workspace you are in.
  if (view === "library" && workspaceName) {
    return { kind: "label", text: workspaceName, variant: "workspace" };
  }
  if (view === "library") return { kind: "label", text: "Library", variant: "view" };
  return { kind: "label", text: "Run", variant: "view" };
}

/// `4 / 12` with the counter padded to the total's width. Unpadded, the tenth
/// completion widens the counter and shoves the timer sideways (UI.md rule 2:
/// values light up, they do not appear).
export function runCounter(done: number, total: number): string {
  return `${String(done).padStart(String(total).length, " ")} / ${total}`;
}
