import { useRef } from "react";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  ClockCounterClockwiseIcon,
  CopyIcon,
  FileTextIcon,
  FolderIcon,
  FolderOpenIcon,
  PlayIcon,
  StopIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import {
  Badge,
  Button,
  Meter,
  Mono,
  Panel,
  Row,
  Segmented,
  Spacer,
  Stack,
  StatusDot,
  Text,
  Well,
} from "@ui";
import { ACTIVE } from "@/app/types";
import type { Job, JobId, Scan, Settings } from "@/app/types";
import { basename, fmtElapsed } from "@/app/format";
import { FlowLayout } from "@/shell/FlowLayout";
import { jobDetailItems, jobDetailText } from "./details";
import { JOBS, type JobDef } from "./jobs";
import { runServiceDescription } from "./plan";

/// A job's status mapped onto the design system's tones. The library knows
/// nothing about our five statuses, and this one line is the whole cost of
/// keeping it that way. The queue specimen uses the dot, not a glyph: colour
/// is the signal, and the slot never changes size.
function toneFor(status: Job["status"]) {
  if (status === "done") return "pass" as const;
  if (status === "failed") return "fault" as const;
  // Queued is waiting for a permit, not being worked on. Amber here would
  // claim a provider is already spending money on it.
  if (status === "queued") return "queued" as const;
  return "live" as const;
}

export function RunView({
  settings,
  scan,
  jobs,
  dragging,
  now,
  running,
  canRun,
  runLabel,
  hint,
  hintActionable,
  note,
  finished,
  total,
  doneCount,
  failedCount,
  job,
  selectedId,
  expanded,
  persist,
  onAddFiles,
  onAddFolders,
  onPickOutput,
  onRemoveInput,
  onClearInputs,
  onRun,
  onStop,
  onRetryFailed,
  onRevealOutput,
  onPreview,
  onCopy,
  onRevealJob,
  onRetryJob,
  onHint,
}: {
  settings: Settings;
  scan: Scan;
  jobs: Job[];
  dragging: boolean;
  now: number;
  running: boolean;
  canRun: boolean;
  runLabel: string;
  hint: string | null;
  /// When false, the hint is status prose, not a control `onHint` answers.
  hintActionable: boolean;
  note: string | null;
  finished: number;
  total: number;
  doneCount: number;
  failedCount: number;
  job: JobDef;
  /// The open document's output path, so the queue can mark which row you are
  /// reading. Null whenever nothing is open.
  selectedId: string | null;
  /// A document is open, so the column is a workspace rather than a launcher.
  /// The controls above the queue compress and the queue takes the rest.
  expanded: boolean;
  persist: (patch: Partial<Settings>) => void;
  onAddFiles: () => void;
  onAddFolders: () => void;
  onPickOutput: () => void;
  onRemoveInput: (path: string) => void;
  onClearInputs: () => void;
  onRun: () => void;
  onStop: () => void;
  onRetryFailed: () => void;
  onRevealOutput: () => void;
  onPreview: (job: Job) => void;
  onCopy: (job: Job) => void;
  onRevealJob: (job: Job) => void;
  onRetryJob: (id: number) => void;
  onHint: () => void;
}) {
  const inputCount = settings.jobType === "transcribe" ? scan.transcribe : scan.convert;

  const jobPanel = (
    <JobPanel
      settings={settings}
      scan={scan}
      job={job}
      expanded={expanded}
      running={running}
      canRun={canRun}
      runLabel={runLabel}
      hint={hint}
      hintActionable={hintActionable}
      note={note}
      finished={finished}
      total={total}
      persist={persist}
      onRun={onRun}
      onStop={onStop}
      onHint={onHint}
    />
  );

  const queuePanel =
    jobs.length > 0 ? (
      <Panel
        className="queue"
        title="Queue"
        bare
        actions={
          <Row gap={2}>
            {!running && failedCount > 0 && (
              <Button variant="link" onClick={onRetryFailed}>
                Retry {failedCount} failed
              </Button>
            )}
            {!running && doneCount > 0 && settings.outputDir && (
              <Button variant="link" onClick={onRevealOutput}>
                Show in Finder
              </Button>
            )}
            <Badge square>{total}</Badge>
          </Row>
        }
      >
        <JobList
          jobs={jobs}
          now={now}
          selectedId={selectedId}
          onOpen={onPreview}
          onCopy={onCopy}
          onReveal={onRevealJob}
          onRetry={onRetryJob}
        />
      </Panel>
    ) : null;

  const inputs = (
    <>
      <InputPicker
        inputs={settings.inputs}
        count={inputCount}
        dragging={dragging}
        onAddFiles={onAddFiles}
        onAddFolders={onAddFolders}
        onRemove={onRemoveInput}
        onClear={onClearInputs}
      />
      <FolderField
        path={settings.outputDir}
        placeholder="Choose where results are saved"
        onPick={onPickOutput}
      />
    </>
  );

  /// Compact launcher: inputs and queue scroll; Job (with Run) stays pinned.
  /// Workspace: one column, queue grows, controls compress via `.flow--workspace`.
  if (!expanded) {
    return (
      <FlowLayout foot={jobPanel}>
        {inputs}
        {queuePanel}
      </FlowLayout>
    );
  }

  return (
    <FlowLayout variant="workspace">
      {inputs}
      {jobPanel}
      {queuePanel}
    </FlowLayout>
  );
}

