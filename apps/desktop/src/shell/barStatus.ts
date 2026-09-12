import { ACTIVE, type Job } from "@/app/types";

/// A run in flight, or null. It reports from the bar's corner.
export interface BarStatus {
  done: number;
  total: number;
  /// When the run started. Null until its first file does.
  since: number | null;
}

/// This run's rows, never the whole queue, which would read "200 / 201" for
/// one tree convert after a batch. See `runOutcome.ts`.
export function barStatus(jobs: readonly Job[]): BarStatus | null {
  const active = jobs.filter((job) => ACTIVE.includes(job.status));
  if (active.length === 0) return null;
  // Elapsed belongs to the run, so it counts from the earliest start.
  const starts = jobs.map((job) => job.startedAt).filter((at): at is number => at !== null);
  return {
    done: jobs.length - active.length,
    total: jobs.length,
    since: starts.length > 0 ? Math.min(...starts) : null,
  };
}

/// Padded to the total's width, or the tenth completion shoves the timer
/// sideways (UI.md rule 2).
export function runCounter(done: number, total: number): string {
  return `${String(done).padStart(String(total).length, " ")} / ${total}`;
}
