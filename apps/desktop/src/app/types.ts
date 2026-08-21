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
/// What the left column holds. Documents open beside it and are not a view,
/// and Settings is a sheet over the whole window rather than a fourth member:
/// it used to evict the Run column mid-run.
export type View = "library" | "run" | "history";

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
  /// The `Inbox/welcome.md` this call wrote, or null when it wrote none. Set
  /// on a brand-new workspace only, so first run opens it as a real tab and a
  /// user who deletes it never sees it again.
  welcomePath: string | null;
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

/// One entry in a project folder, as `list_project_files` reports it. The host
/// decides everything a row renders, so the tree never mirrors the extension
/// table or the preview cap.
export interface FileRow {
  /// Absolute. The only path handed back to a command that opens or reveals.
  path: string;
  /// Workspace-relative. Expansion and selection are keyed on this, so a
  /// workspace renamed in Finder does not strand every entry.
  rel: string;
  name: string;
  isDir: boolean;
  /// Lowercase, no dot. Empty when the name carries none.
  ext: string;
  mediaType: string;
  size: number;
  modifiedMs: number;
  /// The job that takes this file. Null means neither one does.
  job: JobId | null;
  /// The sibling result this source already has, folded into its row.
  resultPath: string | null;
  resultName: string | null;
  /// This file opens in the document pane. Decided by the host, so a click can
  /// never end in a `read_document` failure toast.
  openable: boolean;
  /// The same answer for `resultPath`. An `html` result is not a document this
  /// pane reads, so the source row cannot imply it.
  resultOpenable: boolean;
}

/// One directory level.
export interface DirListing {
  /// The directory's own mtime, so a focus reconcile can skip a folder that
  /// did not change.
  modifiedMs: number;
  entries: FileRow[];
  /// Entries past the host's cap, dropped from `entries`.
  truncated: number;
  /// Files with a job and no result. The project row's count.
  pending: number;
}

/// One folder the tree has listed, paired with the mtime that listing carried.
/// The focus reconcile hands these back so the host can answer which of them
/// moved without reading a single directory.
export interface KnownDir {
  rel: string;
  modifiedMs: number;
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
  /// Library tree rows left open, workspace-relative. Never null: this object
  /// is spread over `DEFAULT_SETTINGS`, where an explicit null would win.
  expandedPaths: string[];
}

export type SecretStatus = Record<SecretId, boolean>;

export const DEFAULT_SETTINGS: Settings = {
  onboardingComplete: false,
  workspacePath: null,
  workspaceId: null,
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
  expandedPaths: [],
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

/// Why the host refused to convert one file. Every reason maps to something
/// the tree can do about it, which is why they are codes and not sentences.
export type ConvertBlockReason =
  | "not_convertible"
  | "run_in_progress"
  | "backend_unavailable"
  | "backend_not_accepting"
  | "local_only_requires_remote"
  | "missing_key";

/// The host's verdict on a one-file convert. The tree renders this rather than
/// planning the conversion itself: the route plan, the key checks and the
/// reuse rule all live in Rust.
export interface ConvertOneOutcome {
  kind: "queued" | "copied" | "blocked";
  reason: ConvertBlockReason | null;
  /// Shown verbatim. Never parsed.
  message: string | null;
}

export interface RunResult {
  /// Files sent to the provider — the only ones that cost anything.
  count: number;
  /// Files left alone: their result is already in the output folder.
  skipped: number;
  /// Files satisfied by copying a result an earlier run produced elsewhere.
  copied: number;
}
