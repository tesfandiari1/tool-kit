import type { JobId, Scan } from "@/app/types";

/// The job a selection implies, with the selection it answers. `answered` is
/// the point: the scan round-trips through an empty result on every unrelated
/// refresh, and detecting again there writes over a manual click. One answer
/// per selection, so a new drop still re-detects.
export function autodetectJob(
  inputs: readonly string[],
  answered: string | null,
  scan: Pick<Scan, "convert" | "transcribe">,
): { selection: string; jobType: JobId } | null {
  if (scan.convert === 0 && scan.transcribe === 0) return null;
  const selection = JSON.stringify(inputs);
  if (selection === answered) return null;
  return { selection, jobType: scan.transcribe > scan.convert ? "transcribe" : "convert" };
}

/// What a selection runs, testable without mounting the app: wrong here
/// either repeats or silently skips work.
export interface RunPlan {
  skipping: number;
  copying: number;
  toRun: number;
  /// Sent again with skip off. `write_output` refuses to clobber, so each
  /// lands as a numbered copy. Zero when skip is on.
  colliding: number;
}

export function planRun(
  jobType: JobId,
  inputCount: number,
  scan: Pick<Scan, "alreadyHereTranscribe" | "reusableTranscribe">,
  skipAlreadyDone: boolean,
): RunPlan {
  // Convert never reuses a result.
  const alreadyHere = jobType === "transcribe" ? scan.alreadyHereTranscribe : 0;
  const reusable = jobType === "transcribe" ? scan.reusableTranscribe : 0;
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
  return "Run";
}
