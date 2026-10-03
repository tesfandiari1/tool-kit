import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ConvertOneOutcome,
  DirListing,
  HistoryEntry,
  Job,
  JobId,
  KnownDir,
  MoveOutcome,
  ProjectSummary,
  RunResult,
  Scan,
  Settings,
  WorkspaceInfo,
} from "./types";
import type { ServiceRequestPayload, ServiceResponsePayload } from "./api/transport";

/// Typed IPC boundary. `@/platform/host` covers dialogs, window, drag-drop.
export const commands = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: async (settings: Settings): Promise<void> => {
    await invoke("save_settings", { settings });
  },
  suggestedWorkspacePath: () => invoke<string>("suggested_workspace_path"),
  /// Read-only: nothing is written until `setupWorkspace`.
  inspectWorkspacePath: (path: string) => invoke<boolean>("inspect_workspace_path", { path }),
  /// Creates or adopts the workspace, and guarantees a catch-all project.
  setupWorkspace: (path: string) => invoke<WorkspaceInfo>("setup_workspace", { path }),
  /// Answers with a `welcomePath` only on the launch that wrote the file.
  ensureWorkspace: () => invoke<WorkspaceInfo | null>("ensure_workspace"),
  listProjects: () => invoke<ProjectSummary[]>("list_projects"),
  createProject: (title: string) => invoke<ProjectSummary>("create_project", { title }),
  /// One directory level, `rel` workspace-relative. Rejects with a `ListError`,
  /// not a string: see `listFailure` in `useProjectTree`.
  listProjectFiles: (rel: string) => invoke<DirListing>("list_project_files", { rel }),
  listJobs: () => invoke<Job[]>("list_jobs"),
  scanInputs: (inputs: string[]) => invoke<Scan>("scan_inputs", { inputs }),
  /// No destination argument: the host derives it from the same settings the
  /// scan's counts came from.
  runPipeline: (inputs: string[], jobType: JobId) =>
    invoke<RunResult>("run_pipeline", { inputs, jobType }),
  /// Moves each dropped path into the project and answers the paths to stage.
  /// One that could not move stays put, is staged anyway, and is named in `failed`.
  moveIntoProject: (inputs: string[], projectRel: string) =>
    invoke<MoveOutcome>("move_into_project", { inputs, projectRel }),
  /// Moves the file and the result beside it. Rejects during a run.
  moveToProject: (rel: string, projectRel: string) =>
    invoke<string>("move_to_project", { rel, projectRel }),
  /// One `stat` a folder, against a `read_dir` plus a `stat` per entry.
  changedProjectDirs: (known: KnownDir[]) => invoke<string[]>("changed_project_dirs", { known }),
  /// Answers a verdict rather than an error. Never joins a run in flight.
  convertOne: (rel: string) => invoke<ConvertOneOutcome>("convert_one", { rel }),
  stopRun: () => invoke<number>("stop_run"),
  retryJob: async (id: number): Promise<void> => {
    await invoke("retry_job", { id });
  },
  retryFailed: () => invoke<number>("retry_failed"),
  revealPath: async (path: string): Promise<void> => {
    await invoke("reveal_path", { path });
  },
  readDocument: (path: string) => invoke<{ text: string; mtimeMs: number }>("read_document", { path }),
  /// Copy's reader. Same file, no preview cap.
  readDocumentText: (path: string) => invoke<string>("read_document_text", { path }),
  /// Rejects unless the file still has `expectedMtimeMs`. Resolves to the new
  /// mtime, which the caller carries into its next save.
  writeDocument: (path: string, text: string, expectedMtimeMs: number) =>
    invoke<number>("write_document", { path, text, expectedMtimeMs }),
  listHistory: (query: string, limit: number) => invoke<HistoryEntry[]>("list_history", { query, limit }),
  clearHistory: async (): Promise<void> => {
    await invoke("clear_history");
  },
  quitApp: async (): Promise<void> => {
    await invoke("quit_app");
  },
  /// Only `@/app/api/transport` calls this. It refuses Markdown artifacts, so
  /// a large result never crosses IPC.
  serviceRequest: (request: ServiceRequestPayload) =>
    invoke<ServiceResponsePayload>("service_request", { request }),
  onJobUpdated: (handler: (job: Job) => void) => listen<Job>("job-updated", (e) => {
    handler(e.payload);
  }),
  onOpenSettings: (handler: () => void) => listen("open-settings", () => {
    handler();
  }),
};
