import type { Job } from "@/app/types";

/// What a run produced, as opposed to what exists. `convert_one` appends to the
/// queue and `retry_job` rewrites a row, and the serialized `Job` carries no
/// generation, so this run's work is a diff against what was terminal.

/// Failed counts as terminal, and a retry moves one back out.
export function terminalIds(jobs: Job[]): Set<number> {
  return new Set(
    jobs.filter((j) => j.status === "done" || j.status === "failed").map((j) => j.id),
  );
}

/// Failures are not results: a run that fails its only file opens nothing.
export function newlyDone(before: ReadonlySet<number>, jobs: Job[]): Job[] {
  return jobs.filter((j) => j.status === "done" && !before.has(j.id));
}

/// The folders a run wrote into, with the ancestors that have to be open for
/// them to show. A run writes beside its source, so the active project is
/// often not one of them. Paths outside the workspace are skipped.
export function resultDirs(workspacePath: string, done: Job[]): string[] {
  const prefix = `${workspacePath}/`;
  const dirs = new Set<string>();
  for (const job of done) {
    if (job.outputPath?.startsWith(prefix) !== true) continue;
    const parts = job.outputPath.slice(prefix.length).split("/");
    parts.pop();
    let rel = "";
    for (const part of parts) {
      rel = rel === "" ? part : `${rel}/${part}`;
      dirs.add(rel);
    }
  }
  return [...dirs];
}

/// One file is a request to read it. Twenty are a batch.
export function autoOpenTarget(before: ReadonlySet<number>, jobs: Job[]): Job | null {
  const fresh = newlyDone(before, jobs);
  if (fresh.length !== 1) return null;
  return fresh[0].outputPath === null ? null : fresh[0];
}
