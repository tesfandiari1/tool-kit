import { FolderIcon, GearSixIcon, TrayIcon } from "@phosphor-icons/react";
import { Button, Label, Mono, Spacer } from "@ui";
import type { ProjectSummary } from "@/app/types";

/// Inbox is where every import lands, so it sits above the list rather than
/// wherever the host happened to return it. Identified by title because that
/// is what the workspace guarantees on disk.
const INBOX = "Inbox";

function pinned(projects: ProjectSummary[]): ProjectSummary[] {
  return [...projects].sort((a, b) => {
    if (a.title === b.title) return 0;
    if (a.title === INBOX) return -1;
    if (b.title === INBOX) return 1;
    return 0;
  });
}

/// The workspace, its projects, and the way out to Settings. Nothing here
/// creates anything: this is where you are, not what you can do.
export function ProjectSidebar({
  workspaceName,
  workspacePath,
  projects,
  activeProjectId,
  onSelectProject,
  onOpenSettings,
}: {
  workspaceName: string;
  workspacePath: string;
  projects: ProjectSummary[];
  activeProjectId: string | null;
  onSelectProject: (id: string) => void;
  onOpenSettings: () => void;
}) {
  return (
    <aside className="lib-side">
      <div className="lib-side__head">
        <Label>Workspace</Label>
        <Mono size="sm" tone="ink" truncate title={workspacePath}>
          {workspaceName}
        </Mono>
      </div>

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
              {/* The document count's slot, held open at its own width and
                  empty until the host reports one. Reserved rather than added
                  later, so the first import lights the number up instead of
                  shifting every row that has one. */}
              <Mono size="xs" tone="ghost" className="lib-project__count" aria-hidden />
            </button>
          );
        })}
      </nav>

      <div className="lib-side__foot">
        <Button variant="ghost" size="sm" icon={<GearSixIcon />} onClick={onOpenSettings}>
          Settings
        </Button>
      </div>
    </aside>
  );
}
