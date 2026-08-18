/// Wire types shared with the Tauri command surface. This module must not
/// import React — keep clients and view code on opposite sides of the boundary.

export type JobId = "convert" | "transcribe";
export type Status = "queued" | "working" | "processing" | "done" | "failed";
export type SecretId = "datalab" | "revai" | "backend";
export type ConversionRoute = "direct" | "backend";
export type ConversionProfile = "standard" | "local_only";
/// The compact launcher, or a panel that replaces the left column. Documents
/// open beside it and are not a view.
export type View = "run" | "settings" | "history";

/// What `scan_inputs` reports for the current selection.
export interface Scan {
  convert: number;
  transcribe: number;
  /// Files whose result is already in the chosen output folder — nothing at
  /// all happens to these. Both jobs are reported so switching job reads a
  /// number already in hand, with no re-scan, and so no new signal for the
  /// autodetect effect to react to.
  alreadyHereConvert: number;
  alreadyHereTranscribe: number;
  /// Files whose result exists in some *other* folder. These are copied rather
  /// than sent to the provider again: real work, but free and instant.
  reusableConvert: number;
  reusableTranscribe: number;
  alreadyText: number;
  suggestedOutput: string | null;
}

/// One finished job, as `list_history` returns it.
export interface HistoryEntry {
  id: number;
  fileName: string;
  sourcePath: string;
  outputPath: string | null;
  jobType: JobId;
  outputFormat: string;
  status: "done" | "failed";
  error: string | null;
  /// Unix seconds.
  finishedAt: number;
}

export interface Job {
  id: number;
  fileName: string;
  sourcePath: string;
  jobType: JobId;
  service: string;
  status: Status;
  progressNote: string;
  outputPath: string | null;
  outputText: string | null;
  error: string | null;
  createdAt: number;
  startedAt: number | null;
}

export interface Settings {
  inputs: string[];
  outputDir: string | null;
  jobType: JobId;
  datalabFormat: string;
  datalabPipelineId: string | null;
  datalabHighAccuracy: boolean;
  conversionRoute: ConversionRoute;
  backendUrl: string;
  conversionProfile: ConversionProfile;
  skipAlreadyDone: boolean;
  /// Last SplitPane layout. Null until the user has dragged the seam.
  splitLayout: Record<string, number> | null;
  /// Inner size while the document pane is open. Null until the first expand.
  expandedWidth: number | null;
  expandedHeight: number | null;
}

export type SecretStatus = Record<SecretId, boolean>;

export const DEFAULT_BACKEND_URL = "http://127.0.0.1:8080";

export const DEFAULT_SETTINGS: Settings = {
  inputs: [],
  outputDir: null,
  jobType: "convert",
  datalabFormat: "markdown",
  datalabPipelineId: null,
  datalabHighAccuracy: true,
  conversionRoute: "direct",
  backendUrl: DEFAULT_BACKEND_URL,
  conversionProfile: "standard",
  skipAlreadyDone: true,
  splitLayout: null,
  expandedWidth: null,
  expandedHeight: null,
};

export const ACTIVE: Status[] = ["queued", "working", "processing"];
/// Above this many files, confirm before spending.
export const BIG_RUN = 25;
/// Rows fetched per history read. Deep enough to scroll through a month of
/// work, shallow enough that the panel opens instantly.
export const HISTORY_LIMIT = 400;
export const EMPTY_SCAN: Scan = {
  convert: 0,
  transcribe: 0,
  alreadyHereConvert: 0,
  alreadyHereTranscribe: 0,
  reusableConvert: 0,
  reusableTranscribe: 0,
  alreadyText: 0,
  suggestedOutput: null,
};

export interface RunResult {
  /// Files sent to the provider — the only ones that cost anything.
  count: number;
  /// Files left alone: their result is already in the output folder.
  skipped: number;
  /// Files satisfied by copying a result an earlier run produced elsewhere.
  copied: number;
}
