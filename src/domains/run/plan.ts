import type { JobId, Scan } from "@/app/types";

/// The buckets a selection costs. Extracted so the numbers the Run button
/// promises can be tested without mounting the app — getting them wrong
/// either over-bills or silently skips work.
export interface RunPlan {
  skipping: number;
  copying: number;
  toRun: number;
  /// Files that already have an output in this folder and will still be sent,
  /// because skip is off. `write_output` refuses to clobber, so each one lands
  /// as a numbered copy (`report (1).md`) rather than replacing the edit.
  /// Zero when skip is on: those files are `skipping` instead.
  colliding: number;
}

export function planRun(
  jobType: JobId,
  inputCount: number,
  scan: Pick<
    Scan,
    "alreadyHereConvert" | "alreadyHereTranscribe" | "reusableConvert" | "reusableTranscribe"
  >,
  skipAlreadyDone: boolean,
): RunPlan {
  const alreadyHere = jobType === "transcribe" ? scan.alreadyHereTranscribe : scan.alreadyHereConvert;
  const reusable = jobType === "transcribe" ? scan.reusableTranscribe : scan.reusableConvert;
  const skipping = skipAlreadyDone ? Math.min(alreadyHere, inputCount) : 0;
  const copying = skipAlreadyDone ? Math.min(reusable, inputCount - skipping) : 0;
  const toRun = Math.max(0, inputCount - skipping - copying);
  const colliding = skipAlreadyDone ? 0 : Math.min(alreadyHere, inputCount);
  return { skipping, copying, toRun, colliding };
}

export function canStartRun({
  hasInputs,
  hasOutput,
  hasKey,
  toRun,
  copying,
  running,
  starting,
}: {
  hasInputs: boolean;
  hasOutput: boolean;
  hasKey: boolean;
  toRun: number;
  copying: number;
  running: boolean;
  starting: boolean;
}): boolean {
  return hasInputs && hasOutput && hasKey && toRun + copying > 0 && !running && !starting;
}

export function runButtonLabel(verb: string, toRun: number, copying: number): string {
  if (toRun > 0) return `${verb} ${toRun} file${toRun > 1 ? "s" : ""}`;
  if (copying > 0) return `Copy ${copying} result${copying > 1 ? "s" : ""}`;
  return "Run pipeline";
}
