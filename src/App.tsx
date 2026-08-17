import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowClockwise,
  CheckCircle,
  CircleNotch,
  Copy,
  Eye,
  FileText,
  Folder,
  FolderOpen,
  GearSix,
  Play,
  Sparkle,
  WarningCircle,
  Waveform,
  X,
  type Icon,
} from "@phosphor-icons/react";
import "./App.css";

type JobId = "convert" | "transcribe" | "summarize";
type Status = "queued" | "working" | "processing" | "done" | "failed";
type SecretId = "datalab" | "revai" | "anthropic";

interface Job {
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
}
interface Settings {
  inputs: string[];
  outputDir: string | null;
  jobType: JobId;
  datalabFormat: string;
  datalabPipelineId: string | null;
  summarizeModel: string;
}
type SecretStatus = Record<SecretId, boolean>;

const DEFAULT_SETTINGS: Settings = {
  inputs: [],
  outputDir: null,
  jobType: "convert",
  datalabFormat: "markdown",
  datalabPipelineId: null,
  summarizeModel: "claude-sonnet-4-6",
};

interface JobDef {
  id: JobId;
  label: string;
  verb: string;
  desc: string;
  secret: SecretId;
  service: string;
  icon: Icon;
}
const JOBS: JobDef[] = [
  { id: "convert", label: "Convert", verb: "Convert", desc: "Documents to Markdown", secret: "datalab", service: "Datalab", icon: FileText },
  { id: "transcribe", label: "Transcribe", verb: "Transcribe", desc: "Audio and video to text", secret: "revai", service: "Rev.ai", icon: Waveform },
  { id: "summarize", label: "Summarize", verb: "Summarize", desc: "Text into AI notes", secret: "anthropic", service: "Claude", icon: Sparkle },
];
const ACTIVE: Status[] = ["queued", "working", "processing"];

