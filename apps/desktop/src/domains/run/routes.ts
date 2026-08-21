import type {
  ConversionProfile,
  ConversionRoute,
  ScannedConversionFile,
  SecretStatus,
} from "@/app/types";

/// AnyDoc has no local Format variant for these file types. They are an
/// intentional permanent Datalab route, even if a future service advertises
/// their MIME types by mistake. Images are not on the list: the Vision engine
/// exists only on macOS 26 and up, so the same build has to route an image
/// either way depending on what the service it is talking to advertises. This
/// set must stay identical to `PERMANENT_DIRECT_FORMATS` in `lib.rs`, which
/// does the authoritative routing at run time.
const PERMANENT_DIRECT_EXTENSIONS = new Set(["html", "htm"]);

export type ConversionCapabilities =
  | { state: "idle" | "loading" }
  /// `message` is what the host said when the probe failed. Nothing in
  /// Settings moves the service, so that sentence is the only thing the run
  /// hint has to offer the user.
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
