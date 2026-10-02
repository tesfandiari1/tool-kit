import { useEffect, useState } from "react";
import { EyeIcon, FolderOpenIcon } from "@phosphor-icons/react";
import { Badge, Button, Input, Meta, Row, Spacer, StatusDot, Text, Well } from "@ui";
import { commands } from "@/app/commands";
import { basename, fmtWhen } from "@/app/format";
import { HISTORY_LIMIT, type HistoryEntry } from "@/app/types";
import { confirm } from "@/platform/host";
import { FlowLayout } from "@/shell/FlowLayout";

/// Wrote a result and it is still on disk. The table remembers where a result
/// went, never whether it is still there.
function hasResult(e: HistoryEntry): e is HistoryEntry & { outputPath: string } {
  return e.outputPath !== null && e.outputExists;
}

/// Every finished job, newest first. The text is not stored, so files stay
/// authoritative.
export function HistoryPanel({
  refreshKey,
  onChanged,
  onOpen,
  onToast,
  dragging = false,
}: {
  refreshKey: number;
  /// Call after any change, or the run screen cites deleted rows.
  onChanged: () => void;
  onOpen: (entry: HistoryEntry) => void;
  onToast: (msg: string) => void;
  /// A file is over the window, and the list has to answer it.
  dragging?: boolean;
}) {
  const [query, setQuery] = useState("");
  // Null while the first read is in flight: empty and pending differ.
  const [rows, setRows] = useState<HistoryEntry[] | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());

  // Without a tick, a row that opened saying "just now" says it an hour
  // later. `fmtWhen`'s finest bucket is a minute.
  useEffect(() => {
    const t = window.setInterval(() => {
      setNowMs(Date.now());
    }, 30_000);
    return () => {
      window.clearInterval(t);
    };
  }, []);

  useEffect(() => {
    let live = true;
    const t = window.setTimeout(() => {
      void commands
        // One row past the page, which is how the badge knows a "+" is earned.
        .listHistory(query, HISTORY_LIMIT + 1)
        .then((r) => live && setRows(r))
        // A failed read must not pass for an empty history.
        .catch((e: unknown) => {
          if (!live) return;
          setRows([]);
          onToast(String(e));
        });
    }, 120);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [query, refreshKey, onToast]);

  const clear = async () => {
    const ok = await confirm(
      "Every entry is removed. The result files themselves are not touched.",
      { title: "Clear the history?", kind: "warning", okLabel: "Clear", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    try {
      await commands.clearHistory();
      setRows([]);
      // Or Run stays disabled citing a history that no longer exists.
      onChanged();
      onToast("History cleared");
    } catch (e) {
      onToast(String(e));
    }
  };

  const shown = rows?.slice(0, HISTORY_LIMIT) ?? null;
  const overflow = rows !== null && rows.length > HISTORY_LIMIT;

  // The result, or the source it came from: the answer for a failed row.
  const reveal = (e: HistoryEntry) => {
    void commands.revealPath(hasResult(e) ? e.outputPath : e.sourcePath).catch((err: unknown) => {
      onToast(String(err));
    });
  };

  return (
    <FlowLayout
      head={
        <>
          <Row gap={2}>
            <Spacer />
            <Badge count>
              {shown === null ? "" : overflow ? `${String(shown.length)}+` : String(shown.length)}
            </Badge>
            <Button
              variant="ghost"
              /* `clear_history` deletes the whole table, so a search matching
                 nothing must not disable it. */
              disabled={!shown || (shown.length === 0 && query === "")}
              onClick={() => void clear()}
            >
              Clear history
            </Button>
          </Row>

          <Input
            type="search"
            aria-label="Search history by file or folder"
            placeholder="Search by file or folder"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
            }}
          />
        </>
      }
    >
      {dragging && (
        <Well className="drop-well is-dropping" selectable={false}>
          <Text size="sm" tone="faint">
            Drop files to add them to Run
          </Text>
        </Well>
      )}
      {shown === null ? (
        <div className="hist-empty">
          <Text size="sm" tone="faint">
            Reading…
          </Text>
        </div>
      ) : shown.length === 0 ? (
        <div className="hist-empty">
          {query ? (
            <Text size="sm" tone="faint">
              Nothing matches “{query}”
            </Text>
          ) : (
            <>
              <Text size="sm" tone="faint">
                Nothing here yet
              </Text>
              <Text size="xs" tone="ghost">
                Every finished file is logged automatically
              </Text>
            </>
          )}
        </div>
      ) : (
        <div className="hist-list">
          {shown.map((e) => {
            /// Nothing to open. The row stays and says so.
            const openable = e.status === "done" && hasResult(e);
            const resultGone = e.outputPath !== null && !e.outputExists;
            return (
              <div
                className="job"
                key={e.id}
                /* The row forwards its click to the primary control, and bails
                   inside a button or the two fire twice. */
                onClick={(ev) => {
                  if (!openable) return;
                  if (ev.target instanceof Element && ev.target.closest("button")) return;
                  onOpen(e);
                }}
              >
                {/* Green means a result you can open, so a deleted one goes
                    neutral. */}
                <StatusDot
                  tone={e.status === "failed" ? "fault" : resultGone ? "idle" : "pass"}
                  label={resultGone ? "result deleted" : e.status}
                />
                {/* The path sits on the body: a disabled button shows no tooltip. */}
                <div className="job-body" title={e.sourcePath}>
                  <button
                    type="button"
                    /// Bare on purpose: the row is already the affordance.
                    aria-disabled={!openable}
                    /// Nothing to open, so the platform takes it out of the
                    /// tab order and makes it inert.
                    disabled={!openable}
                    className="job-open"
                    onClick={
                      openable
                        ? () => {
                            onOpen(e);
                          }
                        : undefined
                    }
                  >
                    <Meta size="xs" tone="ink" className="job-name">
                      {e.fileName}
                    </Meta>
                  </button>
                  {e.status === "failed" && e.error ? (
                    <Text as="span" size="xs" tone="fault" className="job-sub">
                      {e.error}
                    </Text>
                  ) : (
                    e.outputPath && (
                      <Text
                        as="span"
                        size="xs"
                        tone={resultGone ? "ghost" : "faint"}
                        className="job-sub"
                        title={e.outputPath}
                      >
                        {resultGone
                          ? `${basename(e.outputPath)} (no longer on disk)`
                          : basename(e.outputPath)}
                      </Text>
                    )
                  )}
                </div>
                <Meta size="xs" className="job-time">
                  {fmtWhen(e.finishedAt, nowMs)}
                </Meta>
                <div className="job-actions">
                  {openable && (
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
                    title={hasResult(e) ? "Show the result in Finder" : "Show the file in Finder"}
                    aria-label="Show in Finder"
                    onClick={() => {
                      reveal(e);
                    }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}
    </FlowLayout>
  );
}
