import type { OnboardingConversionMode, Settings } from "@/app/types";

/// One question, two settings, so the copy can say "this Mac" while the app
/// speaks in routes and profiles. Where the service runs is not among them.
const PATCH: Record<OnboardingConversionMode, Partial<Settings>> = {
  // Local only forbids the fallback: the bytes cannot leave this Mac.
  local: { conversionRoute: "backend", conversionProfile: "local_only" },
  cloud: { conversionRoute: "direct", conversionProfile: "standard" },
};

export function conversionPatch(mode: OnboardingConversionMode): Partial<Settings> {
  return PATCH[mode];
}

/// The consequence, on screen before the choice.
const CONSEQUENCE: Record<OnboardingConversionMode, string> = {
  local: "Files are converted on this Mac. Nothing is uploaded, and no API key is needed.",
  cloud: "Files are uploaded to Datalab for conversion. You will add an API key in Settings.",
};

export function conversionConsequence(mode: OnboardingConversionMode): string {
  return CONSEQUENCE[mode];
}
