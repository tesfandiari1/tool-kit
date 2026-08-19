import type { ConversionProfile, ConversionRoute, JobId, Scan } from "@/app/types";

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

/// The native backend route deliberately does not reuse direct-provider
/// history yet: a backend result is not interchangeable with a Datalab result.
/// Keep the UI's counts and route plan on that same conservative policy.
export function effectiveSkipAlreadyDone(
  jobType: JobId,
  conversionRoute: ConversionRoute,
  requested: boolean,
): boolean {
  return requested && !(jobType === "convert" && conversionRoute === "backend");
}

export function largeRunConfirmation({
  totalFiles,
  provider,
  backendFiles,
  directFiles,
  highAccuracy,
  profile,
}: {
  totalFiles: number;
  provider: string;
  backendFiles: number;
  directFiles: number;
  highAccuracy: boolean;
  profile: ConversionProfile;
}): string {
  const accuracy =
    highAccuracy && directFiles > 0
      ? ", with high-accuracy convert on (slower, more credits per page)"
      : "";

  if (backendFiles === 0) {
    return `This will send ${totalFiles} files to ${provider}${accuracy}.\n\nEach file uses ${provider} credits.`;
  }
  if (directFiles === 0) {
    const fallbackAccuracy = highAccuracy
      ? " with high-accuracy convert on (slower, more credits per page)"
      : "";
    const cost =
      profile === "local_only"
        ? `Local only forbids ${provider} fallback, so no ${provider} credits are planned.`
        : `Files that require remote fallback may also use ${provider} credits${fallbackAccuracy}.`;
    return `This will send ${totalFiles} files to your conversion backend.\n\n${cost}`;
  }
  const fallback =
    profile === "local_only"
      ? ` Backend files cannot fall back to ${provider} under Local only.`
      : ` Backend files may also use ${provider} credits if remote fallback is required.`;
  return (
    `This will send ${backendFiles} files to your conversion backend and ${directFiles} files to ${provider}${accuracy}.` +
    `\n\nThe ${directFiles} files routed directly to ${provider} use provider credits.${fallback}`
  );
}

export function runServiceDescription({
  description,
  provider,
  jobType,
  conversionRoute,
  profile,
}: {
  description: string;
  provider: string;
  jobType: JobId;
  conversionRoute: ConversionRoute;
  profile: ConversionProfile;
}): string {
  if (jobType !== "convert" || conversionRoute === "direct") {
    return `${description}, via ${provider}`;
  }
  if (profile === "local_only") {
    return `${description}, via your conversion backend only`;
  }
  return `${description}, via your conversion backend with ${provider} fallback where required`;
}
