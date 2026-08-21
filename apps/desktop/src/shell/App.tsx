import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FileTextIcon, PlayIcon } from "@phosphor-icons/react";
import { Button, Mono, Sheet, SplitPane, StatusDot } from "@ui";
import { fmtElapsed } from "@/app/format";
import { barStatus, runCounter } from "./barStatus";
import { conversionClient } from "@/app/api";
import { commands } from "@/app/commands";
import { ACTIVE, BIG_RUN, DEFAULT_SETTINGS, EMPTY_SCAN } from "@/app/types";
import type {
  FileRow,
  Job,
  OnboardingConversionMode,
  ProjectSummary,
  Scan,
  SecretStatus,
  Settings,
  View,
  WorkspaceInfo,
} from "@/app/types";
import { LibraryPane } from "@/domains/library/LibraryPane";
import { WorkspaceViewNav } from "@/domains/library/WorkspaceViewNav";
import { OnboardingGate } from "@/domains/onboarding/OnboardingGate";
import { conversionPatch } from "@/domains/onboarding/conversionMode";
import { RunView } from "@/domains/run/RunView";
import { JOBS } from "@/domains/run/jobs";
import {
  autodetectJob,
  canStartRun,
  effectiveSkipAlreadyDone,
  largeRunConfirmation,
  planRun,
  runButtonLabel,
} from "@/domains/run/plan";
import {
  missingConversionCredentials,
  planConversionRoutes,
  type ConversionCapabilities,
} from "@/domains/run/routes";
import { HistoryPanel } from "@/domains/history/HistoryPanel";
import { SettingsPanel } from "@/domains/settings/SettingsPanel";
import { DocumentPane } from "@/domains/thread/DocumentPane";
import { FileInspector } from "@/domains/thread/FileInspector";
import { isDirty, type OpenDoc } from "@/domains/thread/model";
import {
  confirm,
  copyToClipboard,
  onWindowResized,
  pickDirectory,
  pickFiles,
  pickFolders,
  resizeWindow,
  setWindowMaxSize,
  setWindowMinSize,
  setWindowResizable,
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

/// How often to re-ask the conversion service what it can do while the backend
/// route is selected.
const CAPABILITY_PROBE_INTERVAL_MS = 15_000;

/// How long the window has to sit still before its size is written back. A
/// live drag reports every frame, and each write is a settings save.
const RESIZE_SETTLE_MS = 400;

/// The persisted factor is not trusted. A hand-edited settings.json can hold
/// anything, and handing that to the webview scales the app to nothing.
function clampZoom(factor: number): number {
  if (!Number.isFinite(factor) || factor <= 0) return ZOOM.default;
  return Math.min(ZOOM.max, Math.max(ZOOM.min, factor));
}

export default function App() {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  /// The host has answered `get_settings`. Nothing renders before it does: a
  /// single frame at `DEFAULT_SETTINGS` has no workspace path, which is how
  /// first run is detected, so it would flash the onboarding gate at someone
  /// who has used the app for a year.
  const [loaded, setLoaded] = useState(false);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [secrets, setSecrets] = useState<SecretStatus>({ datalab: false, revai: false, backend: false });
  /// Starts true because that is the safe answer while the host is being
  /// asked. A backend token field shown by mistake is what breaks the session.
  /// One that appears a beat late costs nothing.
  const [appOwnsBackend, setAppOwnsBackend] = useState(true);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [scanResult, setScanResult] = useState<{ key: string; value: Scan }>({
    key: "",
    value: EMPTY_SCAN,
  });
  const [capabilities, setCapabilities] = useState<ConversionCapabilities>({ state: "idle" });
  const [now, setNow] = useState(() => Date.now());
  const [view, setView] = useState<View>("library");
  /// Settings is a sheet over the whole window rather than a view, so opening
  /// it no longer evicts the Run column in the middle of a run.
  const [settingsOpen, setSettingsOpen] = useState(false);
  /// Bumped once when a run finishes. Refreshes the already-done counts and an
  /// open History panel — a 200-file run emits hundreds of job-updated events,
  /// so reacting to those instead would re-scan the disk hundreds of times.
  const [runsFinished, setRunsFinished] = useState(0);
  const [starting, setStarting] = useState(false);
  const wasRunning = useRef(false);
  const wasWorkspace = useRef(false);
  const autoClear = useRef(false);
  /// The selection autodetect has already answered. See the effect below.
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
    openJob,
    openHistory,
    openPath,
    select,
    closeDoc,
    edit,
    setMode,
    setDocMeta,
  } = useDocuments({ showToast });
  const { saveDoc, requestClose } = useDocumentSave({ docs, activeId, closeDoc, setDocMeta });

  const scanKey = JSON.stringify([
    settings.inputs,
    settings.datalabFormat,
    settings.datalabPipelineId,
    settings.outputDir,
    // The route decides which extensions `scan_inputs` counts, so leaving
    // these out lets the Run button promise a count from the other route.
    settings.conversionRoute,
    runsFinished,
  ]);
  const scanCurrent = scanResult.key === scanKey;
  const scan = scanCurrent ? scanResult.value : EMPTY_SCAN;
  const job = useMemo(() => JOBS.find((j) => j.id === settings.jobType) ?? JOBS[0], [settings.jobType]);
  const inputCount = settings.jobType === "transcribe" ? scan.transcribe : scan.convert;

  /// First run, keyed on the one thing the app cannot work without. The gate
  /// replaces the whole tree while this holds.
  const onboarding = loaded && settings.workspacePath === null;
  const workspacePath = settings.workspacePath;
  /// A workspace is open, so the library is home and the run flow is off the
  /// default nav.
  const libraryMode = loaded && workspacePath !== null;

  /// Surface backend failures instead of dropping them on the floor.
  const call = useCallback(
    async (fn: () => Promise<unknown>) => {
      try {
        return await fn();
      } catch (e) {
        showToast(String(e));
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

  const queueSettingsSave = useCallback((next: Settings) => {
    const pending = settingsSave.current
      .catch(() => undefined)
      .then(() => commands.saveSettings(next));
    settingsSave.current = pending;
    void pending.catch(() => undefined);
  }, []);

  const applySettings = useCallback(
    (next: Settings) => {
      settingsRef.current = next;
      setSettings(next);
      queueSettingsSave(next);
    },
    [queueSettingsSave],
  );

  useEffect(() => {
    void (async () => {
      const [s, k, j, owns] = await Promise.all([
        commands.getSettings(),
        commands.secretStatus(),
        commands.listJobs(),
        commands.appOwnsBackend(),
      ]);
      // Spread over the defaults rather than trusting the host's shape: a key
      // the stored settings.json predates arrives absent, and `undefined` is
      // not `null`, so an absent workspacePath would read as "already set up".
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

  // The sidebar's list. Re-read once per finished run rather than per
  // job-updated event, for the same reason the history panel does.
  useEffect(() => {
    if (!libraryMode) return;
    let live = true;
    // Behind the pending settings save. Onboarding's last beat is what binds
    // the workspace, and the host answers this from that same file, so asking
    // ahead of the write returns "No workspace configured" and the sidebar
    // opens empty.
    const pendingSave = settingsSave.current;
    void pendingSave
      .then(() => commands.listProjects())
      .then((p) => {
        if (live) setProjects(p);
      })
      .catch((e: unknown) => {
        // A workspace folder moved or unmounted in Finder fails here. Saying
        // so is the difference between "your workspace is gone" and a sidebar
        // that silently lists nothing.
        if (live) showToast(String(e));
      });
    return () => {
      live = false;
    };
  }, [libraryMode, workspacePath, runsFinished, showToast]);

  // Rescan whenever the selection changes: the counts drive the run label, the
  // job autodetect, and the suggested output folder.
  //
  // Also on the output format, because "already done" is format-specific, and
  // once per finished run, because that run is what just changed the answer.
  useEffect(() => {
    if (settings.inputs.length === 0) {
      // The scan is an async host call. Emptying the selection has to zero the
      // counts here or the run label and autodetect keep citing the last drop.
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

  // Behind the pending settings save, so the route the user just switched on
  // is on disk before the probe reports on it.
  useEffect(() => {
    if (settings.conversionRoute !== "backend") {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- reset external probe state when its route is disabled
      setCapabilities({ state: "idle" });
      return;
    }

    const probe = { cancelled: false };
    const pendingSave = settingsSave.current;
    setCapabilities({ state: "loading" });
    const ask = async () => {
      try {
        await pendingSave;
        const { data } = await conversionClient.GET("/api/v1/capabilities");
        if (!data) throw new Error("Conversion service capabilities were unavailable");
        if (probe.cancelled) return true;
        setCapabilities({
          state: "ready",
          acceptingJobs: data.data.conversion.acceptingJobs,
          inputFormats: data.data.conversion.inputFormats,
        });
        return true;
      } catch (e) {
        // Carry the host's reason through. It names what actually went wrong,
        // from "the service is starting" to a failed start, and the run hint
        // has nothing else to tell the user.
        const message = e instanceof Error ? e.message : String(e);
        if (!probe.cancelled) setCapabilities({ state: "unavailable", message });
        return false;
      }
    };
    // Keep asking. A one-shot probe latches Run off for the whole session when
    // the app starts before the service. `acceptingJobs` also goes stale, so
    // re-ask after a success.
    void ask();
    const retry = window.setInterval(() => void ask(), CAPABILITY_PROBE_INTERVAL_MS);
    return () => {
      probe.cancelled = true;
      window.clearInterval(retry);
    };
  }, [settings.conversionRoute, runsFinished]);

  const persist = useCallback((patch: Partial<Settings>) => {
    applySettings({ ...settingsRef.current, ...patch });
  }, [applySettings]);

  /// Stable, because the tree's fetch effects depend on it. An inline arrow
  /// would re-run them on every render.
  const setExpandedPaths = useCallback(
    (expandedPaths: string[]) => {
      persist({ expandedPaths });
    },
    [persist],
  );

  /// The library tree's session state. It lives up here rather than in the
  /// pane, because `left` swaps the pane out on every trip to Run and the
  /// children cache and the selection have to outlive that.
  const tree = useProjectTree({
    projects,
    expandedPaths: settings.expandedPaths,
    onExpandedChange: setExpandedPaths,
    refreshKey: runsFinished,
    showToast,
  });

  /// First run's answer, in one write: where the workspace is and how
  /// conversion runs. One write rather than two, so a crash between them
  /// cannot leave a workspace with no route.
  const completeOnboarding = useCallback(
    (workspace: WorkspaceInfo, mode: OnboardingConversionMode) => {
      persist({
        onboardingComplete: true,
        workspacePath: workspace.workspacePath,
        workspaceId: workspace.workspaceId,
        ...conversionPatch(mode),
      });
      // Grow here rather than leaving it to the effect below, so the library
      // does not paint one frame at the gate's size. Latching the flag is what
      // stops that effect repeating the grow a beat later.
      wasWorkspace.current = true;
      void (async () => {
        try {
          await setWindowResizable(true);
          await setWindowMinSize(WORKSPACE.minWidth, WORKSPACE.minHeight);
          const area = await workArea().catch(() => null);
          if (area) await setWindowMaxSize(area.width, area.height);
          await resizeWindow(WORKSPACE.width, WORKSPACE.height);
        } catch {
          // No window to size off a real host.
        }
      })();
      setView("library");
      // Only a brand-new workspace carries one. A user who deletes the file
      // never sees it again, because the host writes it once and says so here.
      if (workspace.welcomePath !== null) void openPath(workspace.welcomePath);
    },
    [openPath, persist],
  );

  // Match the job to what was dropped, and put results beside the input.
  //
  // Deliberately depends on the three values it actually reads, not on the
  // `scan` object. Depending on settings.jobType would make a manual click
  // un-clickable — the effect would switch it straight back — and depending on
  // the whole object would re-fire on an already-done refresh, which is the
  // same bug by a longer route.
  //
  // The three values are not enough on their own, which is what `answered` is
  // for: the scan drops to EMPTY_SCAN and back on every unrelated refresh, and
  // the counts returning read as a change. `autodetectJob` holds the rule that
  // one selection gets one answer.
  useEffect(() => {
    // Autodetect must run in an effect: it persists, and it must not depend on
    // jobType or a manual click is undone on the next render. See CLAUDE.md.
    const current = settingsRef.current;
    const detected = autodetectJob(current.inputs, answered.current, {
      convert: scan.convert,
      transcribe: scan.transcribe,
    });
    if (!detected) return;
    answered.current = detected.selection;
    // Read outputDir from the ref rather than a dependency, so defaulting it
    // can't retrigger this effect.
    applySettings({
      ...current,
      jobType: detected.jobType,
      outputDir: current.outputDir ?? scan.suggestedOutput,
    });
  }, [applySettings, scan.convert, scan.transcribe, scan.suggestedOutput]);

  const mutateInputs = useCallback((fn: (cur: string[]) => string[]) => {
    const current = settingsRef.current;
    applySettings({ ...current, inputs: fn(current.inputs) });
  }, [applySettings]);

  const addPaths = useCallback(
    (paths: string[]) => {
      if (paths.length) mutateInputs((cur) => Array.from(new Set([...cur, ...paths])));
    },
    [mutateInputs]
  );

  /// One file from the library, converted into the folder it already sits in.
  ///
  /// The host answers with a verdict and this renders it. Nothing here plans a
  /// route: the route plan, the key checks and the reuse rule all live in Rust,
  /// and a second planner in the webview would eventually disagree with the one
  /// a run uses.
  const convertOne = useCallback(
    async (row: FileRow) => {
      showPreview((cur) => (cur?.rel === row.rel ? null : cur));
      let out;
      try {
        out = await commands.convertOne(row.rel);
      } catch (e) {
        showToast(String(e));
        return;
      }
      if (out.kind === "blocked") {
        // Staging is not optional. The Run view's hint chain is gated on a
        // non-empty selection, so bouncing without it lands the user on an
        // empty Run view with no hint and a disabled button.
        addPaths([row.path]);
        showToast(out.message ?? "Cannot convert this file yet");
        setView("run");
        return;
      }
      if (out.kind === "copied") {
        showToast(out.message ?? "Copied a result from an earlier run — no charge");
        // A copy finishes inside the command, so no job ever runs and the
        // run-finished effect never fires. Refresh the tree so the row pairs.
        setRunsFinished((n) => n + 1);
      }
      // Queued needs nothing: the row's status arrives on the job-updated
      // stream, and `runsFinished` bumps when the run ends.
    },
    [addPaths, showPreview, showToast],
  );

  const addFiles = async () => {
    addPaths(await pickFiles());
  };

  const importFiles = useCallback(async () => {
    addPaths(await pickFiles());
    setView("run");
  }, [addPaths]);

  const addFolders = async () => {
    addPaths(await pickFolders());
  };

  const pickOutput = async () => {
    const dir = await pickDirectory();
    if (dir) persist({ outputDir: dir });
  };

  // Drops land window-wide, but the input list only exists in the run view —
  // so dropping onto an open panel staged the files with no visible sign at
  // all. Switch back so the drop has a visible result. Only on drop, never on
  // enter/over: yanking the user out of Settings because a drag passed over
  // the window would be worse than the bug.
  //
  // The sheet closes first, or the drop stages files and switches the view
  // behind a scrim, which is that same bug through the new door.
  const onDrop = useCallback(
    (paths: string[]) => {
      setSettingsOpen(false);
      addPaths(paths);
      setView("run");
    },
    [addPaths],
  );
  const dragging = useDragDrop(onDrop);

  const finished = jobs.filter((j) => j.status === "done" || j.status === "failed").length;
  const doneCount = jobs.filter((j) => j.status === "done").length;
  const failedCount = jobs.filter((j) => j.status === "failed").length;
  const running = jobs.some((j) => ACTIVE.includes(j.status));
  const total = jobs.length;

  const activeCount = jobs.filter((j) => ACTIVE.includes(j.status)).length;

  /// The live run, or null. The nav owns the centre of the title bar, so the
  /// run reports from the corner.
  const status = barStatus(jobs);

  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [running]);

  useWindowFocusClass();
  // Documents whose edit is still only in memory: the autosave has not landed
  // yet, or the host refused the write. Closing the window is one click from
  // quitting, so it has to say so.
  const dirtyCount = docs.filter((d) => isDirty(d.save)).length;
  useCloseConfirm(running, activeCount, dirtyCount);

  /// Zoom belongs to the app rather than to the document, and it outlives the
  /// session.
  const zoom = clampZoom(settings.zoom);
  const onZoom = useCallback(
    (next: number) => {
      // The ladder clamps at both ends, so Cmd+ at 200% asks for the factor
      // already in force. No toast for a step that changes nothing.
      if (next === zoom) return;
      persist({ zoom: next });
      showToast(`Zoom ${zoomLabel(next)}`);
    },
    [persist, showToast, zoom],
  );
  useZoom(zoom, onZoom);

  /// Something is in the right pane. The split still collapses to one column
  /// when there is not, which is the resting state on every launch, and the
  /// card counts because it has nowhere else to live.
  const expanded = docs.length > 0 || preview !== null;
  const sizesRef = useRef(settings);

  useEffect(() => {
    sizesRef.current = settings;
  }, [settings]);

  // First run opens at its own size. Bounds before size, as everywhere else:
  // the gate is taller and wider than the window's opening minimum, and macOS
  // clamps `setSize` to whatever is in force at that instant.
  useEffect(() => {
    if (!onboarding) return;
    void (async () => {
      try {
        await setWindowMinSize(ONBOARDING.minWidth, ONBOARDING.minHeight);
        await resizeWindow(ONBOARDING.width, ONBOARDING.height);
      } catch {
        // No window to size off a real host.
      }
    })();
  }, [onboarding]);

  // The window grows to the workspace once, when the settings load says a
  // workspace is bound, and opens where the user last left it. It never
  // shrinks back: binding a workspace is permanent, which is why
  // `wasWorkspace` latches instead of tracking.
  //
  // The statement order is load-bearing: macOS clamps `setSize` to the bounds
  // in force at that instant, so widen the bounds before growing.
  //
  // Native resizing is the OS animating a real window: there is nothing here
  // to match in CSS, and trying would fight it.
  useEffect(() => {
    if (!libraryMode || wasWorkspace.current) return;
    wasWorkspace.current = true;
    void (async () => {
      // The one read before the resize. The split restores its saved ratio a
      // frame after this effect starts and clamps that percentage against the
      // width in force right then, so every round trip here costs it.
      const area = await workArea().catch(() => null);
      await setWindowResizable(true);
      await setWindowMinSize(WORKSPACE.minWidth, WORKSPACE.minHeight);
      // A ceiling, so a size restored from a larger display cannot open a
      // window bigger than the screen it is opening on.
      if (area) await setWindowMaxSize(area.width, area.height);
      const { expandedWidth, expandedHeight } = sizesRef.current;
      await resizeWindow(expandedWidth ?? WORKSPACE.width, expandedHeight ?? WORKSPACE.height);
    })();
  }, [libraryMode]);

  // Remember the workspace size while the user is in it. The grow above only
  // reads it back, so without this the window returned to the default on every
  // launch.
  //
  // Our own `resizeWindow` reports through this same event. Writing that back
  // records the size actually on screen, which is what "where you left it"
  // means, ceiling clamp included.
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

  // Escape, in one ordered handler. Several surfaces answer this key and each
  // one binding its own listener is a race with no error and no test, so the
  // order lives here: the sheet outranks the document you are reading, and
  // neither closes the window. Fields keep their own Escape semantics.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // The sheet is a modal <dialog>. Its own `cancel` event owns the key,
      // and answering here as well would close the document behind it.
      if (settingsOpen) return;
      // The card is the top surface while it is up, so it goes before the
      // document it covers.
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
  }, [activeId, preview, requestClose, settingsOpen, showPreview]);

  // ⌘, from the app menu. A real menu item rather than a webview keydown,
  // which would compete with the editor in the same window.
  useEffect(() => {
    const un = commands.onOpenSettings(() => {
      setSettingsOpen(true);
    });
    return () => void un.then((f) => f());
  }, []);

  // ⌘O in workspace mode: same path as a drop — stage files and open Run.
  useEffect(() => {
    if (!libraryMode) return;
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.key.toLowerCase() !== "o" || e.shiftKey || e.altKey) return;
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
  }, [importFiles, libraryMode]);

  // When a run finishes, clear the input selection so the same files can't be
  // re-run by accident — Run greys out until new inputs are added. Gated on a
  // flag set by run(), so finishing a Retry doesn't wipe inputs the user has
  // already staged for the next batch; and skipped when nothing succeeded, so
  // a wholly failed run leaves the selection in place to try again.
  useEffect(() => {
    if (wasRunning.current && !running) {
      if (autoClear.current && doneCount > 0) mutateInputs(() => []);
      autoClear.current = false;
      // The run just changed what counts as already done, and added rows to
      // the history. One bump, not one per event.
      setRunsFinished((n) => n + 1);
    }
    wasRunning.current = running;
  }, [running, doneCount, mutateInputs]);

  // Every hook is above this line, which is the only reason the two
  // whole-window states below can return early at all.
  if (!loaded) return <div className="app" />;

  if (onboarding) {
    return (
      <div className="app">
        {/* The gate carries no chrome, but the window still has to be
            draggable: under titleBarStyle Overlay the webview covers the title
            bar and only `data-tauri-drag-region` moves it. */}
        <header className="bar" data-tauri-drag-region="deep">
          <div className="bar-lights" aria-hidden />
        </header>
        <OnboardingGate onDone={completeOnboarding} onToast={showToast} />
        <div className="toast-region" role="status" aria-live="polite">
          {toast && <div className="toast">{toast}</div>}
        </div>
      </div>
    );
  }

  // What the selection actually costs. Skip, copy, and billable work, plus the
  // skip-off case: files already in this folder that will still be sent and
  // land as numbered copies. The button only ever promises the billable number.
  const skipAlreadyDone = effectiveSkipAlreadyDone(
    settings.jobType,
    settings.conversionRoute,
    settings.skipAlreadyDone,
  );
  const { skipping, copying, toRun, colliding } = planRun(
    settings.jobType,
    inputCount,
    scan,
    skipAlreadyDone,
  );

  const conversionPlan = planConversionRoutes({
    files: scan.convertFiles,
    route: settings.conversionRoute,
    profile: settings.conversionProfile,
    capabilities,
    skipAlreadyDone,
  });
  const missingCredentials =
    settings.jobType === "convert"
      ? missingConversionCredentials(conversionPlan, secrets)
      : toRun > 0 && !secrets.revai
        ? ["revai" as const]
        : [];
  const routeBlocked = settings.jobType === "convert" && conversionPlan.blocked.length > 0;
  const preflightReady =
    scanCurrent && !routeBlocked && missingCredentials.length === 0;
  const canRun = canStartRun({
    hasInputs: settings.inputs.length > 0,
    hasOutput: Boolean(settings.outputDir),
    hasKey: preflightReady,
    toRun,
    copying,
    running,
    starting,
  });

  const run = async () => {
    // `running` is derived from the job list, which stays empty until the first
    // job-updated event lands — so without this guard a double-click fires two
    // runs before the button ever disables.
    if (starting || !canRun) return;
    // A large batch is irreversible spend the moment it starts — Stop only
    // helps once you have noticed. Confirm the size and the cost driver first.
    if (toRun >= BIG_RUN) {
      const backendFiles =
        settings.jobType === "convert" ? conversionPlan.backend.length : 0;
      const directFiles =
        settings.jobType === "convert" ? conversionPlan.direct.length : toRun;
      const go = await confirm(
        largeRunConfirmation({
          totalFiles: toRun,
          provider: job.service,
          backendFiles,
          directFiles,
          highAccuracy:
            settings.jobType === "convert" && settings.datalabHighAccuracy,
          profile: settings.conversionProfile,
        }),
        { title: `${job.verb} ${toRun} files?`, kind: "warning", okLabel: `${job.verb} all`, cancelLabel: "Cancel" }
      );
      if (!go) return;
    }
    const outputDir = settings.outputDir;
    if (!outputDir) return;
    setStarting(true);
    // Queued before the invoke so the run's own job-updated events, which the
    // listener applies with a functional update, land on top of a clean list.
    setJobs([]);
    try {
      const res = await commands.runPipeline(settings.inputs, outputDir, settings.jobType);
      if (res.copied > 0) {
        showToast(
          `Copied ${res.copied} result${res.copied > 1 ? "s" : ""} from an earlier run — no charge`
        );
      } else if (res.skipped > 0) {
        showToast(`Skipped ${res.skipped} file${res.skipped > 1 ? "s" : ""} already done`);
      }
      autoClear.current = true;
      // Copies finish inside run_pipeline, so a run with nothing billable never
      // makes `running` true and the run-finished effect never fires. Do its
      // two jobs here: refresh the counts, and clear the spent selection.
      if (res.count === 0) {
        setRunsFinished((n) => n + 1);
        if (res.copied > 0) mutateInputs(() => []);
        autoClear.current = false;
      }
    } catch (e) {
      showToast(String(e));
      // The run never started, so the backend still holds the previous run's
      // results. Re-sync rather than leaving the list wrongly empty.
      setJobs(await commands.listJobs().catch(() => []));
    } finally {
      setStarting(false);
    }
  };

  const stop = async () => {
    // Stopping is not finishing. Without this, the auto-clear below sees a run
    // end with at least one success and wipes the selection, so the files the
    // user stopped part-way through have to be dragged in again.
    autoClear.current = false;
    await call(() => commands.stopRun());
  };

  // The open document is what is on screen, which stops being the file
  // the moment an edit is saved. A row's Copy has to mean the same thing as the
  // pane's, so prefer the open document and otherwise read the file.
  //
  // There is no third fallback any more. The job row used to carry the
  // provider's bytes, which meant every result crossed IPC and stayed in the
  // webview for the session. `readDocumentText` reads the same file the pane
  // would, without the preview cap, so a result too large to open still copies.
  const copyText = async (j: Job | null) => {
    if (!j?.outputPath) return;
    const open = docs.find((d) => d.id === j.outputPath);
    let text = open?.text ?? null;
    if (text === null) {
      try {
        text = await commands.readDocumentText(j.outputPath);
      } catch {
        showToast("Copy failed");
        return;
      }
    }
    showToast((await copyToClipboard(text)) ? "Copied to clipboard" : "Copy failed");
  };

  // The pane copies what is on screen, which after an edit is not what the
  // job returned. Same two outcomes and the same two messages as a job row.
  const copyDoc = async (doc: OpenDoc) => {
    showToast((await copyToClipboard(doc.text)) ? "Copied to clipboard" : "Copy failed");
  };

  // The button names what is about to happen, and never overstates the cost:
  // when everything left is a copy it says so, because that run is free.
  const runLabel = runButtonLabel(job.verb, toRun, copying);

  // A selection that matches no job at all is a dead end unless we say why,
  // so name the formats rather than just reporting a count of zero.
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
    const reason = conversionPlan.blocked[0]?.reason;
    const count = conversionPlan.blocked.length;
    if (reason === "capabilities_pending") {
      hint = "Checking conversion service capabilities…";
    } else if (reason === "backend_unavailable") {
      // Settings holds no control that moves the service. Where it lives is a
      // deployment file the app never writes, so this hint carries the host's
      // own reason rather than pointing at a field that cannot help.
      const detail = capabilities.state === "unavailable" ? capabilities.message : undefined;
      hint = detail ?? `Conversion service unavailable for ${count} file${count > 1 ? "s" : ""}`;
    } else if (reason === "backend_not_accepting") {
      hint = `Conversion service is not accepting jobs for ${count} file${count > 1 ? "s" : ""} right now`;
      hintOpensSettings = true;
    } else {
      hint = `${count} file${count > 1 ? "s need" : " needs"} Datalab, but Local only forbids remote fallback — choose Standard or remove ${count > 1 ? "them" : "it"}`;
      hintOpensSettings = true;
    }
  } else if (missingCredentials.length > 0) {
    const labels = missingCredentials.map((secret) => {
      if (secret === "datalab") return "Datalab key";
      if (secret === "backend") return "backend token";
      return "Rev.ai key";
    });
    hint = `Add your ${labels.join(" and ")} in Settings`;
    hintOpensSettings = true;
  } else if (toRun === 0 && copying === 0 && skipping > 0) {
    hint = `All ${skipping} already in this folder — turn off “Skip files already done” in Settings to run them again`;
  } else if (settings.inputs.length > 0 && !settings.outputDir) {
    // Last in the chain on purpose: the branches above are more actionable, and
    // this state is only reachable when the autodetect couldn't guess a folder
    // (files from two different parents), so it is the rarer answer.
    hint = "Choose an output folder for the results";
  }

  /// Cobalt and a warning glyph promise a press. Only the hints `onHint` acts
  /// on get that treatment; the rest are status prose.
  const hintActionable =
    hint !== null &&
    (hintOpensSettings || hint === "Choose an output folder for the results");

  // What the run does besides the billable work. Copies are only mentioned
  // when the button isn't already announcing them.
  const noteParts: string[] = [];
  if (skipping > 0) noteParts.push(`${skipping} already in this folder`);
  // Skip-off re-runs still refuse to clobber: write_output numbers the file.
  // Naming that here is the whole increment: the silent duplicate was the bug.
  if (colliding > 0) {
    noteParts.push(
      `${colliding} already in this folder will be saved as numbered copies`,
    );
  }
  if (copying > 0 && toRun > 0) noteParts.push(`${copying} copied from an earlier run`);
  if (
    settings.jobType === "convert" &&
    settings.conversionRoute === "backend" &&
    conversionPlan.direct.length > 0
  ) {
    noteParts.push(
      `${conversionPlan.direct.length} routed direct to Datalab`,
    );
  }
  const note = noteParts.length > 0 ? noteParts.join(" · ") : null;

  const settingsPanel = (
    <SettingsPanel
      settings={settings}
      secrets={secrets}
      appOwnsBackend={appOwnsBackend}
      onPersist={persist}
      onSecrets={setSecrets}
      onToast={showToast}
      onClose={() => {
        setSettingsOpen(false);
      }}
    />
  );

  const historyPanel = (
    <HistoryPanel
      refreshKey={runsFinished}
      onChanged={() => setRunsFinished((n) => n + 1)}
      onOpen={(e) => void openHistory(e)}
      onToast={showToast}
    />
  );

  const runPanel = (
    <RunView
      settings={settings}
      scan={scan}
      jobs={jobs}
      dragging={dragging}
      now={now}
      running={running}
      canRun={canRun}
      runLabel={runLabel}
      hint={hint}
      hintActionable={hintActionable}
      note={note}
      finished={finished}
      total={total}
      doneCount={doneCount}
      failedCount={failedCount}
      job={job}
      selectedId={activeId}
      expanded={expanded}
      persist={persist}
      onAddFiles={() => void addFiles()}
      onAddFolders={() => void addFolders()}
      onPickOutput={() => void pickOutput()}
      onRemoveInput={(p) => mutateInputs((cur) => cur.filter((x) => x !== p))}
      onClearInputs={() => mutateInputs(() => [])}
      onRun={() => void run()}
      onStop={() => void stop()}
      onRetryFailed={() => void call(() => commands.retryFailed())}
      onRevealOutput={() => {
        const dir = settings.outputDir;
        if (dir) void call(() => commands.revealPath(dir));
      }}
      onPreview={(j) => void openJob(j)}
      onCopy={(j) => void copyText(j)}
      onRevealJob={(j) => {
        const path = j.outputPath;
        if (path) void call(() => commands.revealPath(path));
      }}
      onRetryJob={(id) => void call(() => commands.retryJob(id))}
      onHint={() => {
        if (hintOpensSettings) setSettingsOpen(true);
        else if (!settings.outputDir) void pickOutput();
      }}
    />
  );

  // The null arm never renders: the two early returns above prove `loaded` and
  // `!onboarding`, which together bind the workspace path. TypeScript cannot
  // see through a return, so the guard stays.
  const libraryPane =
    workspacePath === null ? null : (
      <LibraryPane
        workspacePath={workspacePath}
        projects={projects}
        tree={tree}
        jobs={jobs}
        /* One click rule: open the best openable thing on the row, and inspect
           when there is none. A plain .md opens. A paired deck.pdf opens its
           deck.md. An unpaired convertible and a binary raise the card. The
           host decides `openable`, so a click can never round-trip into a
           `read_document` failure toast. */
        onActivate={(row) => {
          if (row.openable) void openPath(row.path);
          else if (row.resultOpenable && row.resultPath !== null) void openPath(row.resultPath);
          else showPreview(row);
        }}
        onInspect={(row) => {
          showPreview(preview?.rel === row.rel ? null : row);
        }}
        onConvert={(row) => void convertOne(row)}
        onOpenSettings={() => {
          setSettingsOpen(true);
        }}
        onCreateProject={async (title) => {
          const created = await commands.createProject(title);
          setProjects((cur) => [...cur, created]);
          setView("library");
          try {
            setProjects(await commands.listProjects());
          } catch {
            // The project exists; a stale list is better than an error toast.
          }
        }}
        onRevealPath={(path) => {
          void call(() => commands.revealPath(path));
        }}
        onToast={showToast}
      />
    );

  // One view at a time, at the pane's full width. No shell wrapper: the tree,
  // the run column and the history all want the whole column, and the sidebar
  // that used to sit in front of them was what forced Run under its floor.
  const left = view === "run" ? runPanel : view === "history" ? historyPanel : libraryPane;

  // Always the same element in the same slot, collapsed to one pane when
  // nothing is open. Swapping between `<SplitPane>` and a bare `left` moves the
  // column to a different position in the tree, and React answers a move by
  // remounting: closing the last document threw away a half-typed API key in
  // Settings and whatever was in the History search box.
  const body = (
    <SplitPane
      className="workspace"
      collapsed={!expanded}
      start={left}
      end={
        <DocumentPane
          docs={docs}
          activeId={activeId}
          mode={mode}
          inspector={
            preview === null ? undefined : (
              <FileInspector
                row={preview}
                /* The card is the Convert control's primary home: the row's is
                   under the pointer only, and Enter on a row is what raises
                   this. */
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
          onCopy={(doc) => void copyDoc(doc)}
          onReveal={(doc) => {
            const path = doc.revealPath;
            if (path) void call(() => commands.revealPath(path));
          }}
        />
      }
      // Passed rather than left to the primitive's defaults. The split is a
      // window measurement, so it lives with the rest of them in geometry.ts.
      defaultStart={SPLIT.start}
      minStart={SPLIT.minStart}
      minEnd={SPLIT.minEnd}
      layout={settings.splitLayout ?? undefined}
      onLayoutChanged={(layout) => {
        persist({ splitLayout: layout });
      }}
    />
  );

  /// The live run, rendered in the one bar cell with room for it: the nav owns
  /// the centre for the whole length of the run.
  const runIndicator =
    status !== null ? (
      <>
        <StatusDot tone="live" />
        <Mono size="sm">{runCounter(status.done, status.total)}</Mono>
        {status.since !== null && (
          <>
            <Mono size="sm" tone="ghost" aria-hidden>
              ·
            </Mono>
            <Mono size="sm" tone="ghost">
              {fmtElapsed(now, status.since)}
            </Mono>
          </>
        )}
      </>
    ) : null;

  /* The only channel for errors that never reach a job row (key saves, reveal
     failures, clipboard). It must announce itself: role="status" so a screen
     reader hears it without stealing focus.

     Hoisted into a const because it renders in one of two places. A modal
     <dialog> draws in the top layer, above every z-index, so this region at
     the app root is invisible while the sheet is up — and saving an API key is
     the most common thing Settings does. */
  const toastRegion = (
    <div className="toast-region" role="status" aria-live="polite">
      {toast && <div className="toast">{toast}</div>}
    </div>
  );

  return (
    <div className="app">
      {/* `data-tauri-drag-region="deep"` is what makes the window draggable —
          the webview covers the title bar under titleBarStyle: Overlay, and
          WebKit ignores -webkit-app-region. "deep" so the brand text drags
          too; Tauri's handler already exempts buttons. */}
      <header className="bar" data-tauri-drag-region="deep">
        <div className="bar-lights" aria-hidden />
        {/* The nav, unconditionally: past the two early returns above, a
            workspace is bound and the library is home. */}
        <div className="bar-status">
          <WorkspaceViewNav
            view={view}
            onView={(next) => {
              setView(next);
            }}
          />
        </div>
        {/* Every panel lives in the centre Segmented nav, so the run reports
            in the corner instead: in the centre it displaced the nav and put
            History out of reach for the length of the run. */}
        <div className="bar-actions">{runIndicator}</div>
      </header>

      {body}

      <Sheet
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
        }}
        title="Settings"
        /* A modal dialog makes the rest of the window inert, and macOS drags a
           window by hit-testing this attribute rather than by a CSS state. With
           no strip inside the dialog the window cannot be moved while Settings
           is open. */
        head={<div data-tauri-drag-region="deep" />}
        overlay={toastRegion}
      >
        {settingsPanel}
      </Sheet>

      {!settingsOpen && toastRegion}
    </div>
  );
}
