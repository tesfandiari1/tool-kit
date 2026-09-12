import { useState } from "react";
import { FolderOpenIcon, GearSixIcon, PlusIcon } from "@phosphor-icons/react";
import { Button, Input, Label, Meta, Path, Row, Select, Spacer, Stack } from "@ui";
import type { FileRow, Job, ProjectSummary } from "@/app/types";
import { basename, tildePath } from "@/app/format";
import type { ProjectTreeState } from "@/shell/useProjectTree";
import { ProjectTree } from "./ProjectTree";

/// The workspace, its projects as a tree, and the two ways out.
export function LibraryPane({
  workspacePath,
  projects,
  catchAllPath,
  activeProjectPath,
  selected,
  tree,
  jobs,
  onSelect,
  onActivate,
  onInspect,
  onConvert,
  onSetActiveProject,
  onMoveToProject,
  onOpenSettings,
  onCreateProject,
  onRevealPath,
  onToast,
}: {
  workspacePath: string;
  projects: ProjectSummary[];
  catchAllPath: string | null;
  activeProjectPath: string | null;
  /// What the Move control acts on. Null on a folder or a project root.
  selected: FileRow | null;
  tree: ProjectTreeState;
  jobs: Job[];
  /// Null on a project root, which has no `FileRow`.
  onSelect: (row: FileRow | null) => void;
  onActivate: (row: FileRow) => void;
  onInspect: (row: FileRow) => void;
  onConvert: (row: FileRow) => void;
  onSetActiveProject: (rel: string) => void;
  onMoveToProject: (row: FileRow, projectRel: string) => void;
  onOpenSettings: () => void;
  onCreateProject: (title: string) => Promise<void>;
  onRevealPath: (path: string) => void;
  onToast: (message: string) => void;
}) {
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

  /// The project the file already sits in is dropped: that move does nothing.
  const holding = selected === null ? null : selected.rel.split("/")[0];
  const movable = projects.filter((p) => p.path !== holding);

  const commitProject = async () => {
    const title = draft.trim();
    if (!title) {
      setCreating(false);
      setDraft("");
      return;
    }
    setBusy(true);
    try {
      await onCreateProject(title);
      setDraft("");
      setCreating(false);
    } catch (e) {
      onToast(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <aside className="lib-side">
      <div className="lib-side__head">
        <Label>Workspace</Label>
        <Meta size="sm" tone="ink" truncate title={workspacePath}>
          {basename(workspacePath)}
        </Meta>
        <Path path={tildePath(workspacePath)} className="lib-side__path" />
      </div>

      <div className="lib-side__list">
        <ProjectTree
          projects={projects}
          catchAllPath={catchAllPath}
          activeProjectPath={activeProjectPath}
          tree={tree}
          jobs={jobs}
          onSelect={onSelect}
          onActivate={onActivate}
          onInspect={onInspect}
          onConvert={onConvert}
          onSetActiveProject={onSetActiveProject}
        />
      </div>

      {/* A popup rather than a drag: Tauri owns the drag-and-drop channel for
          the native file drop, so an HTML5 drag has nowhere to land. */}
      {movable.length > 0 && selected !== null && (
        <div className="lib-side__move">
          <Select
            label="Move to"
            value=""
            onChange={(e) => {
              const rel = e.target.value;
              if (rel) onMoveToProject(selected, rel);
            }}
            options={[
              { value: "", label: selected.name },
              ...movable.map((p) => ({ value: p.path, label: p.title })),
            ]}
          />
        </div>
      )}

      <div className="lib-side__actions">
        {creating ? (
          <Stack gap={2}>
            <Input
              autoFocus
              placeholder="Project name"
              value={draft}
              disabled={busy}
              onChange={(e) => {
                setDraft(e.target.value);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") void commitProject();
                if (e.key === "Escape") {
                  // Abandoning a name must not reach App's Escape handler.
                  e.stopPropagation();
                  setCreating(false);
                  setDraft("");
                }
              }}
              onBlur={() => {
                if (!draft.trim()) setCreating(false);
              }}
            />
            <Row gap={2}>
              <Button size="sm" disabled={busy || !draft.trim()} onClick={() => void commitProject()}>
                Create
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() => {
                  setCreating(false);
                  setDraft("");
                }}
              >
                Cancel
              </Button>
            </Row>
          </Stack>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            icon={<PlusIcon />}
            onClick={() => {
              setCreating(true);
            }}
          >
            New project
          </Button>
        )}
      </div>

      <div className="lib-side__foot">
        <Button
          variant="ghost"
          size="sm"
          icon={<FolderOpenIcon />}
          onClick={() => {
            onRevealPath(workspacePath);
          }}
        >
          Reveal
        </Button>
        <Spacer />
        {/* A second door. The app menu carries the real Settings… at ⌘,. */}
        <Button variant="ghost" size="sm" icon={<GearSixIcon />} onClick={onOpenSettings}>
          Settings
        </Button>
      </div>
    </aside>
  );
}
