import type { ReactNode } from "react";
import { FolderIcon, TrayIcon } from "@phosphor-icons/react";
import { Button, Mono, StatusDot, Tree, TreeRow } from "@ui";
import { ACTIVE, latestJobFor, type FileRow, type Job, type ProjectSummary } from "@/app/types";
import type { ProjectTreeState } from "@/shell/useProjectTree";
import { fileGlyph } from "./fileGlyph";

const INBOX = "Inbox";

/// Separates a quiet row's key from the folder it reports on. A file name
/// cannot hold a NUL, so this can never collide with a real entry.
const QUIET = "\u0000";

function pinned(projects: ProjectSummary[]): ProjectSummary[] {
  return [...projects].sort((a, b) => {
    if (a.title === b.title) return 0;
    if (a.title === INBOX) return -1;
    if (b.title === INBOX) return 1;
    return 0;
  });
}

/// The one file that knows both `FileRow` and `TreeRow`.
///
/// It owns the glyph, the paired-result marker, the project count and the
/// job-status mapping. The primitive holds the keyboard and the shape and
/// nothing else, and the host holds every listing rule, so this is the whole
/// translation layer between them.
export function ProjectTree({
  projects,
  tree,
  jobs,
  onSelect,
  onActivate,
  onInspect,
  onConvert,
}: {
  projects: ProjectSummary[];
  tree: ProjectTreeState;
  /// Live rows, matched onto files by `sourcePath`. Without them a failed
  /// conversion is invisible here and the only remedy the user can see is
  /// pressing Convert again, which bills again.
  jobs: Job[];
  /// The selection moved, by arrow, by type-ahead or by click. Null on a
  /// project root, which is a branch rather than a file.
  onSelect: (row: FileRow | null) => void;
  onActivate: (row: FileRow) => void;
  onInspect: (row: FileRow) => void;
  onConvert: (row: FileRow) => void;
}) {
  /// Every row on screen, by workspace-relative path. Filled while the rows
  /// below are built, which is before any handler can fire.
  const rows = new Map<string, FileRow>();

  const quietRow = (key: string, depth: number, text: string) => (
    <TreeRow key={key} path={key} depth={depth} quiet>
      {text}
    </TreeRow>
  );

  /// The rows one level down, or undefined while they are still being read.
  /// An expanded folder with an empty `ul[role=group]` is the one shape the
  /// tree pattern has no answer for, so an empty folder says so instead.
  const childrenOf = (rel: string, depth: number): ReactNode => {
    const listing = tree.listings[rel];
    if (listing === undefined) return undefined;
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

    return (
      <TreeRow
        key={row.rel}
        path={row.rel}
        depth={depth}
        open={open}
        busy={tree.busy.has(row.rel)}
        selected={tree.selected === row.rel}
        /* The only colour a tree row carries. A conversion in flight and one
           that failed are both facts about this row that nothing else on
           screen reports while the Run column is not showing. */
        icon={
          job !== null && ACTIVE.includes(job.status) ? (
            <StatusDot tone="live" label={job.status} />
          ) : job?.status === "failed" ? (
            <StatusDot tone="fault" label="failed" />
          ) : (
            <Glyph />
          )
        }
        /* Paired: the result's name, rather than a green mark. Green means a
           job passed, and a sibling result is a static fact that may predate
           every run in this session.

           Unpaired and convertible: the Convert control, holding its width at
           rest so the row is the same size hovered, focused and converting.
           Out of the tab order, the way a tab's close control is: the keyboard
           route is Enter on the row, which raises the card. */
        end={
          row.resultName !== null ? (
            <Mono size="xs" className="lib-tree__result" truncate>
              {row.resultName}
            </Mono>
          ) : row.job !== null ? (
            <span className="lib-tree__convert">
              <Button
                variant="link"
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
    return (
      <TreeRow
        key={project.id}
        path={project.path}
        depth={0}
        open={open}
        busy={tree.busy.has(project.path)}
        selected={tree.selected === project.path}
        icon={project.title === INBOX ? <TrayIcon weight={open ? "fill" : "regular"} /> : <FolderIcon />}
        title={project.path}
        /* Reserved whether or not it reads, the same two-character slot the
           segmented control's count keeps (UI.md rule 2). */
        end={
          <Mono
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
          </Mono>
        }
        group={open ? childrenOf(project.path, 1) : undefined}
      >
        {project.title}
      </TreeRow>
    );
  };

  const nodes = pinned(projects).map(renderProject);

  /// A click or Enter opens a folder and hands a file to the caller. Arrowing
  /// past a folder must not open forty tabs, which is why selection and
  /// activation are separate keys in the primitive.
  const activate = (rel: string) => {
    const row = rows.get(rel);
    // A project root has no `FileRow`: it is the branch itself.
    if (row === undefined || row.isDir) {
      tree.toggle(rel, !tree.expanded.has(rel), false);
      return;
    }
    onActivate(row);
  };

  return (
    <Tree
      label="Library"
      className="lib-tree"
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
