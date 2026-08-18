import { CodeIcon, CopyIcon, EyeIcon, FolderOpenIcon, XIcon } from "@phosphor-icons/react";
import { Button, Display, Row, Segmented, Spacer, SourceEditor, StatusDot, Tabs, Text } from "@ui";
import { MarkdownViewer } from "./MarkdownViewer";
import { saveNote, saveTone, type DocMode, type OpenDoc } from "./model";

/// The right half of the workspace: every open result, one at a time.
///
/// This is a pane, not a view. The queue that produced these documents stays
/// on screen beside it, which is the whole point of the expansion — reading a
/// result no longer means leaving the thing that made it.
export function DocumentPane({
  docs,
  activeId,
  mode,
  onSelect,
  onClose,
  onModeChange,
  onEdit,
  onCopy,
  onReveal,
}: {
  docs: OpenDoc[];
  activeId: string | null;
  mode: DocMode;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onModeChange: (mode: DocMode) => void;
  onEdit: (id: string, text: string) => void;
  onCopy: (doc: OpenDoc) => void;
  onReveal: (doc: OpenDoc) => void;
}) {
  const doc = docs.find((d) => d.id === activeId) ?? null;

  if (!doc) {
    return (
      <section className="doc doc-empty">
        <Text size="sm" tone="faint">
          Nothing open yet
        </Text>
        <Text size="xs" tone="ghost">
          Pick a finished file on the left to read it here
        </Text>
      </section>
    );
  }

  const note = saveNote(doc.save);

  return (
    <section className="doc">
      <Tabs
        label="Open documents"
        items={docs.map((d) => ({ id: d.id, label: d.title, dirty: d.save === "edited" }))}
        value={doc.id}
        onChange={onSelect}
        onClose={onClose}
      />

      <header className="doc-head">
        <Row gap={2}>
          {/* Rule 2: the dot holds this slot at every save state, so a
              document going dirty never nudges the title beside it. */}
          <StatusDot tone={saveTone(doc.save)} label={note ?? undefined} />
          <Display as="h2" size="lg" className="ui-truncate" title={doc.title}>
            {doc.title}
          </Display>
          <Spacer />
          <Segmented
            label="Document mode"
            value={mode}
            onChange={onModeChange}
            options={[
              {
                value: "read" as const,
                label: "Read",
                icon: (on: boolean) => <EyeIcon weight={on ? "fill" : "regular"} />,
              },
              {
                value: "edit" as const,
                label: "Edit",
                icon: () => <CodeIcon />,
              },
            ]}
          />
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            icon={<CopyIcon />}
            title="Copy"
            aria-label="Copy"
            onClick={() => { onCopy(doc); }}
          />
          {doc.revealPath && (
            <Button
              variant="ghost"
              size="sm"
              iconOnly
              icon={<FolderOpenIcon />}
              title="Show in Finder"
              aria-label="Show in Finder"
              onClick={() => { onReveal(doc); }}
            />
          )}
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            icon={<XIcon />}
            title="Close"
            aria-label="Close document"
            onClick={() => { onClose(doc.id); }}
          />
        </Row>
        {/* The subtitle and the save note share one line. The note wins when
            there is one, because what just happened to the file matters more
            than where it came from. */}
        <Text
          size="xs"
          tone={doc.save === "error" ? "fault" : "faint"}
          truncate
          title={note ?? doc.subtitle ?? undefined}
        >
          {note ?? doc.subtitle}
        </Text>
      </header>

      <div
        className="doc-body"
        role="tabpanel"
        id={`ui-tabpanel-${doc.id}`}
        aria-labelledby={`ui-tab-${doc.id}`}
      >
        {mode === "read" ? (
          <MarkdownViewer text={doc.text} />
        ) : (
          <SourceEditor
            value={doc.text}
            label={`${doc.title} source`}
            onChange={(text) => { onEdit(doc.id, text); }}
          />
        )}
      </div>
    </section>
  );
}
