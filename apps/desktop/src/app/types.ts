/// Wire types for the Tauri command surface. Never import React here.

export type JobId = "convert" | "transcribe";
export type Status = "queued" | "working" | "processing" | "done" | "failed";
export type SecretId = "datalab" | "revai" | "backend";
export type ConversionRoute = "direct" | "backend";
export type ConversionProfile = "standard" | "local_only";
export type ReuseDisposition = "pending" | "already_here" | "reusable";
/// First run's one conversion question. See `domains/onboarding/conversionMode`.
export type OnboardingConversionMode = "local" | "cloud";
/// What the end pane holds. Settings is a sheet, not a fourth member.
export type View = "library" | "run" | "history";

export interface WorkspaceInfo {
  workspacePath: string;
  catchAllProjectId: string;
  /// That project's folder, workspace-relative.
  catchAllPath: string;
  /// Set on the launch that wrote the file, so it opens once per workspace.
  welcomePath: string | null;
}

export interface ProjectSummary {
  id: string;
  title: string;
  /// Relative to the workspace root, so moving the workspace keeps it valid.
  path: string;
  /// RFC 3339 UTC.
  createdAt: string;
}

/// One entry in a project folder. The host decides everything a row renders,
/// so the tree never mirrors the extension table or the preview cap.
export interface FileRow {
  /// Absolute. The only path handed to a command that opens or reveals.
  path: string;
  /// Workspace-relative. Expansion and selection are keyed on this.
  rel: string;
  name: string;
  isDir: boolean;
  /// Lowercase, no dot. Empty when the name carries none.
  ext: string;
  mediaType: string;
  size: number;
  modifiedMs: number;
  /// Null when neither job takes it.
  job: JobId | null;
  resultPath: string | null;
  resultName: string | null;
  /// Opens in the document pane. It cannot answer for the encoding, so a
  /// failed click falls back to the card.
  openable: boolean;
  /// The same answer for `resultPath`.
  resultOpenable: boolean;
}

export interface DirListing {
  /// The directory's own mtime, so a focus reconcile can skip it unchanged.
  modifiedMs: number;
  entries: FileRow[];
  truncated: number;
  /// Files with a job and no result. The project row's count.
  pending: number;
}

/// Only a folder reported `gone` loses its persisted expansion.
export interface ListError {
  gone: boolean;
  message: string;
}

/// A listed folder and its mtime, so the host answers without a `read_dir`.
export interface KnownDir {
  rel: string;
  modifiedMs: number;
}

export interface ScannedConversionFile {
  sourcePath: string;
  mediaType: string;
  reuse: ReuseDisposition;
}

export interface InputMatch {
  path: string;
  /// Relative to the dropped folder.
  name: string;
  job: JobId;
}

/// One dropped path. The host owns the walk, the dotfiles and the depth cap.
export interface InputNode {
  path: string;
  name: string;
  isDir: boolean;
  /// Null for a folder, and for a file neither job takes.
  job: JobId | null;
  matches: InputMatch[];
  truncated: number;
}

export interface Scan {
  convert: number;
  transcribe: number;
  /// Already in the output folder: nothing happens to these. Both jobs report,
  /// so switching job needs no re-scan and autodetect sees no new signal.
  alreadyHereConvert: number;
  alreadyHereTranscribe: number;
  /// Result in another folder: copied rather than bought again.
  reusableConvert: number;
  reusableTranscribe: number;
  convertFiles: ScannedConversionFile[];
  alreadyText: number;
  suggestedOutput: string | null;
  nodes: InputNode[];
}

export interface HistoryEntry {
  id: number;
  fileName: string;
  sourcePath: string;
  outputPath: string | null;
  /// The result is still on disk. False on a failed row.
  outputExists: boolean;
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
  startedAt: number | null;
}

export interface Settings {
  /// The workspace folder. Null means first run.
  workspacePath: string | null;
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
  /// Inner size the user last left the workspace window at. Null until then.
  expandedWidth: number | null;
  expandedHeight: number | null;
  /// Page zoom, not a font size: it scales the whole app. 1 is 100%.
  zoom: number;
  /// Never null: this object is spread over `DEFAULT_SETTINGS`, where an
  /// explicit null would win.
  expandedPaths: string[];
  /// Where a drop is filed and a run writes, workspace-relative. This, not
  /// `outputDir`, once a workspace is bound.
  activeProjectPath: string | null;
}

export type SecretStatus = Record<SecretId, boolean>;

export const DEFAULT_SETTINGS: Settings = {
  workspacePath: null,
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
  activeProjectPath: null,
};

export const ACTIVE: Status[] = ["queued", "working", "processing"];

/// The newest job for a source, never the first. A file converted twice carries
/// two rows, and the stale `done` leaves Convert enabled for one more charge.
/// Ids come from an `AtomicU64`, so the highest is live.
export function latestJobFor(jobs: Job[], sourcePath: string): Job | null {
  let latest: Job | null = null;
  for (const job of jobs) {
    if (job.sourcePath === sourcePath && (latest === null || job.id > latest.id)) latest = job;
  }
  return latest;
}
/// Above this many files, confirm before spending.
export const BIG_RUN = 25;
/// Rows per history read.
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
  nodes: [],
};

/// Codes rather than sentences: every reason maps to something the tree does.
export type ConvertBlockReason =
  | "not_convertible"
  | "run_in_progress"
  | "backend_unavailable"
  | "backend_not_accepting"
  | "local_only_requires_remote"
  | "missing_key";

/// The host's verdict. The route plan, the key checks and the reuse rule all
/// live in Rust.
export interface ConvertOneOutcome {
  kind: "queued" | "copied" | "blocked";
  reason: ConvertBlockReason | null;
  /// Shown verbatim. Never parsed.
  message: string | null;
}

export interface RunResult {
  /// Sent to the provider, the only ones that cost anything.
  count: number;
  /// Left alone: the result is already in the output folder.
  skipped: number;
  /// Satisfied by copying a result from elsewhere.
  copied: number;
}
