import { useCallback, useEffect, useEffectEvent, useMemo, useRef, useState } from "react";
import { FileTextIcon, FolderOpenIcon, PlayIcon, XIcon } from "@phosphor-icons/react";
import { Button, Meta, Segmented, Sheet, SplitPane, StatusDot, Toast } from "@ui";
import { basename, fmtElapsed } from "@/app/format";
import { barStatus, runCounter } from "./barStatus";
import { autoOpenTarget, newlyDone, resultDirs, terminalIds } from "./runOutcome";
import { conversionClient } from "@/app/api";
import { commands } from "@/app/commands";
import { ACTIVE, BIG_RUN, DEFAULT_SETTINGS, EMPTY_SCAN } from "@/app/types";
import type {
  FileRow,
  Job,
  ProjectSummary,
  Scan,
  SecretStatus,
  Settings,
  View,
  WorkspaceInfo,
} from "@/app/types";
import { LibraryPane } from "@/domains/library/LibraryPane";
import { OnboardingGate } from "@/domains/onboarding/OnboardingGate";
import { RunView } from "@/domains/run/RunView";
import { JOBS } from "@/domains/run/jobs";
import {
  autodetectJob,
  canStartRun,
  planRun,
  runButtonLabel,
} from "@/domains/run/plan";
import { HistoryPanel } from "@/domains/history/HistoryPanel";
import { SettingsPanel } from "@/domains/settings/SettingsPanel";
import { DocumentPane } from "@/domains/thread/DocumentPane";
import { FileInspector } from "@/domains/thread/FileInspector";
import { isDirty } from "@/domains/thread/model";
import {
  centerWindow,
  confirm,
  copyToClipboard,
  onWindowResized,
  pickFiles,
  pickFolders,
  resizeWindow,
  setWindowMaxSize,
  setWindowMinSize,
  setWindowResizable,
  showWindow,
  windowSize,
  workArea,
} from "@/platform/host";
import { ONBOARDING, SPLIT, WORKSPACE, ZOOM } from "./geometry";
import { useToast } from "./useToast";
import { useDocuments } from "./useDocuments";
import { useProjectTree } from "./useProjectTree";
import { useDocumentSave } from "./useDocumentSave";
import { useCloseConfirm, useDragDrop, useWindowFocusClass } from "./useHostWindow";
import { useZoom, zoomLabel } from "./useZoom";
import "./App.css";

/// The three surfaces the nav switches between. Settings is a sheet, not one.
const VIEW_ITEMS = [
  { value: "library", label: "Library" },
  { value: "run", label: "Run" },
  { value: "history", label: "History" },
] satisfies { value: View; label: string }[];

const CAPABILITY_PROBE_INTERVAL_MS = 15_000;

type ConversionCapabilities =
  | { state: "loading" }
  /// The host's own reason: nothing in Settings moves the service.
  | { state: "unavailable"; message?: string }
  | { state: "ready"; acceptingJobs: boolean };

/// A live drag reports every frame, and each write is a settings save.
const RESIZE_SETTLE_MS = 400;

/// A hand-edited settings.json can scale the app to nothing.
function clampZoom(factor: number): number {
  if (!Number.isFinite(factor) || factor <= 0) return ZOOM.default;
  return Math.min(ZOOM.max, Math.max(ZOOM.min, factor));
}

