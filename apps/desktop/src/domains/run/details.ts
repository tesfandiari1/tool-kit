import type { Job } from "@/app/types";

export type JobDetailKind = "route" | "reason" | "warning" | "failure";

export interface JobDetailItem {
  kind: JobDetailKind;
  label: string;
  value: string;
}

type JobMetadata = Pick<Job, "route" | "reasonCodes" | "warnings" | "failure" | "error">;

/// Keep service-owned values intact so new route, reason, warning, and failure
/// codes become visible without a frontend release.
export function jobDetailItems(job: JobMetadata): JobDetailItem[] {
  const items: JobDetailItem[] = [];

  if (job.route !== null) {
    items.push({ kind: "route", label: "Route", value: job.route });
  }
  for (const reason of job.reasonCodes) {
    items.push({ kind: "reason", label: "Reason", value: reason });
  }
  for (const warning of job.warnings) {
    items.push({ kind: "warning", label: "Warning", value: warning });
  }
  // A backend failure is also the row's error, in the same words, and the row
  // shows that line already.
  const failure = job.failure && `${job.failure.code}: ${job.failure.message}`;
  if (failure && failure !== job.error) {
    items.push({ kind: "failure", label: "Failure", value: failure });
  }

  return items;
}

export function jobDetailText(items: readonly JobDetailItem[]): string {
  return items.map(({ label, value }) => `${label}: ${value}`).join(" · ");
}
