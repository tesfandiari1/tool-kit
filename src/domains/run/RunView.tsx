import {
  ArrowClockwiseIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  ClockCounterClockwiseIcon,
  CopyIcon,
  EyeIcon,
  FileTextIcon,
  FolderIcon,
  FolderOpenIcon,
  PlayIcon,
  StopIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import {
  Button,
  Label,
  Meter,
  Mono,
  Row,
  Segmented,
  Spacer,
  Stack,
  Status,
  Text,
} from "@ui";
import { ACTIVE } from "@/app/types";
import type { Job, JobId, Scan, Settings } from "@/app/types";
import { basename, fmtElapsed } from "@/app/format";
import { JOBS, type JobDef } from "./jobs";

/// A job's status mapped onto the design system's tones. The library knows
/// nothing about our five statuses, and this one line is the whole cost of
/// keeping it that way.
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
  note,
  finished,
  total,
  doneCount,
  failedCount,
  job,
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
  note: string | null;
  finished: number;
  total: number;
  doneCount: number;
  failedCount: number;
  job: JobDef;
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

  return (
    <main className="flow">
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
        label="Output folder"
        path={settings.outputDir}
        placeholder="Choose where results are saved"
        onPick={onPickOutput}
      />

      <Stack gap={2}>
        <Label>Job</Label>
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
        <Text size="xs" tone="faint">
          {job.desc}, via {job.service}
        </Text>
      </Stack>

      <Stack gap={2}>
        <Row gap={2} align="stretch">
          <Button
            variant="primary"
            size="lg"
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
              size="lg"
              onClick={onStop}
              title="Stop this run"
              icon={<StopIcon weight="fill" />}
            >
              Stop
            </Button>
          )}
        </Row>
        {running && (
          <Meter value={total ? finished / total : 0} label="Run progress" />
        )}
        {/* An app composite rather than a Button: this is a full sentence, and
            Button's label treatment (uppercase mono, nowrap) clips it. */}
        {!running && hint && (
          <button type="button" className="hint" onClick={onHint}>
            <WarningCircleIcon weight="fill" />
            {hint}
          </button>
        )}
        {/* Not a hint: nothing is wrong and there is nothing to click. */}
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
      </Stack>

      {jobs.length > 0 && (
        <section className="results">
          <Row gap={3} className="results-head">
            <Label>
              {running
                ? `${String(total)} files`
                : `${String(doneCount)} done${failedCount ? `, ${String(failedCount)} failed` : ""} of ${String(total)}`}
            </Label>
            <Spacer />
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
          </Row>
          {jobs.map((j) => (
            <JobRow
              key={j.id}
              job={j}
              now={now}
              onPreview={() => {
                onPreview(j);
              }}
              onCopy={() => {
                onCopy(j);
              }}
              onReveal={() => {
                onRevealJob(j);
              }}
              onRetry={() => {
                onRetryJob(j.id);
              }}
            />
          ))}
        </section>
      )}
    </main>
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
    <Stack gap={2}>
      <Label>{label}</Label>
      <button className={`folder ${path ? "set" : ""}`} onClick={onPick} title={path ?? placeholder}>
        {path ? <FolderOpenIcon weight="fill" /> : <FolderIcon />}
        <span className="folder-path">
          {path ? (path.split(/[\\/]/).filter(Boolean).pop() ?? path) : placeholder}
        </span>
        <span className="folder-action">{path ? "Change" : "Choose"}</span>
      </button>
    </Stack>
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
    <Stack gap={2}>
      <Label>Input{count > 0 ? ` · ${String(count)} file${count > 1 ? "s" : ""}` : ""}</Label>
      <div className={`inputs ${dragging ? "dragging" : ""}`}>
        {inputs.length === 0 ? (
          <div className="inputs-empty">
            Drop files or folders here
            <br />
            The job is matched to what you drop
          </div>
        ) : (
          inputs.map((p) => (
            <div className="input-item" key={p} title={p}>
              {isFile(p) ? <FileTextIcon /> : <FolderOpenIcon weight="fill" />}
              <Mono size="xs" truncate className="input-name">
                {basename(p)}
              </Mono>
              <button
                className="input-x"
                onClick={() => {
                  onRemove(p);
                }}
                aria-label="Remove"
              >
                <XIcon />
              </button>
            </div>
          ))
        )}
      </div>
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
  );
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
    <div className={`job ${job.status}`}>
      <Status tone={toneFor(job.status)} label={job.status}>
        {active ? (
          <CircleNotchIcon className="spin" />
        ) : job.status === "done" ? (
          <CheckCircleIcon weight="fill" />
        ) : (
          <WarningCircleIcon weight="fill" />
        )}
      </Status>
      <div className="job-body">
        <Mono size="xs" tone="ink" className="job-name" title={job.sourcePath}>
          {job.fileName}
        </Mono>
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
      </div>
      {(job.status === "working" || job.status === "processing") && job.startedAt !== null && (
        <Mono size="xs" className="job-time">
          {fmtElapsed(now, job.startedAt)}
        </Mono>
      )}
      <div className="job-actions">
        {job.status === "done" && (
          <>
            <Button
              variant="ghost"
              size="sm"
              iconOnly
              icon={<EyeIcon />}
              title="Preview"
              aria-label="Preview"
              onClick={onPreview}
            />
            <Button
              variant="ghost"
              size="sm"
              iconOnly
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