function JobPanel({
  settings,
  scan,
  job,
  expanded,
  running,
  canRun,
  runLabel,
  hint,
  hintActionable,
  note,
  finished,
  total,
  persist,
  onRun,
  onStop,
  onHint,
}: {
  settings: Settings;
  scan: Scan;
  job: JobDef;
  expanded: boolean;
  running: boolean;
  canRun: boolean;
  runLabel: string;
  hint: string | null;
  /// When false, the hint is status prose, not a control `onHint` answers.
  hintActionable: boolean;
  note: string | null;
  finished: number;
  total: number;
  persist: (patch: Partial<Settings>) => void;
  onRun: () => void;
  onStop: () => void;
  onHint: () => void;
}) {
  return (
    <Panel title="Job">
      <Stack gap={3}>
        <Segmented
          label="Job"
          value={settings.jobType}
          onChange={(jobType: JobId) => {
            persist({ jobType });
          }}
          options={JOBS.map((j) => {
            const JIcon = j.icon;
            return {
              value: j.id,
              label: j.label,
              icon: (selected: boolean) => <JIcon weight={selected ? "fill" : "regular"} />,
              count: j.id === "transcribe" ? scan.transcribe : scan.convert,
            };
          })}
        />
        <Text size="xs" tone="faint" className="run-desc">
          {runServiceDescription({
            description: job.desc,
            provider: job.service,
            jobType: settings.jobType,
            conversionRoute: settings.conversionRoute,
            profile: settings.conversionProfile,
          })}
        </Text>
        <Row gap={2} align="stretch">
          {/* `lg` is the launcher's single actuator. With a document open the
              queue is what the column is for, so Run steps down a size rather
              than staying the loudest thing on screen. */}
          <Button
            variant="primary"
            size={expanded ? "md" : "lg"}
            block
            busy={running}
            disabled={!canRun}
            onClick={onRun}
            icon={running ? <CircleNotchIcon className="spin" weight="bold" /> : <PlayIcon weight="fill" />}
          >
            {running ? `Working… ${String(finished)} of ${String(total)}` : runLabel}
          </Button>
          {running && (
            <Button
              variant="quiet"
              size={expanded ? "md" : "lg"}
              onClick={onStop}
              title="Stop this run"
              icon={<StopIcon weight="fill" />}
            >
              Stop
            </Button>
          )}
        </Row>
        <div className="job-msg">
          {running && <Meter value={total ? finished / total : 0} label="Run progress" />}
          {!running && hint && hintActionable && (
            <button type="button" className="hint" onClick={onHint}>
              <WarningCircleIcon weight="fill" />
              {hint}
            </button>
          )}
          {!running && hint && !hintActionable && (
            <Text size="xs" tone="faint">
              {hint}
            </Text>
          )}
          {!running && !hint && note && (
            <Row gap={2}>
              <Text as="span" size="xs" tone="ghost">
                <ClockCounterClockwiseIcon />
              </Text>
              <Text as="span" size="xs" tone="faint">
                {note}
              </Text>
            </Row>
          )}
        </div>
      </Stack>
    </Panel>
  );
}