export default function App() {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [secrets, setSecrets] = useState<SecretStatus>({ datalab: false, revai: false, anthropic: false });
  const [jobs, setJobs] = useState<Job[]>([]);
  const [inputCount, setInputCount] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const [showSettings, setShowSettings] = useState(false);
  const [preview, setPreview] = useState<Job | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<number | null>(null);
  const wasRunning = useRef(false);

  const job = useMemo(() => JOBS.find((j) => j.id === settings.jobType) ?? JOBS[0], [settings.jobType]);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 3400);
  }, []);

  const upsert = useCallback((j: Job) => {
    setJobs((prev) => {
      const i = prev.findIndex((p) => p.id === j.id);
      if (i === -1) return [...prev, j];
      const copy = prev.slice();
      copy[i] = j;
      return copy;
    });
  }, []);

  useEffect(() => {
    void (async () => {
      const [s, k, j] = await Promise.all([
        invoke<Settings>("get_settings"),
        invoke<SecretStatus>("secret_status"),
        invoke<Job[]>("list_jobs"),
      ]);
      setSettings(s);
      setSecrets(k);
      setJobs(j);
    })();
  }, []);

  useEffect(() => {
    const un = listen<Job>("job-updated", (e) => upsert(e.payload));
    return () => void un.then((f) => f());
  }, [upsert]);

  useEffect(() => {
    if (settings.inputs.length === 0) {
      setInputCount(0);
      return;
    }
    void invoke<number>("scan_inputs", { inputs: settings.inputs, jobType: settings.jobType })
      .then(setInputCount)
      .catch(() => setInputCount(0));
  }, [settings.inputs, settings.jobType]);

  const persist = useCallback(
    (patch: Partial<Settings>) => {
      setSettings((prev) => {
        const next = { ...prev, ...patch };
        void invoke("save_settings", { settings: next }).catch(() => undefined);
        return next;
      });
    },
    []
  );

  const mutateInputs = useCallback((fn: (cur: string[]) => string[]) => {
    setSettings((prev) => {
      const next = { ...prev, inputs: fn(prev.inputs) };
      void invoke("save_settings", { settings: next }).catch(() => undefined);
      return next;
    });
  }, []);

  const addPaths = useCallback(
    (paths: string[]) => {
      if (paths.length) mutateInputs((cur) => Array.from(new Set([...cur, ...paths])));
    },
    [mutateInputs]
  );

  const addFiles = async () => {
    const sel = await openDialog({ multiple: true });
    if (Array.isArray(sel)) addPaths(sel);
    else if (typeof sel === "string") addPaths([sel]);
  };

  const addFolders = async () => {
    const sel = await openDialog({ directory: true, multiple: true });
    if (Array.isArray(sel)) addPaths(sel);
    else if (typeof sel === "string") addPaths([sel]);
  };

  const pickOutput = async () => {
    const dir = await openDialog({ directory: true });
    if (typeof dir === "string") persist({ outputDir: dir });
  };

  useEffect(() => {
    const un = getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload as { type: string; paths?: string[] };
      if (p.type === "over" || p.type === "enter") setDragging(true);
      else if (p.type === "drop") {
        setDragging(false);
        addPaths(p.paths ?? []);
      } else setDragging(false);
    });
    return () => void un.then((f) => f());
  }, [addPaths]);

  const finished = jobs.filter((j) => j.status === "done" || j.status === "failed").length;
  const doneCount = jobs.filter((j) => j.status === "done").length;
  const failedCount = jobs.filter((j) => j.status === "failed").length;
  const running = jobs.some((j) => ACTIVE.includes(j.status));
  const total = jobs.length;

  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [running]);

  // When a run finishes, clear the input selection so the same files can't be
  // re-run by accident — Run greys out until new inputs are added.
  useEffect(() => {
    if (wasRunning.current && !running) mutateInputs(() => []);
    wasRunning.current = running;
  }, [running, mutateInputs]);

  const hasKey = secrets[job.secret];
  const canRun = Boolean(settings.inputs.length > 0 && settings.outputDir && hasKey && inputCount > 0 && !running);

  const run = async () => {
    setJobs([]);
    try {
      await invoke<{ count: number }>("run_pipeline", {
        inputs: settings.inputs,
        outputDir: settings.outputDir,
        jobType: settings.jobType,
      });
    } catch (e) {
      showToast(String(e));
    }
  };

  const copyText = async (j: Job | null) => {
    if (!j?.outputText) return;
    try {
      await navigator.clipboard.writeText(j.outputText);
      showToast("Copied to clipboard");
    } catch {
      showToast("Copy failed");
    }
  };

  const runLabel = inputCount > 0 ? `${job.verb} ${inputCount} file${inputCount > 1 ? "s" : ""}` : "Run pipeline";

  let hint: string | null = null;
  if (!hasKey) hint = `Add your ${job.service} key in Settings`;
  else if (settings.inputs.length > 0 && inputCount === 0) hint = `No ${job.label.toLowerCase()} files in your selection`;

  return (
    <div className="app">
      <header className="bar">
        <div className="bar-brand">
          <span className="mark" aria-hidden>
            <Sparkle weight="fill" />
          </span>
          Tool-Kit
        </div>
        <button className="icon-btn" onClick={() => setShowSettings((v) => !v)} title="Settings" aria-label="Settings">
          <GearSix weight={showSettings ? "fill" : "regular"} />
        </button>
      </header>

      {showSettings ? (
        <SettingsPanel
          settings={settings}
          secrets={secrets}
          onPersist={persist}
          onSecrets={setSecrets}
          onToast={showToast}
          onClose={() => setShowSettings(false)}
        />
      ) : (
        <main className="flow">
          <InputPicker
            inputs={settings.inputs}
            count={inputCount}
            dragging={dragging}
            onAddFiles={() => void addFiles()}
            onAddFolders={() => void addFolders()}
            onRemove={(p) => mutateInputs((cur) => cur.filter((x) => x !== p))}
            onClear={() => mutateInputs(() => [])}
          />
          <FolderField
            label="Output folder"
            path={settings.outputDir}
            placeholder="Choose where results are saved"
            onPick={() => void pickOutput()}
          />

          <div className="field">
            <span className="field-label">Job</span>
            <div className="segmented" role="tablist">
              {JOBS.map((j) => {
                const JIcon = j.icon;
                const selected = j.id === settings.jobType;
                return (
                  <button
                    key={j.id}
                    role="tab"
                    aria-selected={selected}
                    className={`segment ${selected ? "selected" : ""}`}
                    onClick={() => persist({ jobType: j.id })}
                  >
                    <JIcon weight={selected ? "fill" : "regular"} />
                    {j.label}
                  </button>
                );
              })}
            </div>
            <span className="field-hint">
              {job.desc}, via {job.service}
            </span>
          </div>

          <div className="run">
            <button className="run-btn" disabled={!canRun} onClick={() => void run()}>
              {running ? (
                <>
                  <CircleNotch className="spin" weight="bold" />
                  Working… {finished} of {total}
                </>
              ) : (
                <>
                  <Play weight="fill" />
                  {runLabel}
                </>
              )}
            </button>
            {running && (
              <div className="progress" aria-hidden>
                <div className="progress-fill" style={{ width: `${total ? (finished / total) * 100 : 0}%` }} />
              </div>
            )}
            {!running && hint && (
              <button className="hint" onClick={() => !hasKey && setShowSettings(true)}>
                <WarningCircle weight="fill" />
                {hint}
              </button>
            )}
          </div>

          {jobs.length > 0 && (
            <section className="results">
              <div className="results-head">
                <span>
                  {running ? `${total} files` : `${doneCount} done${failedCount ? `, ${failedCount} failed` : ""} of ${total}`}
                </span>
                {!running && doneCount > 0 && settings.outputDir && (
                  <button className="link-btn" onClick={() => void invoke("reveal_path", { path: settings.outputDir })}>
                    Show in Finder
                  </button>
                )}
              </div>
              {jobs.map((j) => (
                <JobRow
                  key={j.id}
                  job={j}
                  now={now}
                  onPreview={() => setPreview(j)}
                  onCopy={() => void copyText(j)}
                  onReveal={() => j.outputPath && void invoke("reveal_path", { path: j.outputPath })}
                  onRetry={() => void invoke("retry_job", { id: j.id })}
                />
              ))}
            </section>
          )}
        </main>
      )}

      {preview && (
        <div className="modal-bg" onClick={() => setPreview(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <span className="modal-title">{preview.fileName}</span>
              <div className="modal-actions">
                <button className="btn-quiet" onClick={() => void copyText(preview)}>
                  Copy
                </button>
                <button className="btn-quiet" onClick={() => setPreview(null)}>
                  Close
                </button>
              </div>
            </div>
            <pre className="preview-body">{preview.outputText ?? "(empty)"}</pre>
          </div>
        </div>
      )}

      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}

function FolderField({
  label,
  path,
  placeholder,
  onPick,
}: {
  label: string;
  path: string | null;
  placeholder: string;
  onPick: () => void;
}) {
  return (
    <div className="field">
      <span className="field-label">{label}</span>
      <button className={`folder ${path ? "set" : ""}`} onClick={onPick} title={path ?? placeholder}>
        {path ? <FolderOpen weight="fill" /> : <Folder />}
        <span className="folder-path">{path ? (path.split(/[\\/]/).filter(Boolean).pop() ?? path) : placeholder}</span>
        <span className="folder-action">{path ? "Change" : "Choose"}</span>
      </button>
    </div>
  );
}

function InputPicker({
  inputs,
  count,
  dragging,
  onAddFiles,
  onAddFolders,
  onRemove,
  onClear,
}: {
  inputs: string[];
  count: number;
  dragging: boolean;
  onAddFiles: () => void;
  onAddFolders: () => void;
  onRemove: (p: string) => void;
  onClear: () => void;
}) {
  const basename = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
  const isFile = (p: string) => /\.[^/\\]+$/.test(p);
  return (
    <div className="field">
      <span className="field-label">
        Input{count > 0 ? ` · ${count} file${count > 1 ? "s" : ""}` : ""}
      </span>
      <div className={`inputs ${dragging ? "dragging" : ""}`}>
        {inputs.length === 0 ? (
          <div className="inputs-empty">Drop files or folders here, or add them below</div>
        ) : (
          inputs.map((p) => (
            <div className="input-item" key={p} title={p}>
              {isFile(p) ? <FileText /> : <FolderOpen weight="fill" />}
              <span className="input-name">{basename(p)}</span>
              <button className="input-x" onClick={() => onRemove(p)} aria-label="Remove">
                <X />
              </button>
            </div>
          ))
        )}
      </div>
      <div className="input-actions">
        <button className="mini-btn" onClick={onAddFiles}>
          <FileText /> Files
        </button>
        <button className="mini-btn" onClick={onAddFolders}>
          <FolderOpen /> Folder
        </button>
        {inputs.length > 0 && (
          <button className="mini-btn ghost" onClick={onClear}>
            Clear
          </button>
        )}
      </div>
    </div>
  );
}

function fmtElapsed(nowMs: number, createdSec: number) {
  const s = Math.max(0, Math.floor(nowMs / 1000) - createdSec);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function JobRow({
  job,
  now,
  onPreview,
  onCopy,
  onReveal,
  onRetry,
}: {
  job: Job;
  now: number;
  onPreview: () => void;
  onCopy: () => void;
  onReveal: () => void;
  onRetry: () => void;
}) {
  const active = ACTIVE.includes(job.status);
  return (
    <div className="job">
      <span className={`job-state ${job.status}`}>
        {active ? (
          <CircleNotch className="spin" />
        ) : job.status === "done" ? (
          <CheckCircle weight="fill" />
        ) : (
          <WarningCircle weight="fill" />
        )}
      </span>
      <div className="job-body">
        <span className="job-name" title={job.sourcePath}>
          {job.fileName}
        </span>
        {job.status === "failed" && job.error ? (
          <span className="job-sub err">{job.error}</span>
        ) : active ? (
          <span className="job-sub">{job.progressNote}</span>
        ) : (
          job.outputPath && <span className="job-sub">Saved to {job.outputPath}</span>
        )}
      </div>
      {(job.status === "working" || job.status === "processing") && (
        <span className="job-time">{fmtElapsed(now, job.createdAt)}</span>
      )}
      <div className="job-actions">
        {job.status === "done" && (
          <>
            <button className="icon-btn sm" title="Preview" onClick={onPreview}>
              <Eye />
            </button>
            <button className="icon-btn sm" title="Copy" onClick={onCopy}>
              <Copy />
            </button>
            {job.outputPath && (
              <button className="icon-btn sm" title="Show in Finder" onClick={onReveal}>
                <FolderOpen />
              </button>
            )}
          </>
        )}
        {job.status === "failed" && (
          <button className="icon-btn sm" title="Retry" onClick={onRetry}>
            <ArrowClockwise />
          </button>
        )}
      </div>
    </div>
  );
}

function KeyField({
  label,
  hint,
  saved,
  onSave,
}: {
  label: string;
  hint: string;
  saved: boolean;
  onSave: (value: string) => void;
}) {
  const [value, setValue] = useState("");
  return (
    <div className="setting">
      <label>
        {label}
        {saved && <em className="ok">saved</em>}
      </label>
      <div className="row">
        <input
          type="password"
          placeholder={saved ? "••••••••• (saved)" : hint}
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <button
          className="btn"
          onClick={() => {
            onSave(value);
            setValue("");
          }}
        >
          Save
        </button>
      </div>
    </div>
  );
}

function SettingsPanel({
  settings,
  secrets,
  onPersist,
  onSecrets,
  onToast,
  onClose,
}: {
  settings: Settings;
  secrets: SecretStatus;
  onPersist: (patch: Partial<Settings>) => void;
  onSecrets: (s: SecretStatus) => void;
  onToast: (msg: string) => void;
  onClose: () => void;
}) {
  const [advanced, setAdvanced] = useState(false);

  const saveKey = async (provider: SecretId, value: string) => {
    try {
      await invoke("set_secret", { provider, value });
      onSecrets(await invoke<SecretStatus>("secret_status"));
      onToast(value ? "Key saved" : "Key cleared");
    } catch (e) {
      onToast(String(e));
    }
  };

  return (
    <main className="flow">
      <div className="settings-head">
        <span className="field-label">API keys</span>
        <button className="icon-btn" onClick={onClose} aria-label="Close settings">
          <X />
        </button>
      </div>
      <KeyField label="Datalab" hint="X-API-Key" saved={secrets.datalab} onSave={(v) => void saveKey("datalab", v)} />
      <KeyField label="Rev.ai" hint="Access token" saved={secrets.revai} onSave={(v) => void saveKey("revai", v)} />
      <KeyField label="Claude (Anthropic)" hint="x-api-key" saved={secrets.anthropic} onSave={(v) => void saveKey("anthropic", v)} />

      <button className="disclosure" onClick={() => setAdvanced((v) => !v)}>
        {advanced ? "Hide advanced" : "Advanced"}
      </button>
      {advanced && (
        <>
          <div className="setting">
            <label>Convert output format</label>
            <select value={settings.datalabFormat} onChange={(e) => onPersist({ datalabFormat: e.target.value })}>
              <option value="markdown">Markdown (.md)</option>
              <option value="html">HTML (.html)</option>
              <option value="json">JSON (.json)</option>
            </select>
          </div>
          <div className="setting">
            <label>
              Datalab pipeline ID <span className="muted">(optional)</span>
            </label>
            <input
              type="text"
              defaultValue={settings.datalabPipelineId ?? ""}
              placeholder="pl_… blank uses the standard convert API"
              onBlur={(e) => onPersist({ datalabPipelineId: e.target.value.trim() || null })}
            />
          </div>
          <div className="setting">
            <label>Summarize model</label>
            <input
              type="text"
              defaultValue={settings.summarizeModel}
              onBlur={(e) => onPersist({ summarizeModel: e.target.value.trim() || "claude-sonnet-4-6" })}
            />
          </div>
        </>
      )}
    </main>
  );
}
