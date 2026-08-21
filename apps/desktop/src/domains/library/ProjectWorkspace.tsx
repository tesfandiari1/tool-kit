import { FolderOpenIcon, PlayIcon } from "@phosphor-icons/react";
import { Button, Mono, Path, Row, Spacer, Tabs, Text } from "@ui";
import type { ProjectSummary } from "@/app/types";
import { tildePath } from "@/app/format";
import { MarkdownViewer } from "@/domains/thread/MarkdownViewer";
import { WELCOME_MARKDOWN } from "./welcomeContent";

/// The library home: a pinned welcome tab, the project path, and enough
/// structure that the centre reads as a workspace rather than a blank mailbox.
export function ProjectWorkspace({
  workspacePath,
  project,
  onOpenRun,
  onRevealProject,
}: {
  workspacePath: string;
  project: ProjectSummary;
  onOpenRun: () => void;
  onRevealProject: () => void;
}) {
  const projectPath = `${workspacePath}/${project.path}`;

  return (
    <div className="library-workspace">
      <div className="library-main__head">
        <Tabs
          label="Documents"
          items={[{ id: "welcome", label: "welcome.md" }]}
          value="welcome"
          onChange={() => {
            /* Pinned until import creates real tabs. */
          }}
        />
        <Row gap={2} align="center" className="library-main__meta">
          <Path path={tildePath(projectPath)} />
          <Spacer />
          <Button variant="link" size="sm" icon={<FolderOpenIcon />} onClick={onRevealProject}>
            Reveal
          </Button>
          <Button variant="link" size="sm" icon={<PlayIcon />} onClick={onOpenRun}>
            Open Run
          </Button>
        </Row>
      </div>
      <div
        className="library-workspace__body"
        role="tabpanel"
        id="ui-tabpanel-welcome"
        aria-labelledby="ui-tab-welcome"
      >
        <MarkdownViewer text={WELCOME_MARKDOWN} />
      </div>
      <div className="library-main__foot">
        <Text size="xs" tone="ghost">
          Drop files anywhere in this window, or press{" "}
          <Mono as="span" size="xs" tone="ghost">
            ⌘O
          </Mono>{" "}
          to open the file picker and switch to Run.
        </Text>
      </div>
    </div>
  );
}
