import { basename } from "@/app/format";
import type {
  ConversionProfile,
  ConversionRoute,
  JobId,
  ScannedConversionFile,
  SecretId,
  SecretStatus,
} from "@/app/types";

/// No local Format variant exists for these, so they route to Datalab even if
/// a service advertises them. Images are not here: Vision is macOS 26 and up.
/// Must stay identical to `PERMANENT_DIRECT_FORMATS` in `lib.rs`.
const PERMANENT_DIRECT_EXTENSIONS = new Set(["html", "htm"]);

export type ConversionCapabilities =
  | { state: "idle" | "loading" }
  /// The host's own reason: nothing in Settings moves the service.
  | { state: "unavailable"; message?: string }
  | { state: "ready"; acceptingJobs: boolean; inputFormats: readonly string[] };

export type RouteBlockReason =
  | "capabilities_pending"
  | "backend_unavailable"
  | "backend_not_accepting"
  | "local_only_requires_remote";

export interface BlockedConversionFile {
  file: ScannedConversionFile;
  reason: RouteBlockReason;
}

export interface ConversionRoutePlan {
  direct: ScannedConversionFile[];
  backend: ScannedConversionFile[];
  blocked: BlockedConversionFile[];
  needsDatalabKey: boolean;
  needsBackendToken: boolean;
}

interface PlanConversionRoutesOptions {
  files: readonly ScannedConversionFile[];
  route: ConversionRoute;
  profile: ConversionProfile;
  capabilities: ConversionCapabilities;
  skipAlreadyDone: boolean;
}

export function planConversionRoutes({
  files,
  route,
  profile,
  capabilities,
  skipAlreadyDone,
}: PlanConversionRoutesOptions): ConversionRoutePlan {
  const plan: ConversionRoutePlan = {
    direct: [],
    backend: [],
    blocked: [],
    needsDatalabKey: false,
    needsBackendToken: false,
  };
  const supported =
    capabilities.state === "ready"
      ? new Set(capabilities.inputFormats.map(normalizeMediaType))
      : null;

  for (const file of files) {
    if (skipAlreadyDone && file.reuse !== "pending") continue;

    if (route === "direct") {
      plan.direct.push(file);
      continue;
    }

    if (isPermanentDirect(file.sourcePath)) {
      if (profile === "local_only") {
        plan.blocked.push({ file, reason: "local_only_requires_remote" });
      } else {
        plan.direct.push(file);
      }
      continue;
    }

    if (capabilities.state === "unavailable") {
      plan.blocked.push({ file, reason: "backend_unavailable" });
      continue;
    }
    if (capabilities.state !== "ready") {
      plan.blocked.push({ file, reason: "capabilities_pending" });
      continue;
    }

    if (!supported?.has(normalizeMediaType(file.mediaType))) {
      if (profile === "local_only") {
        plan.blocked.push({ file, reason: "local_only_requires_remote" });
      } else {
        plan.direct.push(file);
      }
      continue;
    }

    if (!capabilities.acceptingJobs) {
      plan.blocked.push({ file, reason: "backend_not_accepting" });
      continue;
    }
    plan.backend.push(file);
  }

  plan.needsDatalabKey = plan.direct.length > 0;
  plan.needsBackendToken = plan.backend.length > 0;
  return plan;
}

/// The keys the plan in force actually spends. `needsDatalabKey` is the direct
/// provider, which is Rev.ai for Transcribe: a run routed to the sidecar needs
/// neither, and demanding one disables the button for local transcription.
export function missingConversionCredentials(
  jobType: JobId,
  plan: Pick<ConversionRoutePlan, "needsDatalabKey" | "needsBackendToken">,
  secrets: SecretStatus,
): SecretId[] {
  const direct: SecretId = jobType === "transcribe" ? "revai" : "datalab";
  const missing: SecretId[] = [];
  if (plan.needsDatalabKey && !secrets[direct]) missing.push(direct);
  if (plan.needsBackendToken && !secrets.backend) missing.push("backend");
  return missing;
}

function normalizeMediaType(mediaType: string): string {
  return mediaType.split(";", 1)[0]?.trim().toLowerCase() ?? "";
}

function isPermanentDirect(sourcePath: string): boolean {
  const fileName = basename(sourcePath);
  const dot = fileName.lastIndexOf(".");
  if (dot < 0) return false;
  return PERMANENT_DIRECT_EXTENSIONS.has(fileName.slice(dot + 1).toLowerCase());
}
