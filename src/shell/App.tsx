import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ClockCounterClockwiseIcon, GearSixIcon, SparkleIcon } from "@phosphor-icons/react";
import { Button, Display, SplitPane } from "@ui";
import { commands } from "@/app/commands";
import { ACTIVE, BIG_RUN, DEFAULT_SETTINGS, EMPTY_SCAN } from "@/app/types";
import type { Job, JobId, Scan, SecretStatus, Settings, View } from "@/app/types";
import { RunView } from "@/domains/run/RunView";
import { JOBS } from "@/domains/run/jobs";
import { canStartRun, planRun, runButtonLabel } from "@/domains/run/plan";
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
} from "@/platform/host";
import { useToast } from "./useToast";
import { useDocuments } from "./useDocuments";
import { useDocumentSave } from "./useDocumentSave";
import { useCloseConfirm, useDragDrop, useWindowFocusClass } from "./useHostWindow";
import "./App.css";

/// The window is two applications at two widths. A launcher fits in 420px; a
/// split with a readable document does not, so the floor moves with the pane
/// rather than being one compromise that serves neither.
const COMPACT_MIN = { width: 420, height: 460 };
const EXPANDED_MIN = { width: 900, height: 460 };
/// Only reached if the compact size was never measured. Matches the size in
/// `tauri.conf.json`, so a collapse can never leave the window at a size the
/// app has no opinion about.
const COMPACT_FALLBACK = { width: 560, height: 560 };

export default function App() {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [secrets, setSecrets] = useState<SecretStatus>({ datalab: false, revai: false, backend: false });
  const [jobs, setJobs] = useState<Job[]>([]);
  const [scan, setScan] = useState<Scan>(EMPTY_SCAN);
  const [now, setNow] = useState(() => Date.now());
  const [view, setView] = useState<View>("run");
  /// Bumped once when a run finishes. Refreshes the already-done counts and an
  /// open History panel — a 200-file run emits hundreds of job-updated events,
  /// so reacting to those instead would re-scan the disk hundreds of times.
  const [runsFinished, setRunsFinished] = useState(0);
  const [starting, setStarting] = useState(false);
  const wasRunning = useRef(false);
  const autoClear = useRef(false);

  const { toast, showToast } = useToast();
  const { docs, activeId, mode, openJob, openHistory, select, closeDoc, edit, setMode, setDocMeta } =
    useDocuments({ showToast });
  const { saveDoc, requestClose } = useDocumentSave({ docs, activeId, closeDoc, setDocMeta });

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

  useEffect(() => {
    void (async () => {
      const [s, k, j] = await Promise.all([
        commands.getSettings(),
        commands.secretStatus(),
        commands.listJobs(),
      ]);
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
      setScan(EMPTY_SCAN);
      return;
    }
    let live = true;
    void commands
      .scanInputs(settings.inputs)
      .then((s) => live && setScan(s))
      .catch(() => live && setScan(EMPTY_SCAN));
    return () => {
      live = false;
    };
  }, [settings.inputs, settings.datalabFormat, settings.datalabPipelineId, settings.outputDir, runsFinished]);

  const persist = useCallback((patch: Partial<Settings>) => {
    setSettings((prev) => {
      const next = { ...prev, ...patch };
      void commands.saveSettings(next).catch(() => undefined);
      return next;
    });
  }, []);

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
    // eslint-disable-next-line react-hooks/set-state-in-effect -- selection autodetect; deps are the scan counts only
    setSettings((prev) => {
      // Read outputDir from prev rather than a dependency, so defaulting it
      // can't retrigger this effect.
      const next = { ...prev, jobType, outputDir: prev.outputDir ?? scan.suggestedOutput };
      void commands.saveSettings(next).catch(() => undefined);
      return next;
    });
  }, [scan.convert, scan.transcribe, scan.suggestedOutput]);

  const mutateInputs = useCallback((fn: (cur: string[]) => string[]) => {
    setSettings((prev) => {
      const next = { ...prev, inputs: fn(prev.inputs) };
      void commands.saveSettings(next).catch(() => undefined);
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
  const wasExpanded = useRef(false);
  /// The launcher's size, taken the moment before it grows, so collapsing
  /// puts the window back where the user had it rather than at a default.
  const compactSize = useRef<{ width: number; height: number } | null>(null);
  const sizesRef = useRef(settings);

  useEffect(() => {
    sizesRef.current = settings;
  }, [settings]);

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
        const back = compactSize.current ?? COMPACT_FALLBACK;
        await resizeWindow(back.width, back.height);
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
  const { skipping, copying, toRun, colliding } = planRun(
    settings.jobType,
    inputCount,
    scan,
    settings.skipAlreadyDone,
  );

  const hasKey = secrets[job.secret];
  const canRun = canStartRun({
    hasInputs: settings.inputs.length > 0,
    hasOutput: Boolean(settings.outputDir),
    hasKey,
    toRun,
    copying,
    running,
    starting,
  });

  const run = async () => {
    // `running` is derived from the job list, which stays empty until the first
    // job-updated event lands — so without this guard a double-click fires two
    // runs before the button ever disables.
    if (starting) return;
    // A large batch is irreversible spend the moment it starts — Stop only
    // helps once you have noticed. Confirm the size and the cost driver first.
    if (toRun >= BIG_RUN) {
      const go = await confirm(
        `This will send ${toRun} files to ${job.service}` +
          (settings.jobType === "convert" && settings.datalabHighAccuracy
            ? ", with high-accuracy convert on (slower, more credits per page)."
            : ".") +
          "\n\nEach file is billed by the provider.",
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
  if (!hasKey) hint = `Add your ${job.service} key in Settings`;
  else if (settings.inputs.length > 0 && inputCount === 0) {
    if (other > 0) {
      hint = `Those look like ${settings.jobType === "transcribe" ? "documents" : "media files"} — switch to ${settings.jobType === "transcribe" ? "Convert" : "Transcribe"}`;
    } else if (scan.alreadyText > 0) {
      hint = `Already text — nothing to extract from ${scan.alreadyText} file${scan.alreadyText > 1 ? "s" : ""}`;
    } else {
      hint = "Nothing to do here. Convert takes PDF, Office and image files; Transcribe takes audio and video.";
    }
  } else if (toRun === 0 && copying === 0 && skipping > 0) {
    hint = `All ${skipping} already in this folder — turn off “Skip files already done” in Settings to run them again`;
  } else if (settings.inputs.length > 0 && !settings.outputDir) {
    // Last in the chain on purpose: the branches above are more actionable, and
    // this state is only reachable when the autodetect couldn't guess a folder
    // (files from two different parents), so it is the rarer answer.
    hint = "Choose an output folder for the results";
  }

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
            if (!hasKey) setView("settings");
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
