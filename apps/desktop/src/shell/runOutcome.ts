import type { Job } from "@/app/types";

/// What a run produced, as opposed to what exists.
///
/// Every count on the run screen is derived from the whole job list, which is
/// the right answer for a fresh run and the wrong one everywhere else.
/// `run_pipeline` clears the queue, but `convert_one` appends to it and
/// `retry_job` rewrites a row in place, and neither bumps the generation. So
/// one file converted from the tree after a 200-file batch reports 201 jobs
/// and 200 done. The serialized `Job` carries no generation, so the only way
/// to name this run's work is to diff against what was already terminal when
/// it started.

/// The rows a run must not claim. Failed counts as terminal: a retry moves one
/// back out of this set, which is what lets retrying a single file open its
/// result the way converting a single file does.
export function terminalIds(jobs: Job[]): Set<number> {
  return new Set(
    jobs.filter((j) => j.status === "done" || j.status === "failed").map((j) => j.id),
  );
}

/// The results this run finished. Failures are not results, so they are not
/// here: a run that fails its only file has nothing to open.
export function newlyDone(before: ReadonlySet<number>, jobs: Job[]): Job[] {
  return jobs.filter((j) => j.status === "done" && !before.has(j.id));
}

/// The one result worth opening on its own, or null.
///
/// One file is a request to read it. Twenty are a batch, and opening twenty
/// tabs, or picking one of them for the user, is not what they asked for.
export function autoOpenTarget(before: ReadonlySet<number>, jobs: Job[]): Job | null {
  const fresh = newlyDone(before, jobs);
  if (fresh.length !== 1) return null;
  // Nothing to read: the job finished but wrote no path we can open.
  return fresh[0].outputPath === null ? null : fresh[0];
}
