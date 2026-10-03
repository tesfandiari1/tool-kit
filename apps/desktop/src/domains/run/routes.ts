import type { ScannedConversionFile } from "@/app/types";

export type ConversionCapabilities =
  | { state: "loading" }
  /// The host's own reason: nothing in Settings moves the service.
  | { state: "unavailable"; message?: string }
  | { state: "ready"; acceptingJobs: boolean; inputFormats: readonly string[] };

export type RouteBlockReason =
  | "capabilities_pending"
  | "backend_unavailable"
  | "backend_not_accepting"
  | "not_supported";

export interface BlockedConversionFile {
  file: ScannedConversionFile;
  reason: RouteBlockReason;
}

export interface ConversionRoutePlan {
  backend: ScannedConversionFile[];
  blocked: BlockedConversionFile[];
}

interface PlanConversionRoutesOptions {
  files: readonly ScannedConversionFile[];
  capabilities: ConversionCapabilities;
  skipAlreadyDone: boolean;
}

/// Every file goes to the local service or nowhere: nothing leaves the Mac.
export function planConversionRoutes({
  files,
  capabilities,
  skipAlreadyDone,
}: PlanConversionRoutesOptions): ConversionRoutePlan {
  const plan: ConversionRoutePlan = { backend: [], blocked: [] };
  const supported =
    capabilities.state === "ready"
      ? new Set(capabilities.inputFormats.map(normalizeMediaType))
      : null;

  for (const file of files) {
    if (skipAlreadyDone && file.reuse !== "pending") continue;

    if (capabilities.state === "unavailable") {
      plan.blocked.push({ file, reason: "backend_unavailable" });
      continue;
    }
    if (capabilities.state !== "ready") {
      plan.blocked.push({ file, reason: "capabilities_pending" });
      continue;
    }

    if (!supported?.has(normalizeMediaType(file.mediaType))) {
      plan.blocked.push({ file, reason: "not_supported" });
      continue;
    }

    if (!capabilities.acceptingJobs) {
      plan.blocked.push({ file, reason: "backend_not_accepting" });
      continue;
    }
    plan.backend.push(file);
  }

  return plan;
}

function normalizeMediaType(mediaType: string): string {
  return mediaType.split(";", 1)[0]?.trim().toLowerCase() ?? "";
}
