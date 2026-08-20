import type { OnboardingConversionMode, Settings } from "@/app/types";

/// First run asks one question and two settings answer it. Keeping the mapping
/// here rather than in the gate is what lets the copy on screen say "this Mac"
/// while the app still speaks in routes and profiles.
///
/// Where the service runs is not among them. It is the one in the bundle unless
/// a deployment dropped `backend-override.json`, which no answer here can do.
export function conversionPatch(mode: OnboardingConversionMode): Partial<Settings> {
  switch (mode) {
    case "local":
      // Local only forbids the remote fallback outright: choosing this Mac has
      // to mean the bytes cannot leave it, not that they usually don't.
      return { conversionRoute: "backend", conversionProfile: "local_only" };
    case "cloud":
      return { conversionRoute: "direct", conversionProfile: "standard" };
    default: {
      const _exhaustive: never = mode;
      return _exhaustive;
    }
  }
}

/// What choosing each mode costs the user, in their words. Shown under the
/// control so the consequence is on screen before the choice is made.
export function conversionConsequence(mode: OnboardingConversionMode): string {
  switch (mode) {
    case "local":
      return "Files are converted on this Mac. Nothing is uploaded, and no API key is needed.";
    case "cloud":
      return "Files are uploaded to Datalab for conversion. You will add an API key in Settings.";
    default: {
      const _exhaustive: never = mode;
      return _exhaustive;
    }
  }
}
