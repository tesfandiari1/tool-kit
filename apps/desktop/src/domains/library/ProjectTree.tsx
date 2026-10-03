import type { ReactNode } from "react";
import { FolderIcon, TrayIcon } from "@phosphor-icons/react";
import { Button, Meta, StatusDot, Tree, TreeRow } from "@ui";
import { ACTIVE, latestJobFor, type FileRow, type Job, type ProjectSummary } from "@/app/types";
import type { ProjectTreeState } from "@/shell/useProjectTree";
import { fileGlyph } from "./fileGlyph";

/// A file name cannot hold a NUL, so a quiet row's key cannot collide.
const QUIET = "\u0000";

/// The text after the last dot, uppercased.
function resultKindOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot <= 0 ? "" : name.slice(dot + 1).toUpperCase();
}

/// The catch-all sits at the top, whatever its created date says.
function pinned(projects: ProjectSummary[], catchAll: string | null): ProjectSummary[] {
  return [...projects].sort((a, b) => Number(b.path === catchAll) - Number(a.path === catchAll));
}

/// The one file that knows both `FileRow` and `TreeRow`. The primitive holds
/// the keyboard and the shape, the host holds every listing rule, and this
/// translates between them.
export function ProjectTree({
  projects,
  catchAllPath,
  activeProjectPath,
  tree,
  jobs,
  onSelect,
  onActivate,
  onInspect,
  onConvert,
  onSetActiveProject,
}: {
  projects: ProjectSummary[];
  /// Pinned to the top, with the tray glyph. Null until the host answers.
  catchAllPath: string | null;
  /// Where a drop is filed and a run writes. The tree says which, rather than
  /// leave it to a Select two views away.
  activeProjectPath: string | null;
  tree: ProjectTreeState;
  /// Matched onto files by `sourcePath`. Without them a failed conversion is
  /// invisible, and the only visible remedy runs it again.
  jobs: Job[];
  /// Null on a project root, which is a branch rather than a file.
  onSelect: (row: FileRow | null) => void;
  onActivate: (row: FileRow) => void;
  onInspect: (row: FileRow) => void;
  onConvert: (row: FileRow) => void;
  onSetActiveProject: (rel: string) => void;
}) {
  const rows = new Map<string, FileRow>();

  const quietRow = (key: string, depth: number, text: string) => (
    <TreeRow key={key} path={key} depth={depth} quiet>
      {text}
    </TreeRow>
  );

  /// Undefined while the level is being read. An empty `ul[role=group]` is
  /// the one shape the tree pattern has no answer for.
  const childrenOf = (rel: string, depth: number): ReactNode => {
    const listing = tree.listings[rel];
    if (listing === undefined) {
      return tree.failed.has(rel)
        ? quietRow(`${rel}${QUIET}failed`, depth, "Could not read this folder")
        : undefined;
    }
    if (listing.entries.length === 0) return quietRow(`${rel}${QUIET}empty`, depth, "Empty");
    return (
      <>
        {listing.entries.map((entry) => renderRow(entry, depth))}
        {listing.truncated > 0 &&
          quietRow(`${rel}${QUIET}more`, depth, `${String(listing.truncated)} more files`)}
      </>
    );
  };

  const renderRow = (row: FileRow, depth: number): ReactNode => {
    rows.set(row.rel, row);
    const open = row.isDir ? tree.expanded.has(row.rel) : undefined;
    const job = latestJobFor(jobs, row.path);
    const Glyph = fileGlyph(row.ext, row.isDir);
    /// Empty holds the slot rather than fall through to Convert, which would
    /// offer to convert a file that already converted.
    const kind = row.resultName === null ? "" : resultKindOf(row.resultName);

    return (
      <TreeRow
        key={row.rel}
        path={row.rel}
        depth={depth}
        open={open}
        busy={tree.busy.has(row.rel)}
        selected={tree.selected === row.rel}
        /* The only colour a row carries: nothing else reports a conversion
           while Run is not up. */
        icon={
          job !== null && ACTIVE.includes(job.status) ? (
            <StatusDot tone="live" label={job.status} />
          ) : job?.status === "failed" ? (
            <StatusDot tone="fault" label="failed" />
          ) : (
            <Glyph />
          )
        }
        /* Paired: the result's kind, never green, which would claim a job
           passed. The kind and not the name, because the name is the source's
           own stem and a truncated copy ate the row's own name.

           Unpaired and convertible: the Convert control, holding its width at
           rest. Out of the tab order, because Enter on the row does it. */
        end={
          row.resultName !== null ? (
            kind === "" ? undefined : (
              <Meta size="xs" tone="ghost" className="lib-tree__result">
                {kind}
              </Meta>
            )
          ) : row.job !== null ? (
            <span className="lib-tree__convert">
              <Button
                variant="ghost"
                size="sm"
                tabIndex={-1}
                disabled={job !== null && ACTIVE.includes(job.status)}
                onClick={() => {
                  onConvert(row);
                }}
              >
                Convert
              </Button>
            </span>
          ) : undefined
        }
        title={
          row.resultName === null ? row.name : `${row.name} converted to ${row.resultName}`
        }
        group={open === true ? childrenOf(row.rel, depth + 1) : undefined}
      >
        {row.name}
      </TreeRow>
    );
  };

  const renderProject = (project: ProjectSummary): ReactNode => {
    const open = tree.expanded.has(project.path);
    const pending = tree.listings[project.path]?.pending ?? 0;
    const active = project.path === activeProjectPath;
    return (
      <TreeRow
        key={project.id}
        path={project.path}
        depth={0}
        open={open}
        busy={tree.busy.has(project.path)}
        selected={tree.selected === project.path}
        icon={
          project.path === catchAllPath ? (
            <TrayIcon weight={open ? "fill" : "regular"} />
          ) : (
            <FolderIcon />
          )
        }
        title={
          active
            ? `${project.path} — new work is saved here`
            : project.path
        }
        /* The count holds its place whether or not it reads (UI.md rule 2).
           Beside it, a static word on the active project and a hover control
           on every other. */
        end={
          /* `.ui-tree__end` is already a flex row at --s2. */
          <>
            <Meta
              size="xs"
              tone="ghost"
              className="lib-tree__count"
              title={
                pending > 0
                  ? `${String(pending)} file${pending === 1 ? "" : "s"} in this folder still to convert. Folders inside are not counted.`
                  : undefined
              }
            >
              {pending > 0 ? String(pending) : ""}
            </Meta>
            {active ? (
              <Meta size="xs" tone="ghost" className="lib-tree__saves">
                saves here
              </Meta>
            ) : (
              <span className="lib-tree__convert">
                <Button
                  variant="ghost"
                  size="sm"
                  tabIndex={-1}
                  onClick={() => {
                    onSetActiveProject(project.path);
                  }}
                >
                  Save here
                </Button>
              </span>
            )}
          </>
        }
        group={open ? childrenOf(project.path, 1) : undefined}
      >
        {project.title}
      </TreeRow>
    );
  };

  // `setup_workspace` always mints a catch-all, so an empty list means the
  // host could not read the workspace.
  const nodes =
    projects.length === 0
      ? [quietRow(`${QUIET}noprojects`, 0, "No projects found in this folder")]
      : pinned(projects, catchAllPath).map(renderProject);

  /// Selection and activation are separate keys, so arrowing past a folder
  /// cannot open forty tabs.
  const activate = (rel: string) => {
    const row = rows.get(rel);
    if (row === undefined || row.isDir) {
      tree.toggle(rel, !tree.expanded.has(rel), false);
      return;
    }
    onActivate(row);
  };

  return (
    <Tree
      label="Library"
      onSelect={(rel) => {
        tree.select(rel);
        onSelect(rows.get(rel) ?? null);
      }}
      onActivate={activate}
      onInspect={(rel) => {
        const row = rows.get(rel);
        if (row !== undefined) onInspect(row);
      }}
      onToggle={tree.toggle}
    >
      {nodes}
    </Tree>
  );
}
