import type {
  ConversionProfile,
  ConversionRoute,
  ScannedConversionFile,
  SecretStatus,
} from "@/app/types";

/// AnyDoc has no local Format variant for these file types. They are an
/// intentional permanent Datalab route, even if a future service advertises
/// their MIME types by mistake.
const PERMANENT_DIRECT_EXTENSIONS = new Set([
  "png",
  "jpg",
  "jpeg",
  "webp",
  "tiff",
  "tif",
  "gif",
  "bmp",
  "html",
  "htm",
]);

export type ConversionCapabilities =
  | { state: "idle" | "loading" }
  | { state: "unavailable" }
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

export function missingConversionCredentials(
  plan: Pick<ConversionRoutePlan, "needsDatalabKey" | "needsBackendToken">,
  secrets: SecretStatus,
): ("datalab" | "backend")[] {
  const missing: ("datalab" | "backend")[] = [];
  if (plan.needsDatalabKey && !secrets.datalab) missing.push("datalab");
  if (plan.needsBackendToken && !secrets.backend) missing.push("backend");
  return missing;
}

function normalizeMediaType(mediaType: string): string {
  return mediaType.split(";", 1)[0]?.trim().toLowerCase() ?? "";
}

function isPermanentDirect(sourcePath: string): boolean {
  const pathParts = sourcePath.replace(/\\/gu, "/").split("/");
  const fileName = pathParts[pathParts.length - 1] ?? "";
  const dot = fileName.lastIndexOf(".");
  if (dot < 0) return false;
  return PERMANENT_DIRECT_EXTENSIONS.has(fileName.slice(dot + 1).toLowerCase());
}
