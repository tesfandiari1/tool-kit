import type { Job } from "@/app/types";

export type JobDetailKind = "warning" | "failure";

export interface JobDetailItem {
  kind: JobDetailKind;
  label: string;
  value: string;
}

type JobMetadata = Pick<Job, "warnings" | "failure" | "error">;

/// Keep service-owned values intact so new warning and failure codes become
/// visible without a frontend release.
export function jobDetailItems(job: JobMetadata): JobDetailItem[] {
  const items: JobDetailItem[] = [];

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
