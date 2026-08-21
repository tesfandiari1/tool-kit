import type { ReactNode } from "react";
import { CodeIcon, CopyIcon, EyeIcon, FolderOpenIcon, XIcon } from "@phosphor-icons/react";
import { Button, cx, Path, Row, Segmented, Spacer, SourceEditor, StatusDot, Tabs, Text } from "@ui";
import { tildePath } from "@/app/format";
import { MarkdownViewer } from "./MarkdownViewer";
import { isDirty, saveNote, saveTone, type DocMode, type OpenDoc } from "./model";

/// The right half of the workspace: every open result, one at a time, with the
/// inspector card over it when the library points at something this pane cannot
/// read.
///
/// This is a pane, not a view. The column that produced these documents stays
/// on screen beside it, which is the whole point of the split — reading a
/// result no longer means leaving the thing that made it.
export function DocumentPane({
  docs,
  activeId,
  mode,
  allowEdit = true,
  inspector,
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
  /// Whether the Read/Edit toggle is offered. False while the host has no way
  /// to write the file back: an edit it cannot save is an edit it discards.
  allowEdit?: boolean;
  /// A Quick Look card, layered over the document rather than replacing it. The
  /// document underneath is covered and inert, never unmounted: `.doc-body` is
  /// keyed on the document id to give each tab its own CodeMirror instance, so
  /// unmounting on a stray tree click would throw away the reader's scroll
  /// position with nothing to say so.
  inspector?: ReactNode;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onModeChange: (mode: DocMode) => void;
  onEdit: (id: string, text: string) => void;
  onCopy: (doc: OpenDoc) => void;
  onReveal: (doc: OpenDoc) => void;
}) {
  const doc = docs.find((d) => d.id === activeId) ?? null;
  const covered = inspector !== undefined && inspector !== null;

  if (!doc) {
    // With nothing open the pane is either its empty state or, on a fresh
    // workspace where the first click lands on a PDF, the card on its own.
    return covered ? (
      <section className="doc is-covered">
        <div className="doc-stack">{inspector}</div>
      </section>
    ) : (
      <section className="doc doc-empty">
        <Text size="sm" tone="faint">
          Nothing open yet
        </Text>
        <Text size="xs" tone="ghost">
          Click a text file in the library to read it here. Everything else opens
          as a card.
        </Text>
      </section>
    );
  }

  const note = saveNote(doc.save);

  return (
    <section className={cx("doc", covered && "is-covered")}>
      <Tabs
        label="Open documents"
        /* `isDirty`, not `save === "edited"`: a document whose write the host
           refused still holds the edit only in memory, so the strip must mark
           it. Otherwise the tab looks settled and then asks on the way out. */
        items={docs.map((d) => ({ id: d.id, label: d.title, dirty: isDirty(d.save) }))}
        value={doc.id}
        onChange={onSelect}
        onClose={onClose}
      />

      {/* One box for the document and the card that covers it. `inert`
          goes on the two covered elements rather than on this wrapper, or
          it would reach the card as well. It is only ever applied on the
          same tick as a tree interaction that has already moved focus out
          of here: `inert` on an ancestor of the focused element blurs it
          with no event to intercept. */}
      <div className="doc-stack">
        <header className="doc-head" inert={covered}>
          <Row gap={2}>
            {/* Rule 2: the dot holds this slot at every save state, so a
                document going dirty never nudges the controls beside it. */}
            <StatusDot tone={saveTone(doc.save)} label={note ?? undefined} />
            {/* The path and the save note share one slot, and the note wins: what
                just happened to the file outranks where it came from. */}
            <div className="doc-id">
              {note !== null || doc.subtitle === null ? (
                <Text size="xs" tone={doc.save === "error" ? "fault" : "faint"} truncate title={note ?? undefined}>
                  {note}
                </Text>
              ) : (
                <Path path={tildePath(doc.subtitle)} />
              )}
            </div>
            <Spacer />
            {allowEdit && (
              <Segmented
                label="Document mode"
                size="sm"
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
            )}
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
        </header>

        {/* Keyed on the document, so switching tabs unmounts the surface rather
            than pushing new text through the one that is already there.
            Without it a single CodeMirror instance serves every tab, and its
            undo stack spans them: ⌘Z after a switch pops the swap that brought
            this document in, restoring the *previous* document's text, which
            then autosaves over this document's file. The read surface has the
            milder version of the same bug — it kept the last document's scroll
            offset — and the same key fixes it. */}
        <div
          key={doc.id}
          className="doc-body"
          inert={covered}
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
        {covered && inspector}
      </div>
    </section>
  );
}
