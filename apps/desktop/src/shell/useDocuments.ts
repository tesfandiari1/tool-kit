import { useCallback, useState } from "react";
import { commands } from "@/app/commands";
import { basename } from "@/app/format";
import type { FileRow, HistoryEntry, Job } from "@/app/types";
import { isDirty, type DocMode, type OpenDoc } from "@/domains/thread/model";
import { confirm } from "@/platform/host";
import { activateOrInsert, NO_DOCS, removeDoc, renameDoc } from "./documents";

/// Every open result and which one the pane shows. Nothing here touches
/// `view`. Every door reads the file from disk: a job row carries a path,
/// never the converted text.
export function useDocuments({ showToast }: { showToast: (msg: string) => void }) {
  const [{ docs, activeId }, setList] = useState(NO_DOCS);
  const [mode, setMode] = useState<DocMode>("read");
  /// The inspector card's file. Here, so "opening a document dismisses the
  /// card" is an invariant rather than three call sites.
  const [preview, setPreview] = useState<FileRow | null>(null);

  /// Reports whether the pane showed the file: `openable` cannot answer for
  /// the encoding, so a caller needs somewhere else to put it.
  const open = useCallback(
    async (title: string, outputPath: string | null): Promise<boolean> => {
      if (!outputPath) {
        showToast("No result file to open");
        return false;
      }
      setPreview(null);
      // Re-reading an open document throws its edit away to learn nothing.
      if (docs.some((d) => d.id === outputPath)) {
        setList((cur) => ({ ...cur, activeId: outputPath }));
        return true;
      }
      try {
        const { text, mtimeMs } = await commands.readDocument(outputPath);
        const doc: OpenDoc = {
          id: outputPath,
          title,
          subtitle: outputPath,
          text,
          revealPath: outputPath,
          save: "clean",
          mtimeMs,
        };
        setList((cur) => activateOrInsert(cur.docs, doc));
        return true;
      } catch (e) {
        showToast(String(e));
        return false;
      }
    },
    [docs, showToast],
  );

  const openJob = useCallback((job: Job) => open(job.fileName, job.outputPath), [open]);

  const openHistory = useCallback(
    (entry: HistoryEntry) => open(entry.fileName, entry.outputPath),
    [open],
  );

  /// The doc id is the path, so three doors open one tab.
  const openPath = useCallback((path: string) => open(basename(path), path), [open]);

  const select = useCallback((id: string) => {
    setPreview(null);
    setList((cur) => ({ ...cur, activeId: id }));
  }, []);

  /// Reports whether it closed, so a caller closing several can stop. A
  /// refused save reads `error`, and that edit is still in memory.
  const closeDoc = useCallback(
    async (id: string): Promise<boolean> => {
      const doc = docs.find((d) => d.id === id);
      if (doc && isDirty(doc.save)) {
        const ok = await confirm("This document has unsaved changes. Close anyway?", {
          title: "Unsaved changes",
          kind: "warning",
          okLabel: "Close",
          cancelLabel: "Keep editing",
        });
        if (!ok) return false;
      }
      setList((cur) => removeDoc(cur.docs, cur.activeId, id));
      return true;
    },
    [docs],
  );

  /// Follow a file the host moved, tab and autosave with it.
  const rename = useCallback((from: string, to: string) => {
    setList((cur) => renameDoc(cur.docs, cur.activeId, from, to));
  }, []);

  /// Only a real change marks it dirty: the editor reports on mount too, and
  /// a document that dirties by being read makes the prompt meaningless.
  const edit = useCallback((id: string, text: string) => {
    setList((cur) => ({
      ...cur,
      docs: cur.docs.map((d) =>
        d.id === id && d.text !== text ? { ...d, text, save: "edited" } : d,
      ),
    }));
  }, []);

  /// The state, and the mtime the next save has to offer back.
  const setDocMeta = useCallback(
    (id: string, patch: Partial<Pick<OpenDoc, "text" | "save" | "mtimeMs">>) => {
      setList((cur) => ({
        ...cur,
        docs: cur.docs.map((d) => (d.id === id ? { ...d, ...patch } : d)),
      }));
    },
    [],
  );

  return {
    docs,
    activeId,
    mode,
    preview,
    showPreview: setPreview,
    openJob,
    openHistory,
    openPath,
    select,
    closeDoc,
    edit,
    renameDoc: rename,
    setMode,
    setDocMeta,
  };
}
