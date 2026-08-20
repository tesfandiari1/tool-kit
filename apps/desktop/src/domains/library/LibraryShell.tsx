import { basename } from "@/app/format";
import type { ProjectSummary } from "@/app/types";
import { EmptyInbox } from "./EmptyInbox";
import { ProjectSidebar } from "./ProjectSidebar";

/// The home surface: projects on the left, the selected project's documents in
/// the centre. This is what the app opens on, in place of the run queue.
export function LibraryShell({
  workspacePath,
  projects,
  activeProjectId,
  onSelectProject,
  onOpenSettings,
}: {
  workspacePath: string;
  projects: ProjectSummary[];
  activeProjectId: string | null;
  onSelectProject: (id: string) => void;
  onOpenSettings: () => void;
}) {
  /// The remembered project, or the first one the workspace has. A saved id
  /// can name a project the user deleted in Finder, so the fallback is not
  /// defensive: it is the normal answer after a reconcile.
  const active =
    projects.find((p) => p.id === activeProjectId) ??
    (projects.length > 0 ? projects[0] : null);

  return (
    <div className="library-shell">
      <ProjectSidebar
        workspaceName={basename(workspacePath)}
        workspacePath={workspacePath}
        projects={projects}
        activeProjectId={active?.id ?? null}
        onSelectProject={onSelectProject}
        onOpenSettings={onOpenSettings}
      />
      {/* The document list belongs in this slot. Nothing can reach a project
          yet — import is the next milestone — so a project has nothing to
          list and the empty state is the whole centre. */}
      <main className="library-main">
        <EmptyInbox projectTitle={active?.title ?? "Inbox"} />
      </main>
    </div>
  );
}
