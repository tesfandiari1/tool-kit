import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { FolderOpenIcon, XIcon } from "@phosphor-icons/react";
import { Badge, Button, Divider, Label, Mono, Path, Row, Spacer, Stack, Text, Well } from "@ui";
import { commands } from "@/app/commands";
import { fmtWhen, tildePath } from "@/app/format";
import type { FileRow } from "@/app/types";

/// Enough to recognize the document, short enough to stay a card.
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

/// The extension, or Folder.
function kindOf(row: FileRow): string {
  if (row.isDir) return "Folder";
  return row.ext === "" ? "File" : row.ext.toUpperCase();
}

/// A Quick Look card over the document pane: it reports, and the caller acts.
/// Not `role="dialog"` and not `aria-modal`, which would hide the live tab
/// strip.
export function FileInspector({
  row,
  primary,
  onReveal,
  onClose,
}: {
  row: FileRow;
  /// The one thing worth doing with this file. Absent when there is none.
  primary?: ReactNode;
  onReveal: (path: string) => void;
  onClose: () => void;
}) {
  const titleId = useId();
  const card = useRef<HTMLElement>(null);
  const restoreTo = useRef<HTMLElement | null>(null);
  const [nowMs] = useState(() => Date.now());

  // Both ways out unmount the card, and focus would land on <body>. Only when
  // the card still holds it, so a click elsewhere is not yanked back. Layout,
  // not passive: a passive cleanup runs after the mutation, too late to see.
  useLayoutEffect(() => {
    const el = card.current;
    restoreTo.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return () => {
      const active = document.activeElement;
      if (active instanceof HTMLElement && el?.contains(active) === true) {
        restoreTo.current?.focus();
      }
      restoreTo.current = null;
    };
  }, []);
  /// Keyed on the path, so an arrow never shows the last file's lines.
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
    <aside ref={card} className="doc-inspect" aria-labelledby={titleId}>
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

        <Stack gap={4} className="doc-inspect__body">
          {/* Focus never moves when the card swaps, so one line announces it.
              A live region over the card reads every field on every press. */}
          <Text size="sm" tone="default" aria-live="polite" aria-atomic="true">
            {state === "" ? row.name : `${row.name}, ${state}`}
          </Text>

          <Row gap={2}>
            <Badge>{row.isDir ? "Folder" : fmtBytes(row.size)}</Badge>
            {/* `truncate` is the library's only nowrap, and the date needs
                it beside the media type. */}
            <Mono size="xs" tone="ghost" truncate>
              {fmtWhen(Math.floor(row.modifiedMs / 1000), nowMs)}
            </Mono>
          </Row>

          {/* Its own line: the longest string in the card. */}
          {!row.isDir && row.mediaType !== "" && (
            <Mono size="xs" tone="ghost" truncate>
              {row.mediaType}
            </Mono>
          )}

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
