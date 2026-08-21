import type { ReactNode } from "react";
import { basename } from "@/app/format";
import type { ProjectSummary } from "@/app/types";
import { ProjectSidebar } from "./ProjectSidebar";
import { ProjectWorkspace } from "./ProjectWorkspace";

/// The workspace chrome: projects stay on the left while Run, Settings, and
/// History swap the main column without hiding the library.
export function LibraryShell({
  workspacePath,
  projects,
  activeProjectId,
  libraryHome,
  panel,
  onSelectProject,
  onOpenSettings,
  onOpenLibrary,
  onOpenRun,
  onCreateProject,
  onRevealPath,
  onToast,
}: {
  workspacePath: string;
  projects: ProjectSummary[];
  activeProjectId: string | null;
  /// True when the main column shows the library home rather than a panel.
  libraryHome: boolean;
  /// Run, Settings, or History when the nav selects them. Null on library home.
  panel: ReactNode | null;
  onSelectProject: (id: string) => void;
  onOpenSettings: () => void;
  onOpenLibrary: () => void;
  onOpenRun: () => void;
  onCreateProject: (title: string) => Promise<void>;
  onRevealPath: (path: string) => void;
  onToast: (message: string) => void;
}) {
  const active =
    projects.find((p) => p.id === activeProjectId) ??
    (projects.length > 0 ? projects[0] : null);

  const main =
    panel ??
    (active ? (
      <ProjectWorkspace
        workspacePath={workspacePath}
        project={active}
        onOpenRun={onOpenRun}
        onRevealProject={() => {
          onRevealPath(`${workspacePath}/${active.path}`);
        }}
      />
    ) : null);

  return (
    <div className="library-shell">
      <ProjectSidebar
        workspaceName={basename(workspacePath)}
        workspacePath={workspacePath}
        projects={projects}
        activeProjectId={active?.id ?? null}
        libraryHome={libraryHome}
        onSelectProject={(id) => {
          onSelectProject(id);
          onOpenLibrary();
        }}
        onOpenWorkspace={() => {
          onOpenLibrary();
        }}
        onOpenSettings={onOpenSettings}
        onCreateProject={onCreateProject}
        onRevealPath={onRevealPath}
        onToast={onToast}
      />
      <main className="library-main">
        {main && <div className="library-main__panel">{main}</div>}
      </main>
    </div>
  );
}
