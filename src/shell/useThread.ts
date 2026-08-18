import { useCallback, useEffect, useState } from "react";
import { commands } from "@/app/commands";
import type { HistoryEntry, Job, View } from "@/app/types";
import type { ThreadDoc } from "@/domains/thread/model";
import { copyToClipboard } from "@/platform/host";

/// Opening a result as a reading surface. Two doors lead here — a finished job
/// row, which already holds its text, and a history entry, which does not and
/// has to be read from disk — and both have to remember which view to go back
/// to. That return path is why this is a hook and not two callbacks: `view`
/// and `thread` have to be set together or Escape lands somewhere surprising.
export function useThread({
  view,
  setView,
  showToast,
}: {
  view: View;
  setView: (v: View) => void;
  showToast: (msg: string) => void;
}) {
  const [thread, setThread] = useState<ThreadDoc | null>(null);
  const [threadReturn, setThreadReturn] = useState<Exclude<View, "thread">>("run");

  const closeThread = useCallback(() => {
    setView(threadReturn);
    setThread(null);
  }, [setView, threadReturn]);

  const openJobThread = useCallback(
    (j: Job) => {
      setThreadReturn("run");
      setThread({
        title: j.fileName,
        subtitle: j.outputPath,
        text: j.outputText,
        revealPath: j.outputPath,
      });
      setView("thread");
    },
    [setView],
  );

  const openHistoryThread = useCallback(
    async (entry: HistoryEntry) => {
      if (!entry.outputPath) {
        showToast("No result file to open");
        return;
      }
      try {
        const text = await commands.readTextFile(entry.outputPath);
        setThreadReturn("history");
        setThread({
          title: entry.fileName,
          subtitle: entry.outputPath,
          text,
          revealPath: entry.outputPath,
        });
        setView("thread");
      } catch (e) {
        showToast(String(e));
      }
    },
    [setView, showToast],
  );

  const copyThread = useCallback(async () => {
    if (!thread?.text) return;
    showToast((await copyToClipboard(thread.text)) ? "Copied to clipboard" : "Copy failed");
  }, [thread, showToast]);

  // Escape closes the reader. Bound only while it is open, so it cannot
  // shadow a panel's own Escape handling.
  useEffect(() => {
    if (view !== "thread") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeThread();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [view, closeThread]);

  return { thread, openJobThread, openHistoryThread, closeThread, copyThread };
}
