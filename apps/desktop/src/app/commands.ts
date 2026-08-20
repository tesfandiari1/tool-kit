import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  HistoryEntry,
  Job,
  JobId,
  ProjectSummary,
  RunResult,
  Scan,
  SecretId,
  SecretStatus,
  Settings,
  WorkspaceInfo,
} from "./types";
import type { ServiceRequestPayload, ServiceResponsePayload } from "./api/transport";

/// Typed IPC boundary. Views import `commands`, never `invoke` with a raw
/// string. This module is the only frontend door to native work, same shape as
/// Yaak/Helios `lib/ipc`; `@/platform/host` covers the other host surfaces
/// (dialogs, window, drag-drop). Conversion-service HTTP stays in Rust (M6),
/// and the OpenAPI-typed client for it lives in `./api`.
export const commands = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: async (settings: Settings): Promise<void> => {
    await invoke("save_settings", { settings });
  },
  /// Where a workspace should go when the user has not said. The host picks
  /// it; first run only confirms it.
  suggestedWorkspacePath: () => invoke<string>("suggested_workspace_path"),
  /// Whether a workspace is already there. Read-only: nothing is written until
  /// `setupWorkspace`, so first run can name what the button is about to do
  /// before the user commits to it.
  inspectWorkspacePath: (path: string) => invoke<boolean>("inspect_workspace_path", { path }),
  /// Creates the workspace, or adopts the one already there, and guarantees an
  /// Inbox project either way.
  setupWorkspace: (path: string) => invoke<WorkspaceInfo>("setup_workspace", { path }),
  listProjects: () => invoke<ProjectSummary[]>("list_projects"),
  secretStatus: () => invoke<SecretStatus>("secret_status"),
  setSecret: async (provider: SecretId, value: string): Promise<void> => {
    await invoke("set_secret", { provider, value });
  },
  listJobs: () => invoke<Job[]>("list_jobs"),
  scanInputs: (inputs: string[]) => invoke<Scan>("scan_inputs", { inputs }),
  runPipeline: (inputs: string[], outputDir: string, jobType: JobId) =>
    invoke<RunResult>("run_pipeline", { inputs, outputDir, jobType }),
  stopRun: () => invoke<number>("stop_run"),
  retryJob: async (id: number): Promise<void> => {
    await invoke("retry_job", { id });
  },
  retryFailed: () => invoke<number>("retry_failed"),
  revealPath: async (path: string): Promise<void> => {
    await invoke("reveal_path", { path });
  },
  readTextFile: (path: string) => invoke<string>("read_text_file", { path }),
  readDocument: (path: string) => invoke<{ text: string; mtimeMs: number }>("read_document", { path }),
  /// Copy's reader. Same file, no preview cap: a result too large to open in
  /// the pane is still worth copying.
  readDocumentText: (path: string) => invoke<string>("read_document_text", { path }),
  /// Saves only when the file on disk still has `expectedMtimeMs`; otherwise it
  /// rejects rather than overwriting an edit made outside the app. Resolves to
  /// the new mtime, which the caller carries into its next save.
  writeDocument: (path: string, text: string, expectedMtimeMs: number) =>
    invoke<number>("write_document", { path, text, expectedMtimeMs }),
  listHistory: (query: string, limit: number) => invoke<HistoryEntry[]>("list_history", { query, limit }),
  clearHistory: async (): Promise<void> => {
    await invoke("clear_history");
  },
  quitApp: async (): Promise<void> => {
    await invoke("quit_app");
  },
  /// One replayed HTTP call against the conversion service. The host resolves
  /// the base URL, attaches the Keychain bearer token, and streams multipart
  /// sources from disk, so neither the token nor file bytes reach the webview.
  /// Only `@/app/api/transport` calls this — it is the `fetch` openapi-fetch
  /// runs on, not something a view invokes. The generic door refuses Markdown
  /// artifacts so a large result can never cross IPC as a response body.
  serviceRequest: (request: ServiceRequestPayload) =>
    invoke<ServiceResponsePayload>("service_request", { request }),
  /// Streams one published Markdown artifact to a collision-safe file in the
  /// host and returns only its path. Artifact bytes never enter the webview.
  downloadConversionMarkdown: (conversionId: string, outputDir: string, fileName: string) =>
    invoke<string>("download_conversion_markdown", { conversionId, outputDir, fileName }),
  onJobUpdated: (handler: (job: Job) => void) => listen<Job>("job-updated", (e) => {
    handler(e.payload);
  }),
};
