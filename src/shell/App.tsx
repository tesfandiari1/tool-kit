import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ClockCounterClockwiseIcon, GearSixIcon, SparkleIcon } from "@phosphor-icons/react";
import { Button, Display } from "@ui";
import { commands } from "@/app/commands";
import { ACTIVE, BIG_RUN, DEFAULT_SETTINGS, EMPTY_SCAN } from "@/app/types";
import type { Job, JobId, Scan, SecretStatus, Settings, View } from "@/app/types";
import { RunView } from "@/domains/run/RunView";
import { JOBS } from "@/domains/run/jobs";
import { canStartRun, planRun, runButtonLabel } from "@/domains/run/plan";
import { HistoryPanel } from "@/domains/history/HistoryPanel";
import { SettingsPanel } from "@/domains/settings/SettingsPanel";
import { ThreadView } from "@/domains/thread/ThreadView";
import { confirm, copyToClipboard, pickDirectory, pickFiles, pickFolders } from "@/platform/host";
import { useToast } from "./useToast";
import { useThread } from "./useThread";
import { useCloseConfirm, useDragDrop, useWindowFocusClass } from "./useHostWindow";
import "./App.css";

export default function App() {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [secrets, setSecrets] = useState<SecretStatus>({ datalab: false, revai: false });
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
  const { thread, openJobThread, openHistoryThread, closeThread, copyThread } = useThread({
    view,
    setView,
    showToast,
  });

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
  useCloseConfirm(running, activeCount);

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

  // What the selection actually costs. Three buckets, and the difference
  // matters to the user: files already sitting in the output folder do nothing
  // at all, files whose result exists elsewhere get copied for free, and the
  // rest go to the provider. The button only ever promises the third number.
  const { skipping, copying, toRun } = planRun(
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

  const copyText = async (j: Job | null) => {
    if (!j?.outputText) return;
    showToast((await copyToClipboard(j.outputText)) ? "Copied to clipboard" : "Copy failed");
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
  if (copying > 0 && toRun > 0) noteParts.push(`${copying} copied from an earlier run`);
  const note = noteParts.length > 0 ? noteParts.join(" · ") : null;

  let body: ReactNode;
  switch (view) {
    case "settings":
      body = (
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
      body = (
        <HistoryPanel
          refreshKey={runsFinished}
          onChanged={() => setRunsFinished((n) => n + 1)}
          onOpen={(e) => void openHistoryThread(e)}
          onToast={showToast}
          onClose={() => setView("run")}
        />
      );
      break;
    case "thread":
      body = thread ? (
        <ThreadView
          title={thread.title}
          subtitle={thread.subtitle}
          text={thread.text}
          onCopy={() => void copyThread()}
          onReveal={
            thread.revealPath
              ? () => {
                  const path = thread.revealPath;
                  if (path) void call(() => commands.revealPath(path));
                }
              : null
          }
          onClose={closeThread}
        />
      ) : null;
      break;
    case "run":
      body = (
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
          onPreview={openJobThread}
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
      body = _exhaustive;
    }
  }

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
