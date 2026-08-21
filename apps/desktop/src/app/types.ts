/// Wire types shared with the Tauri command surface. This module must not
/// import React — keep clients and view code on opposite sides of the boundary.

export type JobId = "convert" | "transcribe";
export type Status = "queued" | "working" | "processing" | "done" | "failed";
export type SecretId = "datalab" | "revai" | "backend";
export type ConversionRoute = "direct" | "backend";
export type ConversionProfile = "standard" | "local_only";
export type ReuseDisposition = "pending" | "already_here" | "reusable";
/// How the local conversion service gets started. `sidecar` is the app
/// spawning and owning it; `manual` is the developer running it themselves.
/// The one conversion question first run asks. It maps onto a route, a
/// profile, and a backend mode; see `domains/onboarding/conversionMode`.
export type OnboardingConversionMode = "local" | "cloud";
/// The library, or a panel that replaces the left column. Documents open
/// beside it and are not a view.
export type View = "library" | "run" | "settings" | "history";

/// The workspace `setup_workspace` created or adopted.
export interface WorkspaceInfo {
  workspacePath: string;
  workspaceId: string;
  /// The project every import lands in until the user picks another.
  inboxProjectId: string;
  /// The folder already held a workspace, so setup adopted it rather than
  /// writing a new one. It is what makes first run say "Open" instead of
  /// "Create".
  adopted: boolean;
}

/// One project folder, as the sidebar lists it.
export interface ProjectSummary {
  id: string;
  title: string;
  /// Relative to the workspace root ("Inbox"), so moving the workspace does
  /// not stale the list.
  path: string;
  /// RFC 3339 UTC.
  createdAt: string;
}

export interface ScannedConversionFile {
  sourcePath: string;
  mediaType: string;
  reuse: ReuseDisposition;
}

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
  /// Concrete Convert files and their MIME/reuse disposition. File contents
  /// stay in the host; this metadata is enough for capability-driven routing.
  convertFiles: ScannedConversionFile[];
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
  /// Backend route identifiers are service-owned data, not a closed frontend enum.
  route: string | null;
  reasonCodes: string[];
  warnings: string[];
  failure: { code: string; message: string } | null;
  outputPath: string | null;
  error: string | null;
  createdAt: number;
  startedAt: number | null;
}

export interface Settings {
  /// First run has been answered. The gate is keyed on `workspacePath`, which
  /// is the thing the app cannot work without; this records that the user was
  /// asked rather than that a folder happens to exist.
  onboardingComplete: boolean;
  /// The workspace folder. Null means first run.
  workspacePath: string | null;
  workspaceId: string | null;
  /// The project the library opens on. Null until a workspace exists.
  activeProjectId: string | null;
  inputs: string[];
  outputDir: string | null;
  jobType: JobId;
  datalabFormat: string;
  datalabPipelineId: string | null;
  datalabHighAccuracy: boolean;
  conversionRoute: ConversionRoute;
  conversionProfile: ConversionProfile;
  languageCorrection: boolean;
  /// Words local OCR should prefer when it is unsure.
  customWords: string[];
  skipAlreadyDone: boolean;
  /// Last SplitPane layout. Null until the user has dragged the seam.
  splitLayout: Record<string, number> | null;
  /// Inner size while the document pane is open. Null until the first expand.
  expandedWidth: number | null;
  expandedHeight: number | null;
  /// Webview page-zoom factor, not a font size: it scales the whole app,
  /// chrome included. 1 is 100%.
  zoom: number;
}

export type SecretStatus = Record<SecretId, boolean>;

export const DEFAULT_SETTINGS: Settings = {
  onboardingComplete: false,
  workspacePath: null,
  workspaceId: null,
  activeProjectId: null,
  inputs: [],
  outputDir: null,
  jobType: "convert",
  datalabFormat: "markdown",
  datalabPipelineId: null,
  datalabHighAccuracy: true,
  conversionRoute: "backend",
  conversionProfile: "standard",
  languageCorrection: true,
  customWords: [],
  skipAlreadyDone: true,
  splitLayout: null,
  expandedWidth: null,
  expandedHeight: null,
  zoom: 1,
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
  convertFiles: [],
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
