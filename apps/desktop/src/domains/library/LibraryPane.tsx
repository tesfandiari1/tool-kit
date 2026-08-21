import { useState } from "react";
import { FolderOpenIcon, GearSixIcon, PlusIcon } from "@phosphor-icons/react";
import { Button, Input, Label, Mono, Path, Row, Spacer, Stack } from "@ui";
import type { FileRow, Job, ProjectSummary } from "@/app/types";
import { basename, tildePath } from "@/app/format";
import type { ProjectTreeState } from "@/shell/useProjectTree";
import { ProjectTree } from "./ProjectTree";

/// The library: the workspace it belongs to, its projects as a disclosure tree,
/// and the two ways out of it.
///
/// The whole left column, not a sidebar beside a centre column. Run and History
/// take the same pane when the nav asks for them, which is why the tree gets
/// the full width rather than a fixed 200px track.
export function LibraryPane({
  workspacePath,
  projects,
  tree,
  jobs,
  onSelect,
  onActivate,
  onInspect,
  onConvert,
  onOpenSettings,
  onCreateProject,
  onRevealPath,
  onToast,
}: {
  workspacePath: string;
  projects: ProjectSummary[];
  tree: ProjectTreeState;
  jobs: Job[];
  /// The selection moved. Null on a project root, which has no `FileRow` of
  /// its own.
  onSelect: (row: FileRow | null) => void;
  onActivate: (row: FileRow) => void;
  onInspect: (row: FileRow) => void;
  onConvert: (row: FileRow) => void;
  onOpenSettings: () => void;
  onCreateProject: (title: string) => Promise<void>;
  onRevealPath: (path: string) => void;
  onToast: (message: string) => void;
}) {
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

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
        <Mono size="sm" tone="ink" truncate title={workspacePath}>
          {basename(workspacePath)}
        </Mono>
        <Path path={tildePath(workspacePath)} className="lib-side__path" />
      </div>

      <div className="lib-side__list">
        <ProjectTree
          projects={projects}
          tree={tree}
          jobs={jobs}
          onSelect={onSelect}
          onActivate={onActivate}
          onInspect={onInspect}
          onConvert={onConvert}
        />
      </div>

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
                  // The app's one Escape handler closes the open document.
                  // Stopping here is what keeps that from firing while the user
                  // is only abandoning a name.
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
        {/* The gear stays here, where the user asked for it. A Mac user's first
            look is the app menu, which now carries a real Settings… at ⌘,. */}
        <Button variant="ghost" size="sm" icon={<GearSixIcon />} onClick={onOpenSettings}>
          Settings
        </Button>
      </div>
    </aside>
  );
}