function FolderField({
  path,
  placeholder,
  onPick,
}: {
  path: string | null;
  placeholder: string;
  onPick: () => void;
}) {
  return (
    <Panel
      title="Output"
      actions={
        <Button variant="ghost" size="sm" onClick={onPick}>
          {path ? "Change" : "Choose"}
        </Button>
      }
    >
      <button type="button" className="folder-hit" onClick={onPick} title={path ?? placeholder}>
        <Row gap={3}>
          {path ? <FolderOpenIcon weight="fill" /> : <FolderIcon />}
          <Mono truncate>{path ? (path.split(/[\\/]/).filter(Boolean).pop() ?? path) : placeholder}</Mono>
        </Row>
      </button>
    </Panel>
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
  const isFile = (p: string) => /\.[^/\\]+$/.test(p);
  return (
    <Panel
      className="drop"
      title="Input"
      actions={count > 0 ? <Badge square>{count}</Badge> : undefined}
    >
      <Stack gap={3}>
        <Well className={dragging ? "is-dropping" : undefined} selectable={false}>
          {inputs.length === 0 ? (
            <div className="drop-empty">
              <Text size="sm" tone="faint">
                Drop files or folders here
              </Text>
              <Text size="xs" tone="ghost">
                The job is matched to what you drop
              </Text>
            </div>
          ) : (
            <Stack gap={1}>
              {inputs.map((p) => (
                <Row key={p} gap={3} className="drop-item" title={p}>
                  {isFile(p) ? <FileTextIcon /> : <FolderOpenIcon weight="fill" />}
                  <Mono size="xs" truncate>
                    {basename(p)}
                  </Mono>
                  <Spacer />
                  <Button
                    variant="ghost"
                    size="sm"
                    iconOnly
                    icon={<XIcon />}
                    title="Remove"
                    aria-label="Remove"
                    onClick={() => {
                      onRemove(p);
                    }}
                  />
                </Row>
              ))}
            </Stack>
          )}
        </Well>
        <Row gap={2}>
          <Button size="sm" icon={<FileTextIcon />} onClick={onAddFiles}>
            Files
          </Button>
          <Button size="sm" icon={<FolderOpenIcon />} onClick={onAddFolders}>
            Folder
          </Button>
          {inputs.length > 0 && (
            <>
              <Spacer />
              <Button size="sm" variant="ghost" onClick={onClear}>
                Clear
              </Button>
            </>
          )}
        </Row>
      </Stack>
    </Panel>
  );
}

/// The queue, once a document is open, is how you move between results — so it
/// is a real listbox and not a log you happen to be able to click.
///
/// Arrows move focus and Enter opens, rather than selection following focus the
/// way `Tabs` does. That rule bends exactly where a panel costs something:
/// switching tab is free, but opening a result reads a file off disk and
/// resizes the window, so walking past four rows must not do it four times.
function JobList({
  jobs,
  now,
  selectedId,
  onOpen,
  onCopy,
  onReveal,
  onRetry,
}: {
  jobs: Job[];
  now: number;
  selectedId: string | null;
  onOpen: (job: Job) => void;
  onCopy: (job: Job) => void;
  onReveal: (job: Job) => void;
  onRetry: (id: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);

  const options = () =>
    Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("[role='option']") ?? []);

  const focusAt = (i: number) => {
    const opts = options();
    if (opts.length === 0) return;
    opts[Math.min(Math.max(i, 0), opts.length - 1)].focus();
  };

  /// Which row holds focus, read from the row rather than the focused element:
  /// a click lands on an action button, and the arrows still have to work from
  /// there.
  const current = () => {
    const row = document.activeElement?.closest(".job");
    if (!row) return -1;
    return options().findIndex((o) => row.contains(o));
  };

  const selectedIndex =
    selectedId === null ? -1 : jobs.findIndex((j) => j.outputPath === selectedId);

  return (
    <div
      ref={ref}
      className="job-list"
      role="listbox"
      aria-label="Results"
      /// A list, not a ring: the arrows stop at the ends and Home/End jump to
      /// them, so holding a key cannot cycle you past the row you were reading.
      onKeyDown={(e) => {
        const i = current();
        if (i < 0) return;
        if (e.key === "ArrowDown") {
          e.preventDefault();
          focusAt(i + 1);
        } else if (e.key === "ArrowUp") {
          e.preventDefault();
          focusAt(i - 1);
        } else if (e.key === "Home") {
          e.preventDefault();
          focusAt(0);
        } else if (e.key === "End") {
          e.preventDefault();
          focusAt(jobs.length - 1);
        }
      }}
    >
      {jobs.map((j, i) => (
        <JobRow
          key={j.id}
          job={j}
          now={now}
          selected={i === selectedIndex}
          /// Roving tabindex: the queue is one stop in the tab order and the
          /// arrows move inside it. The row you are reading is the way back in,
          /// or the first row when nothing is open.
          stop={i === (selectedIndex === -1 ? 0 : selectedIndex)}
          onOpen={() => {
            onOpen(j);
          }}
          onCopy={() => {
            onCopy(j);
          }}
          onReveal={() => {
            onReveal(j);
          }}
          onRetry={() => {
            onRetry(j.id);
          }}
        />
      ))}
    </div>
  );
}

function JobRow({
  job,
  now,
  selected,
  stop,
  onOpen,
  onCopy,
  onReveal,
  onRetry,
}: {
  job: Job;
  now: number;
  selected: boolean;
  stop: boolean;
  onOpen: () => void;
  onCopy: () => void;
  onReveal: () => void;
  onRetry: () => void;
}) {
  const active = ACTIVE.includes(job.status);
  const done = job.status === "done";
  const time =
    (job.status === "working" || job.status === "processing") && job.startedAt !== null
      ? fmtElapsed(now, job.startedAt)
      : "0:00";
  const details = jobDetailItems(job);
  const detailText = jobDetailText(details);
  const detailTone = details.some(({ kind }) => kind === "failure")
    ? "fault"
    : details.some(({ kind }) => kind === "warning")
      ? "muted"
      : "ghost";
  return (
    /* Not a <button>: the row already holds three of them, and nesting is
       invalid. The name is the primary control and the actions are its
       siblings, the same shape `Tabs` uses for a tab and its close control.
       The rest of the row forwards a click to that control so the whole row
       stays the target it looks like. */
    <div
      className={`job ${job.status}${selected ? " is-selected" : ""}`}
      onClick={(e) => {
        if (!done) return;
        if (e.target instanceof Element && e.target.closest("button")) return;
        onOpen();
      }}
    >
      <StatusDot tone={toneFor(job.status)} label={job.status} />
      <div className="job-body">
        <button
          type="button"
          role="option"
          aria-selected={selected}
          /// A row still queued or in flight has nothing to open yet. It stays
          /// in the listbox so the arrows walk the whole queue, and says so
          /// rather than answering a click with silence.
          aria-disabled={!done}
          tabIndex={stop ? 0 : -1}
          className="job-open"
          title={job.sourcePath}
          onClick={done ? onOpen : undefined}
        >
          <Mono size="xs" className="job-name">
            {job.fileName}
          </Mono>
        </button>
        {job.status === "failed" && job.error ? (
          <Text as="span" size="xs" tone="fault" className="job-sub">
            {job.error}
          </Text>
        ) : active ? (
          <Text as="span" size="xs" tone="faint" className="job-sub">
            {job.progressNote}
          </Text>
        ) : (
          job.outputPath && (
            <Text as="span" size="xs" tone="faint" className="job-sub" title={job.outputPath}>
              {basename(job.outputPath)}
            </Text>
          )
        )}
        {details.length > 0 && (
          <Text as="span" size="xs" tone={detailTone} className="job-sub" title={detailText}>
            {detailText}
          </Text>
        )}
      </div>
      <Mono size="xs" tone={active ? "ink" : "ghost"} className="job-time">
        {time}
      </Mono>
      {/* Out of the tab order on purpose, the way a tab's close control is:
          the queue is one stop, and every action here has a keyboard route
          elsewhere — the open document's own header copies and reveals it, and
          the results head retries the failures. */}
      <div className="job-actions">
        {done && (
          <>
            <Button
              variant="ghost"
              size="sm"
              iconOnly
              tabIndex={-1}
              icon={<CopyIcon />}
              title="Copy"
              aria-label="Copy"
              onClick={onCopy}
            />
            {job.outputPath && (
              <Button
                variant="ghost"
                size="sm"
                iconOnly
                tabIndex={-1}
                icon={<FolderOpenIcon />}
                title="Show in Finder"
                aria-label="Show in Finder"
                onClick={onReveal}
              />
            )}
          </>
        )}
        {job.status === "failed" && (
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            tabIndex={-1}
            icon={<ArrowClockwiseIcon />}
            title="Retry"
            aria-label="Retry"
            onClick={onRetry}
          />
        )}
      </div>
    </div>
  );
}
