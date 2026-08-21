import { useState } from "react";
import { FolderIcon, FolderOpenIcon, GearSixIcon, PlusIcon, TrayIcon } from "@phosphor-icons/react";
import { Button, Input, Label, Mono, Path, Row, Spacer, Stack } from "@ui";
import type { ProjectSummary } from "@/app/types";
import { tildePath } from "@/app/format";

const INBOX = "Inbox";

function pinned(projects: ProjectSummary[]): ProjectSummary[] {
  return [...projects].sort((a, b) => {
    if (a.title === b.title) return 0;
    if (a.title === INBOX) return -1;
    if (b.title === INBOX) return 1;
    return 0;
  });
}

/// The workspace, its projects, and the way out to Settings.
export function ProjectSidebar({
  workspaceName,
  workspacePath,
  projects,
  activeProjectId,
  libraryHome,
  onSelectProject,
  onOpenWorkspace,
  onOpenSettings,
  onCreateProject,
  onRevealPath,
  onToast,
}: {
  workspaceName: string;
  workspacePath: string;
  projects: ProjectSummary[];
  activeProjectId: string | null;
  /// On library home no project row is the current place; the workspace head is.
  libraryHome: boolean;
  onSelectProject: (id: string) => void;
  onOpenWorkspace: () => void;
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
      <button
        type="button"
        className={`lib-side__head lib-side__workspace${libraryHome ? " is-active" : ""}`}
        onClick={onOpenWorkspace}
      >
        <Label>Workspace</Label>
        <Mono size="sm" tone="ink" truncate title={workspacePath}>
          {workspaceName}
        </Mono>
        <Path path={tildePath(workspacePath)} className="lib-side__path" />
      </button>

      <nav className="lib-side__list" aria-label="Projects">
        {pinned(projects).map((p) => {
          const active = p.id === activeProjectId;
          return (
            <button
              key={p.id}
              type="button"
              className={`lib-project${active ? " is-active" : ""}`}
              aria-current={active ? "true" : undefined}
              title={p.path}
              onClick={() => {
                onSelectProject(p.id);
              }}
            >
              {p.title === INBOX ? <TrayIcon weight={active ? "fill" : "regular"} /> : <FolderIcon />}
              <Mono size="xs" tone={active ? "ink" : "default"} truncate>
                {p.title}
              </Mono>
              <Spacer />
              <Mono size="xs" tone="ghost" className="lib-project__count" aria-hidden />
            </button>
          );
        })}
      </nav>

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
        <Button variant="ghost" size="sm" icon={<GearSixIcon />} onClick={onOpenSettings}>
          Settings
        </Button>
      </div>
    </aside>
  );
}