export default function App() {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  /// Nothing renders before the host answers: a frame at `DEFAULT_SETTINGS`
  /// reads as first run.
  const [loaded, setLoaded] = useState(false);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  /// The catch-all's folder. Null until `ensure_workspace` answers.
  const [catchAllPath, setCatchAllPath] = useState<string | null>(null);
  const [secrets, setSecrets] = useState<SecretStatus>({ backend: false });
  /// Starts true: a backend token field shown by mistake breaks the session.
  const [appOwnsBackend, setAppOwnsBackend] = useState(true);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [scanResult, setScanResult] = useState<{ key: string; value: Scan }>({
    key: "",
    value: EMPTY_SCAN,
  });
  const [capabilities, setCapabilities] = useState<ConversionCapabilities>({ state: "loading" });
  const [now, setNow] = useState(() => Date.now());
  const [view, setView] = useState<View>("library");
  const [settingsOpen, setSettingsOpen] = useState(false);
  /// Bumped once when a run finishes. Reacting to `job-updated` instead would
  /// re-scan the disk hundreds of times a run.
  const [runsFinished, setRunsFinished] = useState(0);
  const [starting, setStarting] = useState(false);
  const wasRunning = useRef(false);
  /// The rows already finished when this run started. Every door that starts
  /// work sets it.
  const [terminalAtStart, setTerminalAtStart] = useState<ReadonlySet<number>>(new Set());
  /// Lets the finish effect read the list without depending on `jobs`.
  const jobsRef = useRef<Job[]>([]);
  const wasWorkspace = useRef(false);
  const autoClear = useRef(false);
  /// The selection autodetect has already answered.
  const answered = useRef<string | null>(null);
  const settingsRef = useRef<Settings>(DEFAULT_SETTINGS);
  const settingsSave = useRef<Promise<void>>(Promise.resolve());

  const { toast, showToast } = useToast();
  const {
    docs,
    activeId,
    mode,
    preview,
    showPreview,
    openPath,
    select,
    closeDoc,
    edit,
    renameDoc,
    setMode,
    setDocMeta,
  } = useDocuments({ showToast });
  const { saveDoc, requestClose } = useDocumentSave({ docs, activeId, closeDoc, setDocMeta });

  const scanKey = JSON.stringify([
    settings.inputs,
    // The active project is where a run writes, which the scan judges "already
    // here" against.
    settings.activeProjectPath,
    // The host plans the route against the live service, so a service that
    // comes back has to re-plan what it counted while it was down.
    capabilities.state,
    runsFinished,
  ]);
  const scanCurrent = scanResult.key === scanKey;
  /// Counts zero while the scan is re-asked, or Run over-counts. Nodes stay.
  const scan = scanCurrent ? scanResult.value : { ...EMPTY_SCAN, nodes: scanResult.value.nodes };
  const job = useMemo(() => JOBS.find((j) => j.id === settings.jobType) ?? JOBS[0], [settings.jobType]);
  const inputCount = settings.jobType === "transcribe" ? scan.transcribe : scan.convert;

  const onboarding = loaded && settings.workspacePath === null;
  const workspacePath = settings.workspacePath;
  const libraryMode = loaded && workspacePath !== null;

  const call = useCallback(
    async (fn: () => Promise<unknown>) => {
      try {
        return await fn();
      } catch (e) {
        showToast(String(e), "danger");
        return undefined;
      }
    },
    [showToast]
  );

  const upsert = useCallback((j: Job) => {
    setJobs((prev) => {
      const i = prev.findIndex((p) => p.id === j.id);
      if (i === -1) return [...prev, j];
      const copy = prev.slice();
      copy[i] = j;
      return copy;
    });
  }, []);

  const applySettings = useCallback((next: Settings) => {
    settingsRef.current = next;
    setSettings(next);
    const pending = settingsSave.current
      .catch(() => undefined)
      .then(() => commands.saveSettings(next));
    settingsSave.current = pending;
    // The UI already shows the new value, which a relaunch would lose.
    void pending.catch((e: unknown) => {
      showToast(String(e), "danger");
    });
  }, [showToast]);

  const persist = useCallback((patch: Partial<Settings>) => {
    applySettings({ ...settingsRef.current, ...patch });
  }, [applySettings]);

  useEffect(() => {
    void (async () => {
      const [s, k, j, owns] = await Promise.all([
        commands.getSettings(),
        commands.secretStatus(),
        commands.listJobs(),
        commands.appOwnsBackend(),
      ]);
      // A key the stored settings.json predates arrives `undefined`, not
      // `null`, so an absent workspacePath would read as already set up.
      const merged = { ...DEFAULT_SETTINGS, ...s };
      settingsRef.current = merged;
      setSettings(merged);
      setSecrets(k);
      setJobs(j);
      setAppOwnsBackend(owns);
      setLoaded(true);
    })();
  }, []);

  useEffect(() => {
    const un = commands.onJobUpdated(upsert);
    return () => void un.then((f) => f());
  }, [upsert]);

  // Above the run-finished effect, so that effect never reads last render's.
  useEffect(() => {
    jobsRef.current = jobs;
  }, [jobs]);

  // The sidebar's list, re-read once per finished run.
  useEffect(() => {
    if (!libraryMode) return;
    let live = true;
    // Behind the pending save: asking ahead of it returns "No workspace
    // configured".
    const pendingSave = settingsSave.current;
    // Captured before the chain: the state variable inside the `then` is the
    // render's value, not the one that arrived.
    let catchAll: string | null = null;
    void pendingSave
      // Before the listing, so a welcome file it seeds is on disk when the
      // tree reads the folder.
      .then(() => commands.ensureWorkspace())
      .then((info) => {
        if (!live || !info) return;
        catchAll = info.catchAllPath;
        setCatchAllPath(info.catchAllPath);
        if (info.welcomePath) void openPath(info.welcomePath);
      })
      .catch(() => {
        // It still lists below, and that failure is the one worth reporting.
      })
      .then(() => commands.listProjects())
      .then((p) => {
        if (!live) return;
        setProjects(p);
        // Adopt the catch-all when the bound destination is gone, or the run
        // has nowhere to write.
        const current = settingsRef.current.activeProjectPath;
        const stillThere = p.some((project) => project.path === current);
        if (!stillThere && p.length > 0) {
          const home = p.find((project) => project.path === catchAll) ?? p[0];
          persist({ activeProjectPath: home.path });
        }
      })
      .catch((e: unknown) => {
        // A workspace moved in Finder fails here, and unsaid it reads as an
        // empty sidebar.
        if (live) showToast(String(e), "danger");
      });
    return () => {
      live = false;
    };
  }, [libraryMode, workspacePath, runsFinished, showToast, openPath, persist]);

  // The counts drive the run label, the autodetect and the output folder, and
  // "already done" is format-specific.
  useEffect(() => {
    if (settings.inputs.length === 0) {
      // Emptying the selection has to zero the counts here.
      // eslint-disable-next-line react-hooks/set-state-in-effect -- reset derived scan; there is no store to subscribe to
      setScanResult({ key: scanKey, value: EMPTY_SCAN });
      return;
    }
    let live = true;
    const pendingSave = settingsSave.current;
    void pendingSave
      .then(() => commands.scanInputs(settings.inputs))
      .then((value) => live && setScanResult({ key: scanKey, value }))
      .catch(() => live && setScanResult({ key: scanKey, value: EMPTY_SCAN }));
    return () => {
      live = false;
    };
  }, [scanKey, settings.inputs]);

  useEffect(() => {
    const probe = { cancelled: false, ready: false };
    const ask = async () => {
      try {
        const { data } = await conversionClient.GET("/api/v1/capabilities");
        if (!data) throw new Error("Conversion service capabilities were unavailable");
        // The sidecar mints its token after the mount read, so the first
        // answer re-reads the keys.
        const keys = probe.ready ? null : await commands.secretStatus().catch(() => null);
        if (probe.cancelled) return true;
        probe.ready = true;
        setCapabilities({
          state: "ready",
          acceptingJobs: data.data.conversion.acceptingJobs,
        });
        if (keys) setSecrets(keys);
        return true;
      } catch (e) {
        // The run hint has nothing to say but the host's own reason.
        const message = e instanceof Error ? e.message : String(e);
        if (!probe.cancelled) setCapabilities({ state: "unavailable", message });
        return false;
      }
    };
    // A one-shot probe latches Run off for the session when the app starts
    // first, and `acceptingJobs` goes stale.
    void ask();
    const retry = window.setInterval(() => void ask(), CAPABILITY_PROBE_INTERVAL_MS);
    return () => {
      probe.cancelled = true;
      window.clearInterval(retry);
    };
  }, []);


  /// Stable: the tree's fetch effects depend on it.
  const setExpandedPaths = useCallback(
    (expandedPaths: string[]) => {
      persist({ expandedPaths });
    },
    [persist],
  );

  /// The library tree's session state. The run-finished effect reads it too.
  const tree = useProjectTree({
    projects,
    expandedPaths: settings.expandedPaths,
    onExpandedChange: setExpandedPaths,
    /// A finished run, or the tree calls a converted file unconverted until
    /// its folder moves.
    refreshKey: String(runsFinished),
    showToast,
  });

  /// The file the Move control acts on, derived from the tree's own selection,
  /// which climbs to the parent when a folder collapses. Null on a folder.
  const selectedRow = useMemo(() => {
    const rel = tree.selected;
    if (rel === null) return null;
    const cut = rel.lastIndexOf("/");
    if (cut < 0) return null;
    const row = tree.listings[rel.slice(0, cut)]?.entries.find((entry) => entry.rel === rel);
    return row === undefined || row.isDir ? null : row;
  }, [tree.listings, tree.selected]);

  /// First run's answer in one write: two would let a crash between them leave
  /// a workspace with no project.
  const completeOnboarding = useCallback(
    (workspace: WorkspaceInfo) => {
      persist({
        workspacePath: workspace.workspacePath,
        // A drop needs somewhere to land before anyone picks a project.
        activeProjectPath: workspace.catchAllPath,
      });
      setCatchAllPath(workspace.catchAllPath);
      // Grow here, or the library paints a frame at the gate's size. The flag
      // latches so the effect below does not repeat it.
      wasWorkspace.current = true;
      void (async () => {
        try {
          await setWindowResizable(true);
          await setWindowMinSize(WORKSPACE.minWidth, WORKSPACE.minHeight);
          const area = await workArea().catch(() => null);
          if (area) await setWindowMaxSize(area.width, area.height);
          await resizeWindow(WORKSPACE.width, WORKSPACE.height);
          await centerWindow(WORKSPACE.width, WORKSPACE.height);
        } catch {
          // No window to size off a real host.
        }
      })();
      setView("library");
      // A new workspace carries one, and so does one the gate seeded on a run
      // that quit before binding it.
      if (workspace.welcomePath !== null) void openPath(workspace.welcomePath);
    },
    [openPath, persist],
  );

  // Never depend on `settings.jobType`, which undoes a manual click, or on the
  // whole `scan`, which re-fires on an already-done refresh. `answered` covers
  // the scan dropping to EMPTY_SCAN and back.
  useEffect(() => {
    const current = settingsRef.current;
    const detected = autodetectJob(current.inputs, answered.current, {
      convert: scan.convert,
      transcribe: scan.transcribe,
    });
    if (!detected) return;
    answered.current = detected.selection;
    // Read from the ref, so the write cannot retrigger this.
    applySettings({ ...current, jobType: detected.jobType });
  }, [applySettings, scan.convert, scan.transcribe]);

  const mutateInputs = useCallback((fn: (cur: string[]) => string[]) => {
    const current = settingsRef.current;
    applySettings({ ...current, inputs: fn(current.inputs) });
  }, [applySettings]);

  const addPaths = useCallback(
    (paths: string[]) => {
      if (!paths.length) return;
      // Anything staged during a run is for the next one. Every staging door
      // comes here, so the auto-clear stands down here.
      autoClear.current = false;
      mutateInputs((cur) => Array.from(new Set([...cur, ...paths])));
    },
    [mutateInputs]
  );

  /// One file, converted into the folder it already sits in. The host answers
  /// a verdict and this renders it: a second planner here would disagree with
  /// the one a run uses.
  const convertOne = useCallback(
    async (row: FileRow) => {
      showPreview((cur) => (cur?.rel === row.rel ? null : cur));
      let out;
      try {
        // `convert_one` appends to the queue rather than clearing it, so
        // everything already in there belongs to an earlier run. Only on a
        // free queue: the host refuses anything else with `run_in_progress`,
        // and a baseline taken mid-run drops that run's finished rows from the
        // counter and turns its finish into a one-file auto-open.
        if (!jobsRef.current.some((j) => ACTIVE.includes(j.status))) {
          setTerminalAtStart(terminalIds(jobsRef.current));
        }
        out = await commands.convertOne(row.rel);
      } catch (e) {
        showToast(String(e), "danger");
        return;
      }
      if (out.kind === "blocked") {
        // No job takes this file: it is gone, or nothing converts its kind. Run
        // resolves neither, so staging it persists a path the host just refused.
        if (out.reason === "not_convertible") {
          showToast(out.message ?? "Cannot convert this file", "danger");
          return;
        }
        // Staging is not optional for the rest: the Run hint chain is gated on
        // a non-empty selection, so bouncing without it lands on an empty view.
        addPaths([row.path]);
        showToast(out.message ?? "Cannot convert this file yet", "danger");
        // A run already underway needs no bounce: the toast says it is staged.
        if (out.reason !== "run_in_progress") setView("run");
        return;
      }
      if (out.kind === "copied") {
        showToast(out.message ?? "Copied a result from an earlier run");
        // A copy finishes inside the command, so no job runs and the
        // run-finished effect never fires. Refresh the tree so the row pairs.
        setRunsFinished((n) => n + 1);
        // What a freshly converted result does: put it on screen.
        if (out.path !== null) {
          void openPath(out.path).then((opened) => {
            if (opened) setView("library");
          });
        }
      }
      // Queued needs nothing: its status arrives on the job-updated stream.
    },
    [addPaths, openPath, showPreview, showToast],
  );

  /// File one library row, and the result beside it, into another project. The
  /// host moves both halves or neither, and this follows the file.
  const moveToProject = useCallback(
    async (row: FileRow, projectRel: string) => {
      showPreview((cur) => (cur?.rel === row.rel ? null : cur));
      let landed;
      try {
        landed = await commands.moveToProject(row.rel, projectRel);
      } catch (e) {
        showToast(String(e), "danger");
        return;
      }
      // A tab is keyed on the path. Left behind, the next autosave writes to
      // the folder the file left, fails, and strands the edit.
      const workspace = settingsRef.current.workspacePath;
      if (workspace !== null) {
        renameDoc(row.path, `${workspace}/${landed}`);
        if (row.resultPath !== null) {
          renameDoc(row.resultPath, `${workspace}/${projectRel}/${basename(row.resultPath)}`);
        }
      }
      setRunsFinished((n) => n + 1);
      tree.toggle(projectRel, true, false);
      tree.select(landed);
      const project = projects.find((p) => p.path === projectRel);
      showToast(`Moved ${row.name} to ${project?.title ?? projectRel}`);
    },
    [projects, renameDoc, showPreview, showToast, tree],
  );

  /// Stage a drop where it is, or move it into the active project first when
  /// the setting asks. Never a copy: the result is the only new file.
  const stageDrop = useCallback(
    async (paths: string[]) => {
      const { moveDroppedFiles, activeProjectPath } = settingsRef.current;
      if (!moveDroppedFiles || activeProjectPath === null || paths.length === 0) {
        addPaths(paths);
        return;
      }
      try {
        const { staged, failed } = await commands.moveIntoProject(paths, activeProjectPath);
        // No watcher, so without this the moved files land invisibly.
        setRunsFinished((n) => n + 1);
        if (failed.length > 0) {
          const rest = failed.length - 1;
          showToast(rest > 0 ? `${failed[0]} (+${String(rest)} more)` : failed[0], "danger");
        }
        addPaths(staged);
      } catch (e) {
        // Nothing moved, so stage the drop where it is rather than lose it.
        showToast(String(e), "danger");
        addPaths(paths);
      }
    },
    [addPaths, showToast],
  );

  const importFiles = useCallback(async () => {
    const picked = await pickFiles();
    // The view switch waits for the picker: a cancel evicts nothing.
    if (picked.length === 0) return;
    setView("run");
    await stageDrop(picked);
  }, [stageDrop]);

  const addFolders = async () => {
    await stageDrop(await pickFolders());
  };

  /// One setting for both a drop and a run, so they cannot name two folders.
  const pickProject = useCallback(
    (rel: string) => {
      persist({ activeProjectPath: rel });
    },
    [persist],
  );

  // Drops land window-wide and the input list renders only in Run. Only on
  // drop: a passing drag must not yank the user out of Settings.
  const onDrop = useCallback(
    (paths: string[]) => {
      setSettingsOpen(false);
      setView("run");
      void stageDrop(paths);
    },
    [stageDrop],
  );
  const dragging = useDragDrop(onDrop);

  const doneCount = jobs.filter((j) => j.status === "done").length;
  const activeCount = jobs.filter((j) => ACTIVE.includes(j.status)).length;
  const running = activeCount > 0;

  /// This run's rows. Over the whole list, one file from the tree after a
  /// batch reads "200 / 201".
  const runJobs = jobs.filter((j) => !terminalAtStart.has(j.id));

  const status = barStatus(runJobs);

  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [running]);

  useWindowFocusClass();
  // Edits still only in memory. Closing the window is one click from quitting.
  const dirtyCount = docs.filter((d) => isDirty(d.save)).length;
  useCloseConfirm(running, activeCount, dirtyCount);

  const zoom = clampZoom(settings.zoom);
  const onZoom = useCallback(
    (next: number) => {
      // The ladder clamps, so Cmd+ at 200% asks for the factor already in force.
      if (next === zoom) return;
      persist({ zoom: next });
      showToast(`Zoom ${zoomLabel(next)}`);
    },
    [persist, showToast, zoom],
  );
  useZoom(zoom, onZoom);

  // Bounds before size: macOS clamps `setSize` to what is in force then.
  useEffect(() => {
    if (!onboarding) return;
    void (async () => {
      try {
        await setWindowMinSize(ONBOARDING.minWidth, ONBOARDING.minHeight);
        await resizeWindow(ONBOARDING.width, ONBOARDING.height);
        await centerWindow(ONBOARDING.width, ONBOARDING.height);
      } catch {
        // No window to size off a real host.
      } finally {
        // In a `finally` so a failed step still hands the user a window.
        await showWindow().catch(() => null);
      }
    })();
  }, [onboarding]);

  // The window grows once, to where the user last left it, and `wasWorkspace`
  // latches because binding a workspace is permanent. Statement order is
  // load-bearing: macOS clamps `setSize` to the bounds in force, so widen
  // the bounds first.
  useEffect(() => {
    if (!libraryMode || wasWorkspace.current) return;
    wasWorkspace.current = true;
    void (async () => {
      try {
        // The one read before the resize: the split restores its ratio a frame
        // from now, against whatever width is in force then.
        const area = await workArea().catch(() => null);
        await setWindowResizable(true);
        await setWindowMinSize(WORKSPACE.minWidth, WORKSPACE.minHeight);
        // A size restored from a larger display must not exceed this screen.
        if (area) await setWindowMaxSize(area.width, area.height);
        const { expandedWidth, expandedHeight } = settingsRef.current;
        // Clamp to the same ceiling: the position uses the size that lands.
        const want = { w: expandedWidth ?? WORKSPACE.width, h: expandedHeight ?? WORKSPACE.height };
        const width = area ? Math.min(want.w, area.width) : want.w;
        const height = area ? Math.min(want.h, area.height) : want.h;
        await resizeWindow(width, height);
        // macOS applies `setSize` asynchronously, so the host's `center()`
        // measures the frame from before the grow.
        await centerWindow(width, height);
      } catch {
        // No window to size off a real host.
      } finally {
        // In a `finally` so a failed step still reveals the hidden window.
        await showWindow().catch(() => null);
      }
    })();
  }, [libraryMode]);

  // The grow above only reads this back. Our own `resizeWindow` reports here
  // too, so what lands is the size on screen, ceiling clamp included.
  useEffect(() => {
    if (!libraryMode) return;
    let live = true;
    let settle = 0;
    const un = onWindowResized(() => {
      window.clearTimeout(settle);
      settle = window.setTimeout(() => {
        void windowSize()
          .then((size) => {
            if (live) persist({ expandedWidth: size.width, expandedHeight: size.height });
          })
          .catch(() => undefined);
      }, RESIZE_SETTLE_MS);
    });
    return () => {
      live = false;
      window.clearTimeout(settle);
      void un.then((f) => f());
    };
  }, [libraryMode, persist]);

  // Escape, in one ordered handler: a listener per surface is a silent race.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // The sheet's own `cancel` event owns the key.
      if (settingsOpen) return;
      if (view !== "library") return;
      // The card covers the document, so it closes first.
      if (preview !== null) {
        e.preventDefault();
        showPreview(null);
        return;
      }
      if (!activeId) return;
      const el = document.activeElement;
      if (el instanceof HTMLElement && el.closest("input, textarea, select, [contenteditable]")) {
        return;
      }
      e.preventDefault();
      void requestClose(activeId);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [activeId, preview, requestClose, settingsOpen, showPreview, view]);

  // A real menu item, because a webview keydown competes with the editor.
  useEffect(() => {
    const un = commands.onOpenSettings(() => {
      // The gate renders no sheet, so the flag would latch and ambush the
      // user when it closes.
      if (settingsRef.current.workspacePath === null) return;
      setSettingsOpen(true);
    });
    return () => void un.then((f) => f());
  }, []);

  // ⌘O stages files and opens Run, the same path a drop takes.
  useEffect(() => {
    if (!libraryMode) return;
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.ctrlKey || e.key.toLowerCase() !== "o" || e.shiftKey || e.altKey) {
        return;
      }
      // A keydown inside the modal <dialog> still bubbles here.
      if (settingsOpen) return;
      const el = document.activeElement;
      if (el instanceof HTMLElement && el.closest("input, textarea, select, [contenteditable]")) {
        return;
      }
      e.preventDefault();
      void importFiles();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [importFiles, libraryMode, settingsOpen]);

  /// Put a result on screen. `openPath` never touches the view, so a tab
  /// opened from Run or History lands where nobody can see it. Save first,
  /// because `openPath` does not flush the document it replaces. Finder is the
  /// fallback when the pane refuses the file.
  const reveal = useCallback(
    async (path: string | null) => {
      if (activeId) await saveDoc(activeId);
      if (await openPath(path)) {
        setView("library");
        return;
      }
      if (path) void call(() => commands.revealPath(path));
    },
    [activeId, call, openPath, saveDoc],
  );

  /// Effect Events: `reveal` changes identity with the active tab and
  /// `tree.toggle` as the tree loads, and the effect below must fire on a
  /// run's edges alone.
  const revealEvent = useEffectEvent(reveal);
  const openDestination = useEffectEvent(tree.toggle);

  // Clear the selection so the same files cannot be re-run by accident. The
  // flag is set by `run()` alone, so a Retry leaves the next batch staged.
  useEffect(() => {
    if (wasRunning.current && !running) {
      if (autoClear.current && doneCount > 0) mutateInputs(() => []);
      autoClear.current = false;
      setRunsFinished((n) => n + 1);
      // The bump re-lists the folders, so this only opens the branches. Read
      // off the results: a run writes beside each source, not to the project.
      const workspace = settingsRef.current.workspacePath;
      if (workspace !== null) {
        for (const rel of resultDirs(workspace, newlyDone(terminalAtStart, jobsRef.current))) {
          openDestination(rel, true, false);
        }
      }
      // One file is a request to read it. A batch is not, so nothing opens.
      const target = autoOpenTarget(terminalAtStart, jobsRef.current);
      if (target !== null) {
        void revealEvent(target.outputPath);
      }
    }
    wasRunning.current = running;
  }, [running, doneCount, mutateInputs, terminalAtStart]);

  /* The only channel for errors that never reach a job row. Hoisted because a
     modal <dialog> draws in the top layer, so a region at the app root is
     invisible while the sheet is up. */
  const toastRegion = toast ? <Toast tone={toast.tone}>{toast.text}</Toast> : null;

  // Every hook is above this line, which lets these two return early.
  if (!loaded) return <div className="app" />;

  if (onboarding) {
    return (
      <div className="app">
        {/* No chrome, but the window still has to drag. */}
        <header className="bar" data-tauri-drag-region="deep">
          <div className="bar-lights" aria-hidden />
        </header>
        <OnboardingGate onDone={completeOnboarding} onToast={showToast} />
        {toastRegion}
      </div>
    );
  }

  // Skip, copy, conversions, and files resent as numbered copies. The
  // button promises the conversion count alone.
  const skipAlreadyDone = settings.skipAlreadyDone;
  const { skipping, copying, toRun, colliding } = planRun(
    settings.jobType,
    inputCount,
    scan,
    skipAlreadyDone,
  );

  /// Both jobs run in the local service, so its state belongs in the
  /// preflight. The scan already dropped what the service cannot take.
  const serviceBlocked =
    capabilities.state !== "ready"
      ? capabilities.state
      : capabilities.acceptingJobs
        ? null
        : "not_accepting";
  const missingToken = toRun > 0 && !secrets.backend;
  /// For naming and revealing the destination, never for deciding it. Keep it
  /// in step with `output_dir_for`, which settles that.
  const destination =
    workspacePath !== null && settings.activeProjectPath !== null
      ? {
          rel: settings.activeProjectPath,
          path: `${workspacePath}/${settings.activeProjectPath}`,
        }
      : null;

  const routeBlocked = serviceBlocked !== null && toRun > 0;
  const preflightReady = scanCurrent && !routeBlocked && !missingToken;
  const canRun = canStartRun({
    hasInputs: settings.inputs.length > 0,
    hasOutput: destination !== null,
    hasKey: preflightReady,
    toRun,
    copying,
    running,
    starting,
  });

  const run = async () => {
    // `running` stays false until the first event lands, so without this a
    // double-click fires two runs.
    if (starting || !canRun) return;
    if (destination === null) return;
    setStarting(true);
    // Cleared before the invoke, so this run's events land on a clean list.
    setJobs([]);
    // `run_pipeline` clears the host's queue too.
    setTerminalAtStart(new Set());
    try {
      // Behind the pending save: the host reads the destination off disk.
      await settingsSave.current;
      const res = await commands.runPipeline(settings.inputs, settings.jobType);
      if (res.copied > 0) {
        showToast(
          `Copied ${res.copied} result${res.copied > 1 ? "s" : ""} from an earlier run`
        );
      } else if (res.skipped > 0) {
        showToast(`Skipped ${res.skipped} file${res.skipped > 1 ? "s" : ""} already done`);
      }
      autoClear.current = true;
      // Copies finish inside `run_pipeline`, so a run with nothing to convert
      // never makes `running` true.
      if (res.count === 0) {
        setRunsFinished((n) => n + 1);
        if (res.copied > 0) mutateInputs(() => []);
        autoClear.current = false;
      }
    } catch (e) {
      showToast(String(e), "danger");
      // The run never started, so the host still holds the previous run's rows.
      setJobs(await commands.listJobs().catch(() => []));
    } finally {
      setStarting(false);
    }
  };

  const stop = async () => {
    // Stopping is not finishing: the auto-clear would wipe the selection.
    autoClear.current = false;
    const stopped = await call(() => commands.stopRun());
    if (typeof stopped === "number" && stopped > 0) {
      showToast(`Stopped ${String(stopped)} file${stopped === 1 ? "" : "s"}`);
    }
  };

  /// Stop turns a cancelled batch into failed rows, so one click here can run
  /// all of it again.
  const retryFailed = async () => {
    const failed = jobs.filter((j) => j.status === "failed");
    if (failed.length === 0) return;
    if (failed.length >= BIG_RUN) {
      const go = await confirm(
        `This will run ${String(failed.length)} files again, including anything Stop cancelled.`,
        {
          title: `Retry ${String(failed.length)} files?`,
          kind: "warning",
          okLabel: "Retry all",
          cancelLabel: "Cancel",
        },
      );
      if (!go) return;
    }
    // The retried rows are this run's work, so they leave the baseline.
    const before = terminalIds(jobs);
    for (const j of failed) before.delete(j.id);
    setTerminalAtStart(before);
    await call(() => commands.retryFailed());
  };

  const retryJob = async (id: number) => {
    // A retry inside a run joins it, and must not disturb its baseline.
    const before = running ? new Set(terminalAtStart) : terminalIds(jobs);
    before.delete(id);
    setTerminalAtStart(before);
    await call(() => commands.retryJob(id));
  };

  // Prefer the open document, which is what the pane shows. The file is the
  // fallback, and `readDocumentText` has no preview cap.
  const copyText = async (j: Job | null) => {
    if (!j?.outputPath) return;
    const open = docs.find((d) => d.id === j.outputPath);
    let text = open?.text ?? null;
    if (text === null) {
      try {
        text = await commands.readDocumentText(j.outputPath);
      } catch {
        showToast("Copy failed", "danger");
        return;
      }
    }
    const copied = await copyToClipboard(text);
    showToast(copied ? "Copied to clipboard" : "Copy failed", copied ? "info" : "danger");
  };

  // The button never overstates the work: a run of copies alone converts nothing.
  const runLabel = runButtonLabel(job.verb, toRun, copying);

  // A selection matching no job needs the formats named, not a count of zero.
  const other = settings.jobType === "transcribe" ? scan.convert : scan.transcribe;
  let hint: string | null = null;
  let hintOpensSettings = false;
  if (settings.inputs.length > 0 && !scanCurrent) {
    hint = "Checking the selected files…";
  } else if (settings.inputs.length > 0 && inputCount === 0) {
    if (other > 0) {
      hint = `Those look like ${settings.jobType === "transcribe" ? "documents" : "media files"} — switch to ${settings.jobType === "transcribe" ? "Convert" : "Transcribe"}`;
    } else if (scan.alreadyText > 0) {
      hint = `Already text — nothing to extract from ${scan.alreadyText} file${scan.alreadyText > 1 ? "s" : ""}`;
    } else {
      hint = "Nothing to do here. Convert takes PDF, Office and image files; Transcribe takes audio and video.";
    }
  } else if (routeBlocked) {
    const count = toRun;
    if (serviceBlocked === "loading") {
      hint = "Checking conversion service capabilities…";
    } else if (serviceBlocked === "unavailable") {
      // Carry the host's own reason.
      const detail = capabilities.state === "unavailable" ? capabilities.message : undefined;
      hint = detail ?? `Conversion service unavailable for ${count} file${count > 1 ? "s" : ""}`;
    } else {
      hint = `Conversion service is not accepting jobs for ${count} file${count > 1 ? "s" : ""} right now`;
    }
  } else if (missingToken) {
    hint = "Add your backend token in Settings";
    hintOpensSettings = true;
  } else if (toRun === 0 && copying === 0 && skipping > 0) {
    hint = `All ${skipping} already have a result beside them — turn off “Skip files already done” in Settings to run them again`;
    hintOpensSettings = true;
  }

  // What the run does besides converting.
  const noteParts: string[] = [];
  if (skipping > 0) noteParts.push(`${skipping} already done`);
  // Skip-off re-runs still refuse to clobber: `write_output` numbers the file.
  if (colliding > 0) {
    noteParts.push(
      `${colliding} already have a result and will be saved as numbered copies`,
    );
  }
  if (copying > 0 && toRun > 0) noteParts.push(`${copying} copied from an earlier run`);
  const note = noteParts.length > 0 ? noteParts.join(" · ") : null;

  const settingsPanel = (
    <SettingsPanel
      settings={settings}
      secrets={secrets}
      appOwnsBackend={appOwnsBackend}
      onPersist={persist}
      onSecrets={setSecrets}
      onToast={showToast}
    />
  );

  const historyPanel = (
    <HistoryPanel
      refreshKey={runsFinished}
      dragging={dragging}
      onChanged={() => setRunsFinished((n) => n + 1)}
      onOpen={(e) => void reveal(e.outputPath)}
      onToast={showToast}
    />
  );

  const runPanel = (
    <RunView
      settings={settings}
      scan={scan}
      jobs={jobs}
      runJobs={runJobs}
      projects={projects}
      dragging={dragging}
      now={now}
      running={running}
      canRun={canRun}
      runLabel={runLabel}
      hint={hint}
      hintActionable={hintOpensSettings}
      note={note}
      job={job}
      selectedId={activeId}
      persist={persist}
      onAddFiles={() => void importFiles()}
      onAddFolders={() => void addFolders()}
      onPickProject={pickProject}
      onRemoveInput={(p) => mutateInputs((cur) => cur.filter((x) => x !== p))}
      onClearInputs={() => mutateInputs(() => [])}
      onRun={() => void run()}
      onStop={() => void stop()}
      onRetryFailed={() => void retryFailed()}
      onRevealOutput={() => {
        // A run writes beside each source, not into the project folder.
        const written = jobs.filter((j) => j.status === "done" && j.outputPath !== null);
        const path = written[written.length - 1]?.outputPath ?? destination?.path;
        if (path) void call(() => commands.revealPath(path));
      }}
      onPreview={(j) => void reveal(j.outputPath)}
      onCopy={(j) => void copyText(j)}
      onRevealJob={(j) => {
        const path = j.outputPath;
        if (path) void call(() => commands.revealPath(path));
      }}
      onRetryJob={(id) => void retryJob(id)}
      onHint={() => {
        if (hintOpensSettings) setSettingsOpen(true);
      }}
    />
  );

  // The null arm never renders: TypeScript cannot see through the two early
  // returns that bind the workspace path.
  const libraryPane =
    workspacePath === null ? null : (
      <LibraryPane
        workspacePath={workspacePath}
        projects={projects}
        catchAllPath={catchAllPath}
        activeProjectPath={settings.activeProjectPath}
        selected={selectedRow}
        tree={tree}
        jobs={jobs}
        /* The card follows the selection while it is up, or its Convert runs
           the file the user stopped looking at. It never raises the card. */
        onSelect={(row) => {
          showPreview((cur) => (cur === null ? null : row));
        }}
        /* Open the best openable thing on the row, and inspect when there is
           none. `openable` leaves out the encoding, so a failed open falls
           back to the card rather than a bare toast. */
        onActivate={(row) => {
          // The document and the card render only in the library pane.
          setView("library");
          const path = row.openable
            ? row.path
            : row.resultOpenable
              ? row.resultPath
              : null;
          if (path === null) {
            showPreview(row);
            return;
          }
          void openPath(path).then((opened) => {
            if (!opened) showPreview(row);
          });
        }}
        onInspect={(row) => {
          // A card the Run column was covering must show, not toggle off.
          const covered = view !== "library";
          setView("library");
          showPreview(!covered && preview?.rel === row.rel ? null : row);
        }}
        onConvert={(row) => void convertOne(row)}
        onSetActiveProject={pickProject}
        onMoveToProject={(row, projectRel) => void moveToProject(row, projectRel)}
        onOpenSettings={() => {
          setSettingsOpen(true);
        }}
        onCreateProject={async (title) => {
          const created = await commands.createProject(title);
          setProjects((cur) => [...cur, created]);
          try {
            setProjects(await commands.listProjects());
          } catch {
            // The project exists. A stale list beats an error toast.
          }
        }}
        onRevealPath={(path) => {
          void call(() => commands.revealPath(path));
        }}
        onToast={showToast}
      />
    );

  // Two panes, always: the library stays put under whatever the nav swaps in.
  // Always the same element in the same slot, or React remounts the column and
  // throws away a half-typed API key.
  const body = (
    <SplitPane
      className="workspace"
      start={libraryPane}
      end={
        view === "run" ? (
          runPanel
        ) : view === "history" ? (
          historyPanel
        ) : (
        <DocumentPane
          docs={docs}
          activeId={activeId}
          mode={mode}
          dragging={dragging}
          onPick={() => void importFiles()}
          inspector={
            preview === null ? undefined : (
              <FileInspector
                row={preview}
                /* The card is Convert's primary home: the row's answers to
                   the pointer only. */
                primary={
                  preview.resultOpenable && preview.resultPath !== null ? (
                    <Button
                      variant="primary"
                      size="sm"
                      icon={<FileTextIcon />}
                      onClick={() => {
                        const path = preview.resultPath;
                        if (path) void openPath(path);
                      }}
                    >
                      Open result
                    </Button>
                  ) : preview.job !== null && preview.resultName === null ? (
                    <Button
                      variant="primary"
                      size="sm"
                      icon={<PlayIcon weight="fill" />}
                      onClick={() => void convertOne(preview)}
                    >
                      {preview.job === "transcribe" ? "Transcribe" : "Convert"}
                    </Button>
                  ) : preview.resultPath !== null ? (
                    /* Converted to something the pane cannot read, and the
                       card must never be actionless. */
                    <Button
                      variant="primary"
                      size="sm"
                      icon={<FolderOpenIcon />}
                      onClick={() => {
                        const path = preview.resultPath;
                        if (path) void call(() => commands.revealPath(path));
                      }}
                    >
                      Show result in Finder
                    </Button>
                  ) : undefined
                }
                onReveal={(path) => {
                  void call(() => commands.revealPath(path));
                }}
                onClose={() => {
                  showPreview(null);
                }}
              />
            )
          }
          onSelect={(id) => {
            if (activeId && id !== activeId) void saveDoc(activeId);
            select(id);
          }}
          onClose={(id) => {
            void requestClose(id);
          }}
          onModeChange={setMode}
          onEdit={edit}
          // The pane copies what is on screen, not what the job returned.
          onCopy={(doc) => {
            void copyToClipboard(doc.text).then((ok) => {
              showToast(ok ? "Copied to clipboard" : "Copy failed", ok ? "info" : "danger");
            });
          }}
          onReveal={(doc) => {
            void call(() => commands.revealPath(doc.id));
          }}
        />
        )
      }
      // The split is a window measurement, so it lives in geometry.ts.
      defaultStart={SPLIT.start}
      minStart={SPLIT.minStart}
      minEnd={SPLIT.minEnd}
      layout={settings.splitLayout ?? undefined}
      onLayoutChanged={(layout) => {
        persist({ splitLayout: layout });
      }}
    />
  );

  /// The live run, in the corner: the nav owns the centre of the bar.
  const runIndicator =
    status !== null ? (
      <>
        <StatusDot tone="live" />
        <Meta size="sm">{runCounter(status.done, status.total)}</Meta>
        {status.since !== null && (
          <>
            <Meta size="sm" tone="ghost" aria-hidden>
              ·
            </Meta>
            <Meta size="sm" tone="ghost">
              {fmtElapsed(now, status.since)}
            </Meta>
          </>
        )}
      </>
    ) : null;

  return (
    <div className="app">
      {/* The only thing that makes the window draggable: the webview covers
          the title bar and WebKit ignores -webkit-app-region. "deep" so the
          text drags too, and Tauri already exempts buttons. */}
      <header className="bar" data-tauri-drag-region="deep">
        <div className="bar-lights" aria-hidden />
        <div className="bar-status">
          <Segmented
            className="bar-nav"
            size="sm"
            label="Workspace"
            value={view}
            onChange={setView}
            options={VIEW_ITEMS}
          />
        </div>
        <div className="bar-actions">{runIndicator}</div>
      </header>

      {body}

      <Sheet
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
        }}
        title="Settings"
        /* A modal dialog makes the rest inert, and macOS drags by hit-testing
           this attribute, so the sheet needs its own strip. */
        head={<div data-tauri-drag-region="deep" />}
        titleActions={
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            icon={<XIcon />}
            aria-label="Close settings"
            onClick={() => {
              setSettingsOpen(false);
            }}
          />
        }
        overlay={toastRegion}
      >
        {settingsPanel}
      </Sheet>

      {!settingsOpen && toastRegion}
    </div>
  );
}
