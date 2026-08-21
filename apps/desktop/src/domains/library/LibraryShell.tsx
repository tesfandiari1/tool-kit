import type { ReactNode } from "react";
import { Text } from "@ui";
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
  compact,
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
  /// The whole shell is in the split's start pane, beside an open document.
  /// The sidebar goes away there: `SPLIT.minStart` is what the run column
  /// alone needs, so a 200px project list beside it leaves the job segmented
  /// clipping at every window width. The four views stay one click away in the
  /// title bar's nav, and closing the document brings the projects back.
  compact: boolean;
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

  // A project list that came back empty (no projects yet, or a workspace the
  // host could not read) still needs a centre column: rendering nothing here
  // reads as the app being broken rather than as an empty library.
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
    ) : (
      <div className="library-main__empty">
        <Text size="sm" tone="faint">
          No projects yet.
        </Text>
        <Text size="xs" tone="ghost">
          Use New project in the sidebar to create one.
        </Text>
      </div>
    ));

  return (
    <div className={`library-shell${compact ? " is-compact" : ""}`}>
      {!compact && (
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
      )}
      <main className="library-main">
        {main && <div className="library-main__panel">{main}</div>}
      </main>
    </div>
  );
}
