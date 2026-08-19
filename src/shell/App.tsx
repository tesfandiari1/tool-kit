import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ClockCounterClockwiseIcon, GearSixIcon, SparkleIcon } from "@phosphor-icons/react";
import { Button, Display, SplitPane } from "@ui";
import { conversionClient } from "@/app/api";
import { commands } from "@/app/commands";
import { ACTIVE, BIG_RUN, DEFAULT_SETTINGS, EMPTY_SCAN } from "@/app/types";
import type { Job, JobId, Scan, SecretStatus, Settings, View } from "@/app/types";
import { RunView } from "@/domains/run/RunView";
import { JOBS } from "@/domains/run/jobs";
import {
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
import { isDirty, type OpenDoc } from "@/domains/thread/model";
import {
  confirm,
  copyToClipboard,
  pickDirectory,
  pickFiles,
  pickFolders,
  resizeWindow,
  setWindowMinSize,
  windowSize,
  type WindowSize,
} from "@/platform/host";
import { useToast } from "./useToast";
import { useDocuments } from "./useDocuments";
import { useDocumentSave } from "./useDocumentSave";
import { useCloseConfirm, useDragDrop, useWindowFocusClass } from "./useHostWindow";
import { useFitWindow } from "./useFitWindow";
import "./App.css";

/// The window is two applications at two widths. A launcher fits in 420px; a
/// split with a readable document does not, so the floor moves with the pane
/// rather than being one compromise that serves neither.
const COMPACT_MIN = { width: 420, height: 460 };
const EXPANDED_MIN = { width: 900, height: 460 };
/// Ultimate fallback, reached only when the launch-size read failed and no
/// compact size was ever measured. `tauri.conf.json` is the source of truth
/// for the first-run default; keep these numbers in sync with its
/// width/height so a collapse can never leave the window at a size the app
/// has no opinion about.
const COMPACT_FALLBACK = { width: 560, height: 560 };

export default function App() {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [secrets, setSecrets] = useState<SecretStatus>({ datalab: false, revai: false, backend: false });
  const [jobs, setJobs] = useState<Job[]>([]);
  const [scanResult, setScanResult] = useState<{ key: string; value: Scan }>({
    key: "",
    value: EMPTY_SCAN,
  });
  const [capabilities, setCapabilities] = useState<ConversionCapabilities>({ state: "idle" });
  const [now, setNow] = useState(() => Date.now());
  const [view, setView] = useState<View>("run");
  /// Bumped once when a run finishes. Refreshes the already-done counts and an
  /// open History panel — a 200-file run emits hundreds of job-updated events,
  /// so reacting to those instead would re-scan the disk hundreds of times.
  const [runsFinished, setRunsFinished] = useState(0);
  const [starting, setStarting] = useState(false);
  const wasRunning = useRef(false);
  const autoClear = useRef(false);
  const settingsRef = useRef<Settings>(DEFAULT_SETTINGS);
  const settingsSave = useRef<Promise<void>>(Promise.resolve());

  const { toast, showToast } = useToast();
  const { docs, activeId, mode, openJob, openHistory, select, closeDoc, edit, setMode, setDocMeta } =
    useDocuments({ showToast });
  const { saveDoc, requestClose } = useDocumentSave({ docs, activeId, closeDoc, setDocMeta });

  const scanKey = JSON.stringify([
    settings.inputs,
    settings.datalabFormat,
    settings.datalabPipelineId,
    settings.outputDir,
    runsFinished,
  ]);
  const scanCurrent = scanResult.key === scanKey;
  const scan = scanCurrent ? scanResult.value : EMPTY_SCAN;
  const job = useMemo(() => JOBS.find((j) => j.id === settings.jobType) ?? JOBS[0], [settings.jobType]);
  const inputCount = settings.jobType === "transcribe" ? scan.transcribe : scan.convert;

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
      const [s, k, j] = await Promise.all([
        commands.getSettings(),
        commands.secretStatus(),
        commands.listJobs(),
      ]);
      settingsRef.current = s;
      setSettings(s);
      setSecrets(k);
      setJobs(j);
    })();
  }, []);

  useEffect(() => {
    const un = commands.onJobUpdated(upsert);
    return () => void un.then((f) => f());
  }, [upsert]);

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

  // A backend URL is persisted before the host probes it. The host resolves
  // the URL from its own Settings on every request, so issuing the GET first
  // would race the save and occasionally inspect the previous server.
  useEffect(() => {
    if (settings.conversionRoute !== "backend") {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- reset external probe state when its route is disabled
      setCapabilities({ state: "idle" });
      return;
    }

    const probe = { cancelled: false };
    const pendingSave = settingsSave.current;
    setCapabilities({ state: "loading" });
    void (async () => {
      try {
        await pendingSave;
        const { data } = await conversionClient.GET("/api/v1/capabilities");
        if (!data) throw new Error("Conversion service capabilities were unavailable");
        if (probe.cancelled) return;
        setCapabilities({
          state: "ready",
          acceptingJobs: data.data.conversion.acceptingJobs,
          inputFormats: data.data.conversion.inputFormats,
        });
      } catch {
        if (!probe.cancelled) setCapabilities({ state: "unavailable" });
      }
    })();
    return () => {
      probe.cancelled = true;
    };
  }, [settings.backendUrl, settings.conversionRoute]);

  const persist = useCallback((patch: Partial<Settings>) => {
    applySettings({ ...settingsRef.current, ...patch });
  }, [applySettings]);

  // Match the job to what was dropped, and put results beside the input.
  //
  // Deliberately depends on the three values it actually reads, not on the
  // `scan` object. That is what makes it deterministic: it re-runs when the
  // selection changes and at no other time, so a new drop always re-detects,
  // and clicking a job by hand sticks until the next drop. Depending on
  // settings.jobType would make a manual click un-clickable — the effect would
  // switch it straight back — and depending on the whole object would re-fire
  // on an already-done refresh, which is the same bug by a longer route.
  useEffect(() => {
    if (scan.convert === 0 && scan.transcribe === 0) return;
    const jobType: JobId = scan.transcribe > scan.convert ? "transcribe" : "convert";
    // Autodetect must run in an effect: it persists, and it must not depend on
    // jobType or a manual click is undone on the next render. See CLAUDE.md.
    const current = settingsRef.current;
    // Read outputDir from the ref rather than a dependency, so defaulting it
    // can't retrigger this effect.
    const next = { ...current, jobType, outputDir: current.outputDir ?? scan.suggestedOutput };
    applySettings(next);
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

  const addFiles = async () => {
    addPaths(await pickFiles());
  };

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
  const onDrop = useCallback(
    (paths: string[]) => {
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

  const expanded = docs.length > 0;
  useFitWindow(!expanded);
  const wasExpanded = useRef(false);
  /// The launcher's size, taken the moment before it grows, so collapsing
  /// puts the window back where the user had it rather than at a default.
  const compactSize = useRef<WindowSize | null>(null);
  /// The size the window launched at, read once on mount below. Covers a
  /// collapse that fires before the first compact measurement lands — a
  /// document opened and closed inside its await — so even that returns to a
  /// real size rather than the hardcoded fallback.
  const launchSize = useRef<WindowSize | null>(null);
  const sizesRef = useRef(settings);

  useEffect(() => {
    sizesRef.current = settings;
  }, [settings]);

  // Window defaults live in tauri.conf.json; this read is how JS learns them
  // instead of hardcoding the config's numbers. A failure just leaves the
  // hardcoded fallback in charge.
  useEffect(() => {
    void windowSize()
      .then((size) => {
        launchSize.current = size;
      })
      .catch(() => undefined);
  }, []);

  // The window follows the document. Each mode remembers its own size, so
  // widening the workspace never leaves the launcher stretched, and a compact
  // window the user shrank by hand is what comes back.
  //
  // Native resizing is the OS animating a real window: there is nothing here
  // to match in CSS, and trying would fight it.
  useEffect(() => {
    if (expanded === wasExpanded.current) return;
    wasExpanded.current = expanded;
    void (async () => {
      const current = await windowSize();
      if (expanded) {
        compactSize.current = current;
        await setWindowMinSize(EXPANDED_MIN.width, EXPANDED_MIN.height);
        const { expandedWidth, expandedHeight } = sizesRef.current;
        await resizeWindow(
          expandedWidth ?? Math.max(current.width, EXPANDED_MIN.width),
          expandedHeight ?? current.height,
        );
      } else {
        // Record the workspace size before shrinking, or the next expand
        // reads back the launcher's.
        persist({ expandedWidth: current.width, expandedHeight: current.height });
        await setWindowMinSize(COMPACT_MIN.width, COMPACT_MIN.height);
        const w =
          compactSize.current?.width ?? launchSize.current?.width ?? COMPACT_FALLBACK.width;
        // Height is set by `useFitWindow` once the compact layout paints.
        await resizeWindow(w, COMPACT_MIN.height);
      }
    })();
  }, [expanded, persist]);

  // Escape closes the document you are reading, not the window and not the
  // panel beside it. Bound only while something is open, so an app with no
  // document never swallows the key. `requestClose` saves an edit first, then
  // asks before discarding if the write was refused.
  useEffect(() => {
    if (!activeId) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      void requestClose(activeId);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [activeId, requestClose]);

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

  // `Job.outputText` is what the provider returned, which stops being the file
  // the moment an edit is saved. A row's Copy has to mean the same thing as the
  // pane's, so prefer the open document, then the file, and fall back to the
  // provider's bytes only when there is no path to read.
  const copyText = async (j: Job | null) => {
    if (!j) return;
    let text = j.outputText;
    if (j.outputPath) {
      const open = docs.find((d) => d.id === j.outputPath);
      if (open) text = open.text;
      else {
        try {
          text = (await commands.readDocument(j.outputPath)).text;
        } catch {
          // Unreadable — too large, gone, not text. The provider's copy is
          // still worth having, so fall through rather than failing the copy.
        }
      }
    }
    if (!text) return;
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
      hint = `Conversion service unavailable for ${count} file${count > 1 ? "s" : ""} — check the backend URL in Settings`;
      hintOpensSettings = true;
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

  // The left column, and the whole body when nothing is open. Settings and
  // History replace this column only: a document is a place you are reading,
  // and changing a setting is not a reason to lose it.
  let left: ReactNode;
  switch (view) {
    case "settings":
      left = (
        <SettingsPanel
          settings={settings}
          secrets={secrets}
          onPersist={persist}
          onSecrets={setSecrets}
          onToast={showToast}
          onClose={() => setView("run")}
        />
      );
      break;
    case "history":
      left = (
        <HistoryPanel
          refreshKey={runsFinished}
          onChanged={() => setRunsFinished((n) => n + 1)}
          onOpen={(e) => void openHistory(e)}
          onToast={showToast}
          onClose={() => setView("run")}
        />
      );
      break;
    case "run":
      left = (
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
            if (hintOpensSettings) setView("settings");
            else if (!settings.outputDir) void pickOutput();
          }}
        />
      );
      break;
    default: {
      const _exhaustive: never = view;
      left = _exhaustive;
    }
  }

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
      layout={settings.splitLayout ?? undefined}
      onLayoutChanged={(layout) => {
        persist({ splitLayout: layout });
      }}
    />
  );

  return (
    <div className="app">
      {/* `data-tauri-drag-region="deep"` is what makes the window draggable —
          the webview covers the title bar under titleBarStyle: Overlay, and
          WebKit ignores -webkit-app-region. "deep" so the brand text drags
          too; Tauri's handler already exempts buttons. */}
      <header className="bar" data-tauri-drag-region="deep">
        <div className="bar-brand">
          <span className="mark" aria-hidden>
            <SparkleIcon weight="fill" />
          </span>
          <Display as="span" size="lg">
            Tool-Kit
          </Display>
        </div>
        <div className="bar-actions">
          <Button
            variant="ghost"
            iconOnly
            icon={<ClockCounterClockwiseIcon weight={view === "history" ? "fill" : "regular"} />}
            onClick={() => {
              setView((v) => (v === "history" ? "run" : "history"));
            }}
            title="History"
            aria-label="History"
          />
          <Button
            variant="ghost"
            iconOnly
            icon={<GearSixIcon weight={view === "settings" ? "fill" : "regular"} />}
            onClick={() => {
              setView((v) => (v === "settings" ? "run" : "settings"));
            }}
            title="Settings"
            aria-label="Settings"
          />
        </div>
      </header>

      {body}

      {/* The only channel for errors that never reach a job row (key saves,
          reveal failures, clipboard). It must announce itself: role="status"
          so a screen reader hears it without stealing focus. */}
      <div className="toast-region" role="status" aria-live="polite">
        {toast && <div className="toast">{toast}</div>}
      </div>
    </div>
  );
}
