import type { ReactNode } from "react";
import { CodeIcon, CopyIcon, EyeIcon, FolderOpenIcon, XIcon } from "@phosphor-icons/react";
import { Button, cx, Path, Row, Segmented, Spacer, SourceEditor, StatusDot, Tabs, Text, Well } from "@ui";
import { basename, tildePath } from "@/app/format";
import { MarkdownViewer } from "./MarkdownViewer";
import { isDirty, saveNote, saveTone, type DocMode, type OpenDoc } from "./model";

/// Every open result, one at a time, with the inspector card over it when the
/// library points at something this pane cannot read.
export function DocumentPane({
  docs,
  activeId,
  mode,
  inspector,
  dragging = false,
  onPick,
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
  /// Layered over the document, which stays mounted: `.doc-body` is keyed on
  /// the document id, so unmounting loses the reader's scroll position.
  inspector?: ReactNode;
  /// A file is over the window, and the empty state has to answer it.
  dragging?: boolean;
  /// Open the file picker. A box that looks like a target answers a click.
  onPick: () => void;
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
    // With nothing open: the empty state, or the card on its own.
    return covered ? (
      <section className="doc is-covered">
        <div className="doc-stack">{inspector}</div>
      </section>
    ) : (
      <section className="doc doc-empty">
        {/* The window's one drop target, and it answers a click as well as a
            drag. The same control as the Run column's well. */}
        <Well
          className={cx("drop-well", dragging && "is-dropping")}
          selectable={false}
          /* The button inside fills the well and bubbles here, so one click
             would open the file panel twice. */
          onClick={(e) => {
            if (e.target instanceof Element && e.target.closest("button")) return;
            onPick();
          }}
        >
          <button
            type="button"
            className="drop-empty"
            title="Choose files (⌘O)"
            onClick={onPick}
          >
            <FolderOpenIcon weight="light" aria-hidden />
            <Text size="sm" tone="default">
              Choose files to turn into markdown
            </Text>
            <Text size="xs" tone="ghost" className="drop-hint">
              Or drop them here. PDFs, Word and Office files, images, audio, video.
            </Text>
          </button>
        </Well>
        <Text size="xs" tone="ghost">
          Click a text file in the library to read and edit it here. Everything
          else opens as a card.
        </Text>
      </section>
    );
  }

  const note = saveNote(doc.save);

  return (
    <section className={cx("doc", covered && "is-covered")}>
      <Tabs
        label="Open documents"
        /* `isDirty`, not `save === "edited"`: a refused write still holds the
           edit in memory, and the tab must not look settled. */
        items={docs.map((d) => ({ id: d.id, label: basename(d.id), dirty: isDirty(d.save) }))}
        value={doc.id}
        onChange={onSelect}
        onClose={onClose}
      />

      {/* `inert` goes on the two covered elements, never on this wrapper,
          which would reach the card too. It lands on the same tick as the tree
          interaction that already moved focus out: `inert` over the focused
          element blurs it with no event to intercept. */}
      <div className="doc-stack">
        <header className="doc-head" inert={covered}>
          <Row gap={2}>
            {/* The dot holds this slot at every state (UI.md rule 2). */}
            <StatusDot tone={saveTone(doc.save)} label={note ?? undefined} />
            {/* One slot, and the note wins over the path. */}
            <div className="doc-id">
              {note !== null ? (
                <Text size="xs" tone={doc.save === "error" ? "fault" : "faint"} truncate title={note}>
                  {note}
                </Text>
              ) : (
                <Path path={tildePath(doc.id)} />
              )}
            </div>
            <Spacer />
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
            <Button
              variant="ghost"
              size="sm"
              iconOnly
              icon={<CopyIcon />}
              title="Copy"
              aria-label="Copy"
              onClick={() => { onCopy(doc); }}
            />
            <Button
              variant="ghost"
              size="sm"
              iconOnly
              icon={<FolderOpenIcon />}
              title="Show in Finder"
              aria-label="Show in Finder"
              onClick={() => { onReveal(doc); }}
            />
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
          id={`ui-tabpanel-${encodeURIComponent(doc.id)}`}
          aria-labelledby={`ui-tab-${encodeURIComponent(doc.id)}`}
        >
          {mode === "read" ? (
            <MarkdownViewer text={doc.text} />
          ) : (
            <SourceEditor
              value={doc.text}
              label={`${basename(doc.id)} source`}
              onChange={(text) => { onEdit(doc.id, text); }}
            />
          )}
        </div>
        {covered && inspector}
      </div>
    </section>
  );
}
