import { ACTIVE, type Job } from "@/app/types";

/// A run in flight, or null when nothing is running.
///
/// The title bar's centre slot holds the workspace nav at all times, so the
/// strip no longer names the surface: the nav already says where you are. What
/// is left for the bar to compute is the run, which reports from the corner so
/// it cannot displace the nav for the length of a 200-file batch.
export interface BarStatus {
  done: number;
  total: number;
  /// When the run started. Null until its first file does.
  since: number | null;
}

export function barStatus(jobs: readonly Job[]): BarStatus | null {
  const active = jobs.filter((job) => ACTIVE.includes(job.status));
  if (active.length === 0) return null;
  // Elapsed belongs to the run, not to whichever file happens to be first, so
  // it counts from the earliest start and keeps climbing as files hand over.
  const starts = jobs.map((job) => job.startedAt).filter((at): at is number => at !== null);
  return {
    done: jobs.length - active.length,
    total: jobs.length,
    since: starts.length > 0 ? Math.min(...starts) : null,
  };
}

/// `4 / 12` with the counter padded to the total's width. Unpadded, the tenth
/// completion widens the counter and shoves the timer sideways (UI.md rule 2:
/// values light up, they do not appear).
export function runCounter(done: number, total: number): string {
  return `${String(done).padStart(String(total).length, " ")} / ${total}`;
}
