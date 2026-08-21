import { useEffect, useId, useState, type ReactNode } from "react";
import { FolderOpenIcon, XIcon } from "@phosphor-icons/react";
import { Badge, Button, Divider, Label, Mono, Path, Row, Spacer, Stack, Text, Well } from "@ui";
import { commands } from "@/app/commands";
import { fmtWhen, tildePath } from "@/app/format";
import type { FileRow } from "@/app/types";

/// How much of a text file the card shows. Enough to recognize the document,
/// short enough that the card stays a card.
const PREVIEW_LINES = 12;

function fmtBytes(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : String(Math.round(value))} ${units[unit]}`;
}

/// What a file is, said in the way the row says it: the extension, or Folder.
function kindOf(row: FileRow): string {
  if (row.isDir) return "Folder";
  return row.ext === "" ? "File" : row.ext.toUpperCase();
}

/// A Quick Look card over the document pane.
///
/// It follows the tree's selection, so nothing here opens anything and nothing
/// here converts anything: it reports, and the caller acts.
///
/// Not `role="dialog"`: there is no task and no dismissal contract. Not
/// `aria-modal`: that would hide the tab strip this design keeps live.
export function FileInspector({
  row,
  primary,
  onReveal,
  onClose,
}: {
  row: FileRow;
  /// The one thing worth doing with this file, decided by the caller: opening
  /// the result it already has, or converting it. Absent when there is neither.
  primary?: ReactNode;
  onReveal: (path: string) => void;
  onClose: () => void;
}) {
  const titleId = useId();
  const [nowMs] = useState(() => Date.now());
  /// Keyed on the path rather than reset in an effect, so arrowing to the next
  /// row never shows the previous file's opening lines for a frame.
  const [head, setHead] = useState<{ path: string; text: string } | null>(null);

  useEffect(() => {
    if (!row.openable) return;
    let live = true;
    void commands
      .readDocument(row.path)
      .then(({ text }) => {
        if (live) setHead({ path: row.path, text: text.split("\n").slice(0, PREVIEW_LINES).join("\n") });
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [row.openable, row.path]);

  const opening = head?.path === row.path ? head.text : null;
  const state = row.resultName !== null
    ? `converted to ${row.resultName}`
    : row.job !== null
      ? "not converted yet"
      : "";

  return (
    <aside className="doc-inspect" aria-labelledby={titleId}>
      <div className="doc-inspect__card">
        <Row gap={2} className="doc-inspect__head">
          <Label as="h2" id={titleId} tone="strong">
            {kindOf(row)}
          </Label>
          <Spacer />
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            icon={<XIcon />}
            title="Close"
            aria-label="Close inspector"
            onClick={onClose}
          />
        </Row>

        <Stack gap={3} className="doc-inspect__body">
          {/* Focus never moves when the card swaps, so an arrow press would
              change the pane silently. One line announces it. The whole card
              inside a live region would read every field on every press. */}
          <Text size="sm" tone="default" aria-live="polite" aria-atomic="true">
            {state === "" ? row.name : `${row.name}, ${state}`}
          </Text>

          <Row gap={2}>
            <Badge>{row.isDir ? "Folder" : fmtBytes(row.size)}</Badge>
            <Mono size="xs" tone="ghost">
              {fmtWhen(Math.floor(row.modifiedMs / 1000), nowMs)}
            </Mono>
            <Spacer />
            {!row.isDir && row.mediaType !== "" && (
              <Mono size="xs" tone="ghost" truncate>
                {row.mediaType}
              </Mono>
            )}
          </Row>

          <Path path={tildePath(row.path)} />

          {opening !== null && (
            <>
              <Divider />
              <Well selectable={false}>
                <Text size="xs" tone="faint" className="doc-inspect__peek">
                  {opening}
                </Text>
              </Well>
            </>
          )}
        </Stack>

        <Row gap={2} className="doc-inspect__foot">
          <Button
            variant="ghost"
            size="sm"
            icon={<FolderOpenIcon />}
            onClick={() => {
              onReveal(row.path);
            }}
          >
            Reveal
          </Button>
          <Spacer />
          {primary}
        </Row>
      </div>
    </aside>
  );
}
