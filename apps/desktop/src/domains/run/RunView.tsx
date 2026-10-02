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
  Meta,
  Panel,
  Row,
  Segmented,
  Select,
  Spacer,
  Stack,
  StatusDot,
  Text,
  Well,
} from "@ui";
import { ACTIVE } from "@/app/types";
import type { InputNode, Job, JobId, ProjectSummary, Scan, Settings } from "@/app/types";
import { basename, fmtElapsed } from "@/app/format";
import { FlowLayout } from "@/shell/FlowLayout";
import { jobDetailItems, jobDetailText } from "./details";
import { JOBS, type JobDef } from "./jobs";
import { runServiceDescription } from "./plan";

/// Five job statuses onto three library tones. `@ui` knows neither.
function toneFor(status: Job["status"]) {
  if (status === "done") return "pass" as const;
  if (status === "failed") return "fault" as const;
  // Queued waits for a permit: the live colour would claim a provider is spending.
  if (status === "queued") return "queued" as const;
  return "live" as const;
}

export function RunView({
  settings,
  scan,
  jobs,
  runJobs,
  projects,
  dragging,
  now,
  running,
  canRun,
  runLabel,
  hint,
  hintActionable,
  note,
  job,
  selectedId,
  persist,
  onAddFiles,
  onAddFolders,
  onPickOutput,
  onPickProject,
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
  /// This run's rows: the queue keeps every row of the session.
  runJobs: Job[];
  /// Empty only before onboarding binds a workspace.
  projects: ProjectSummary[];
  dragging: boolean;
  now: number;
  running: boolean;
  canRun: boolean;
  runLabel: string;
  hint: string | null;
  /// When false, the hint is status prose, not a control `onHint` answers.
  hintActionable: boolean;
  note: string | null;
  job: JobDef;
  /// The open document's output path, so the queue can mark its row.
  selectedId: string | null;
  persist: (patch: Partial<Settings>) => void;
  onAddFiles: () => void;
  onAddFolders: () => void;
  onPickOutput: () => void;
  onPickProject: (rel: string) => void;
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
  const total = jobs.length;
  const doneCount = jobs.filter((j) => j.status === "done").length;
  const failedCount = jobs.filter((j) => j.status === "failed").length;
  // Progress is this run's. Retry and Finder act on the whole list.
  const runTotal = runJobs.length;
  const finished = runJobs.filter(
    (j) => j.status === "done" || j.status === "failed",
  ).length;

  const jobPanel = (
    /* Outside the Panel: two lines held open inside a border read as a
       failure. */
    <>
      <Panel title="Job">
        <Stack gap={3}>
          <Stack gap={2}>
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
            {/* Closer to the control above it than that is to Run (rule 4). */}
            <Text size="xs" tone="faint" className="run-desc">
              {runServiceDescription({
                description: job.desc,
                provider: job.service,
                conversionRoute: settings.conversionRoute,
                profile: settings.conversionProfile,
              })}
            </Text>
          </Stack>
          <Row gap={2} align="stretch" className="job-run">
            {/* The column's single actuator, at one size. */}
            <Button
              variant="primary"
              size="lg"
              block
              busy={running}
              disabled={!canRun}
              onClick={onRun}
              icon={running ? <CircleNotchIcon className="spin" weight="bold" /> : <PlayIcon weight="fill" />}
            >
              {running ? `Working… ${String(finished)} of ${String(runTotal)}` : runLabel}
            </Button>
            {running && (
              <Button
                variant="secondary"
                size="lg"
                onClick={onStop}
                title="Stop this run"
                icon={<StopIcon weight="fill" />}
              >
                Stop
              </Button>
            )}
          </Row>
        </Stack>
      </Panel>
      <div className="job-msg">
        {running && <Meter value={runTotal ? finished / runTotal : 0} label="Run progress" />}
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
    </>
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
              <Button variant="ghost" onClick={onRetryFailed}>
                Retry {failedCount} failed
              </Button>
            )}
            {!running && doneCount > 0 && (
              <Button variant="ghost" onClick={onRevealOutput}>
                Show in Finder
              </Button>
            )}
            <Badge count>{total}</Badge>
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
        nodes={scan.nodes}
        jobType={settings.jobType}
        count={inputCount}
        dragging={dragging}
        onAddFiles={onAddFiles}
        onAddFolders={onAddFolders}
        onRemove={onRemoveInput}
        onClear={onClearInputs}
      />
      {projects.length > 0 ? (
        <ProjectField
          projects={projects}
          value={settings.activeProjectPath}
          onPick={onPickProject}
        />
      ) : (
        <FolderField
          path={settings.outputDir}
          placeholder="Choose where results are saved"
          onPick={onPickOutput}
        />
      )}
    </>
  );

  /// Inputs and queue scroll, and the Job panel stays pinned. One layout,
  /// because Run owns the end pane whole whenever it is on screen.
  return (
    <FlowLayout foot={jobPanel}>
      {inputs}
      {queuePanel}
    </FlowLayout>
  );
}


