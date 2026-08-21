import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { LibraryShell } from "./LibraryShell";

const noop = () => undefined;

describe("LibraryShell", () => {
  it("renders a message in the centre column when the project list is empty", () => {
    const html = renderToStaticMarkup(
      <LibraryShell
        workspacePath="/tmp/workspace"
        projects={[]}
        activeProjectId={null}
        libraryHome
        compact={false}
        panel={null}
        onSelectProject={noop}
        onOpenSettings={noop}
        onOpenLibrary={noop}
        onOpenRun={noop}
        onCreateProject={() => Promise.resolve()}
        onRevealPath={noop}
        onToast={noop}
      />,
    );

    expect(html).toContain("library-main__panel");
    expect(html).toContain("No projects yet.");
  });
});
