import { useEffect, useState } from "react";
import { CheckCircleIcon, EyeIcon, FolderOpenIcon, WarningCircleIcon, XIcon } from "@phosphor-icons/react";
import { Button, Input, Label, Mono, Row, Spacer, Status, Text } from "@ui";
import { commands } from "@/app/commands";
import { basename, fmtWhen } from "@/app/format";
import { HISTORY_LIMIT, type HistoryEntry } from "@/app/types";
import { confirm } from "@/platform/host";

/// Every finished job, newest first. Reveal-only on purpose: the extracted
/// text is not stored, so the database stays small and the files stay the
/// single source of truth.
export function HistoryPanel({
  refreshKey,
  onChanged,
  onOpen,
  onToast,
  onClose,
}: {
  refreshKey: number;
  /// Call after anything that changes what the history says, so the
  /// already-done counts on the run screen stop citing rows we just deleted.
  onChanged: () => void;
  onOpen: (entry: HistoryEntry) => void;
  onToast: (msg: string) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  // null while the first read is in flight, so an empty history and a pending
  // one don't show the same thing.
  const [rows, setRows] = useState<HistoryEntry[] | null>(null);
  const [nowMs] = useState(() => Date.now());

  useEffect(() => {
    let live = true;
    // Debounced so typing a search doesn't fire a query per keystroke.
    const t = window.setTimeout(() => {
      void commands
        .listHistory(query, HISTORY_LIMIT)
        .then((r) => live && setRows(r))
        .catch(() => live && setRows([]));
    }, 120);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [query, refreshKey]);

  const clear = async () => {
    const ok = await confirm(
      "Every entry is removed. The result files themselves are not touched.",
      { title: "Clear the history?", kind: "warning", okLabel: "Clear", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    try {
      await commands.clearHistory();
      // Empty the list now rather than waiting on the refetch debounce.
      setRows([]);
      // Without this the run screen keeps its cached already-done counts, so
      // Run stays disabled insisting the files are already done — citing a
      // history that no longer exists.
      onChanged();
      onToast("History cleared");
    } catch (e) {
      onToast(String(e));
    }
  };

  // The result if it is still there, otherwise the file it came from — which
  // is the useful answer for a row that failed.
  const reveal = (e: HistoryEntry) => {
    void commands.revealPath(e.outputPath ?? e.sourcePath).catch((err: unknown) => {
      onToast(String(err));
    });
  };

  return (
    <main className="flow">
      <Row gap={3}>
        {/* A full page is "400+", not "400": the count is what we fetched, and
            claiming it is the whole archive would be a lie the user can't see. */}
        <Label tone="strong">
          History
          {rows && rows.length > 0
            ? ` · ${String(rows.length)}${rows.length >= HISTORY_LIMIT ? "+" : ""}`
            : ""}
        </Label>
        <Spacer />
        {rows && rows.length > 0 && (
          <Button variant="link" onClick={() => void clear()}>
            Clear history
          </Button>
        )}
        <Button variant="ghost" iconOnly icon={<XIcon />} onClick={onClose} aria-label="Close history" />
      </Row>

      {/* aria-label rather than a visible Label: the placeholder carries the
          meaning visually, but a placeholder disappears on first keystroke and
          is not a reliable accessible name. */}
      <Input
        type="search"
        aria-label="Search history by file or folder"
        placeholder="Search by file or folder"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
        }}
      />

      {rows === null ? (
        <div className="hist-empty">Reading…</div>
      ) : rows.length === 0 ? (
        <div className="hist-empty">
          {query ? (
            `Nothing matches “${query}”`
          ) : (
            <>
              Nothing here yet
              <br />
              Every finished file is logged automatically
            </>
          )}
        </div>
      ) : (
        <div className="hist-list">
          {rows.map((e) => (
            <div className="job" key={e.id}>
              <Status tone={e.status === "done" ? "pass" : "fault"} label={e.status}>
                {e.status === "done" ? (
                  <CheckCircleIcon weight="fill" />
                ) : (
                  <WarningCircleIcon weight="fill" />
                )}
              </Status>
              <div className="job-body">
                <Mono size="xs" tone="ink" className="job-name" title={e.sourcePath}>
                  {e.fileName}
                </Mono>
                {e.status === "failed" && e.error ? (
                  <Text as="span" size="xs" tone="fault" className="job-sub">
                    {e.error}
                  </Text>
                ) : (
                  e.outputPath && (
                    <Text as="span" size="xs" tone="faint" className="job-sub" title={e.outputPath}>
                      {basename(e.outputPath)}
                    </Text>
                  )
                )}
              </div>
              <Mono size="xs" className="job-time">
                {fmtWhen(e.finishedAt, nowMs)}
              </Mono>
              <div className="job-actions">
                {e.status === "done" && e.outputPath && (
                  <Button
                    variant="ghost"
                    size="sm"
                    iconOnly
                    icon={<EyeIcon />}
                    title="Open"
                    aria-label="Open"
                    onClick={() => {
                      onOpen(e);
                    }}
                  />
                )}
                <Button
                  variant="ghost"
                  size="sm"
                  iconOnly
                  icon={<FolderOpenIcon />}
                  title={e.outputPath ? "Show the result in Finder" : "Show the file in Finder"}
                  aria-label="Show in Finder"
                  onClick={() => {
                    reveal(e);
                  }}
                />
              </div>
            </div>
          ))}
        </div>
      )}
    </main>
  );
}