/// A project, not a folder: the run writes beside the imported copy, which is
/// what pairs a source and its result on one tree row.
function ProjectField({
  projects,
  value,
  onPick,
}: {
  projects: ProjectSummary[];
  value: string | null;
  onPick: (rel: string) => void;
}) {
  return (
    <Panel title="Save to">
      <Select
        label="Project"
        hint="Dropped files are filed here, and results land beside them."
        value={value ?? ""}
        onChange={(e) => {
          onPick(e.target.value);
        }}
        options={projects.map((p) => ({ value: p.path, label: p.title }))}
      />
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
        <Row gap={2}>
          {path ? <FolderOpenIcon weight="fill" /> : <FolderIcon />}
          <Meta truncate>{path ? basename(path) : placeholder}</Meta>
        </Row>
      </button>
    </Panel>
  );
}

/// The drop well: what a run would take out of the selection, which a dropped
/// folder's name cannot answer.
///
/// Rows are built from `inputs` and only enriched by `scan.nodes`, which empties
/// on every unrelated refresh. The nesting lights up, it does not appear
/// (UI.md rule 2).
function InputPicker({
  inputs,
  nodes,
  jobType,
  count,
  dragging,
  onAddFiles,
  onAddFolders,
  onRemove,
  onClear,
}: {
  inputs: string[];
  nodes: InputNode[];
  jobType: JobId;
  count: number;
  dragging: boolean;
  onAddFiles: () => void;
  onAddFolders: () => void;
  onRemove: (p: string) => void;
  onClear: () => void;
}) {
  const described = new Map(nodes.map((node) => [node.path, node]));
  // Finder's guess until the host answers.
  const looksLikeFile = (p: string) => /\.[^/\\]+$/.test(p);

  return (
    <Panel
      className="drop"
      title="Input"
      actions={
        <Row gap={2}>
          {inputs.length > 0 && (
            <>
              <Button variant="ghost" onClick={onClear}>
                Clear
              </Button>
              {/* Holds its slot once the panel has anything in it, so a scan
                  landing does not resize the header (UI.md rule 2). */}
              <Badge count>{count > 0 ? count : ""}</Badge>
            </>
          )}
        </Row>
      }
    >
      <Stack gap={3}>
        {/* The well is the picker. A click on a row belongs to that row. */}
        <Well
          className={`drop-well${dragging ? " is-dropping" : ""}`}
          selectable={false}
          onClick={(e) => {
            if (e.target instanceof Element && e.target.closest("button, .drop-node")) return;
            onAddFiles();
          }}
        >
          {inputs.length === 0 ? (
            <button type="button" className="drop-empty" onClick={onAddFiles}>
              <FolderOpenIcon weight="light" aria-hidden />
              <Text size="sm" tone="default">
                Choose files or folders
              </Text>
              <Text size="xs" tone="ghost" className="drop-hint">
                Or drop them here. The job is matched to what you drop.
              </Text>
            </button>
          ) : (
            <ul className="drop-list">
              {inputs.map((path) => (
                <InputRow
                  key={path}
                  path={path}
                  node={described.get(path)}
                  fallbackIsDir={!looksLikeFile(path)}
                  jobType={jobType}
                  onRemove={onRemove}
                />
              ))}
            </ul>
          )}
        </Well>
        {/* Wraps, or the second button pushes past the panel. */}
        <Row gap={2} wrap>
          <Button size="sm" icon={<FileTextIcon />} onClick={onAddFiles}>
            {inputs.length > 0 ? "Add files" : "Files"}
          </Button>
          <Button size="sm" icon={<FolderOpenIcon />} onClick={onAddFolders}>
            Folder
          </Button>
        </Row>
      </Stack>
    </Panel>
  );
}

