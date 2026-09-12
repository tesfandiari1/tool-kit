import { useEffect, useState } from "react";
import { EyeIcon, FolderOpenIcon } from "@phosphor-icons/react";
import { Badge, Button, Input, Mono, Row, Spacer, StatusDot, Text } from "@ui";
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
}: {
  refreshKey: number;
  /// Call after any change, or the run screen cites deleted rows.
  onChanged: () => void;
  onOpen: (entry: HistoryEntry) => void;
  onToast: (msg: string) => void;
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
      setRows([]);
      // Or Run stays disabled citing a history that no longer exists.
      onChanged();
      onToast("History cleared");
    } catch (e) {
      onToast(String(e));
    }
  };

  // The result, or the source it came from: the answer for a failed row.
  const reveal = (e: HistoryEntry) => {
    void commands.revealPath(hasResult(e) ? e.outputPath : e.sourcePath).catch((err: unknown) => {
      onToast(String(err));
    });
  };

  return (
    <FlowLayout
      className="history-panel"
      head={
        <>
          <Row gap={2}>
            <Spacer />
            <Badge square>
              {rows === null
                ? ""
                : rows.length >= HISTORY_LIMIT
                  ? `${String(rows.length)}+`
                  : String(rows.length)}
            </Badge>
            <Button
              variant="link"
              /* `clear_history` deletes the whole table, so a search matching
                 nothing must not disable it. */
              disabled={!rows || (rows.length === 0 && query === "")}
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
      {rows === null ? (
        <div className="hist-empty">
          <Text size="sm" tone="faint">
            Reading…
          </Text>
        </div>
      ) : rows.length === 0 ? (
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
          {rows.map((e) => {
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
                <div className="job-body">
                  <button
                    type="button"
                    /// Bare on purpose: the row is already the affordance.
                    aria-disabled={!openable}
                    className="job-open"
                    title={e.sourcePath}
                    onClick={
                      openable
                        ? () => {
                            onOpen(e);
                          }
                        : undefined
                    }
                  >
                    <Mono size="xs" tone="ink" className="job-name">
                      {e.fileName}
                    </Mono>
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
                <Mono size="xs" className="job-time">
                  {fmtWhen(e.finishedAt, nowMs)}
                </Mono>
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