/// One dropped path and what a run would take from it. Matches list flat: the
/// well answers "what gets parsed", which a tree puts three clicks away.
function InputRow({
  path,
  node,
  fallbackIsDir,
  jobType,
  onRemove,
}: {
  path: string;
  node: InputNode | undefined;
  fallbackIsDir: boolean;
  jobType: JobId;
  onRemove: (p: string) => void;
}) {
  const isDir = node?.isDir ?? fallbackIsDir;
  const matches = node?.matches ?? [];
  const taken = matches.filter((m) => m.job === jobType).length;
  // Still shown: a vanished row reads as the drop having missed it.
  const inactive = node !== undefined && !isDir && node.job !== jobType;

  return (
    <li className="drop-node">
      <div className="drop-item" title={path}>
        {isDir ? <FolderOpenIcon weight="fill" /> : <FileTextIcon />}
        <Meta size="xs" truncate tone={inactive ? "ghost" : "ink"}>
          {node?.name ?? basename(path)}
        </Meta>
        <Spacer />
        {/* Holds its slot whether or not it reads, so the well does not
            reflow when a scan lands (UI.md rule 2). */}
        <Meta size="xs" tone={taken > 0 ? "default" : "ghost"} className="drop-count">
          {isDir && node !== undefined ? String(taken) : ""}
        </Meta>
        <Button
          variant="ghost"
          size="sm"
          iconOnly
          icon={<XIcon />}
          title="Remove"
          aria-label={`Remove ${node?.name ?? basename(path)}`}
          onClick={() => {
            onRemove(path);
          }}
        />
      </div>
      {matches.length > 0 && (
        <ul className="drop-matches">
          {matches.map((match) => (
            <li key={match.path} className="drop-match" title={match.path}>
              <Meta size="xs" truncate tone={match.job === jobType ? "default" : "ghost"}>
                {match.name}
              </Meta>
            </li>
          ))}
          {node !== undefined && node.truncated > 0 && (
            <li className="drop-match">
              <Text size="xs" tone="ghost">
                and {node.truncated} more
              </Text>
            </li>
          )}
        </ul>
      )}
    </li>
  );
}

/// A real listbox, not a log you can click. Arrows move focus and Enter opens,
/// unlike `Tabs`: opening a result reads a file off disk, so walking past four
/// rows must not do it four times.
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

  /// Read from the row, not the focused element: a click lands on a button.
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
      /// A list, not a ring: holding a key cannot cycle past the open row.
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
          /// Roving tabindex: the queue is one stop and the arrows move
          /// inside it.
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
    /* Not a <button>: the row holds three already and nesting is invalid. The
       rest of the row forwards its click to the name. */
    <div
      className={`job${selected ? " is-selected" : ""}`}
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
          /// Nothing to open yet. It stays in the listbox so the arrows walk
          /// the whole queue, and says so rather than fall silent.
          aria-disabled={!done}
          tabIndex={stop ? 0 : -1}
          className="job-open"
          title={job.sourcePath}
          onClick={done ? onOpen : undefined}
        >
          <Meta size="xs" className="job-name">
            {job.fileName}
          </Meta>
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
      <Meta size="xs" tone={active ? "ink" : "ghost"} className="job-time">
        {time}
      </Meta>
      {/* Out of the tab order: every action here has a keyboard route in the
          document header or the results head. */}
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
