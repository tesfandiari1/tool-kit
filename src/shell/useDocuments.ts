import { useCallback, useState } from "react";
import { commands } from "@/app/commands";
import type { HistoryEntry, Job } from "@/app/types";
import { isDirty, type DocMode, type OpenDoc, type SaveState } from "@/domains/thread/model";
import { confirm } from "@/platform/host";
import { activateOrInsert, NO_DOCS, removeDoc } from "./documents";

/// Every result the user has opened, and which one the document pane is
/// showing. Documents are a list beside the queue, not a view that replaces it,
/// so nothing here touches `view`: reading a result and changing a setting are
/// independent, and one must not close the other.
///
/// Both doors — a finished job row and a history entry — read the file from
/// disk. `Job.outputText` is the same bytes only until the first edit is saved,
/// and a pane that opens the stale copy would show the user their own work
/// missing.
export function useDocuments({ showToast }: { showToast: (msg: string) => void }) {
  const [{ docs, activeId }, setList] = useState(NO_DOCS);
  const [mode, setMode] = useState<DocMode>("read");

  const open = useCallback(
    async (title: string, outputPath: string | null) => {
      if (!outputPath) {
        showToast("No result file to open");
        return;
      }
      // An open document is already the file on disk, plus any unsaved edit.
      // Re-reading it here would throw that edit away to learn nothing.
      if (docs.some((d) => d.id === outputPath)) {
        setList((cur) => ({ ...cur, activeId: outputPath }));
        return;
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
      } catch (e) {
        showToast(String(e));
      }
    },
    [docs, showToast],
  );

  const openJob = useCallback((job: Job) => open(job.fileName, job.outputPath), [open]);

  const openHistory = useCallback(
    (entry: HistoryEntry) => open(entry.fileName, entry.outputPath),
    [open],
  );

  const select = useCallback((id: string) => {
    setList((cur) => ({ ...cur, activeId: id }));
  }, []);

  /// Reports whether the document closed, so a caller closing several (or
  /// quitting) can stop at the one the user kept.
  ///
  /// A save the host refused leaves the state `error`, not `edited`, and the
  /// edit is still only in memory — so that asks too. See `isDirty`.
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

  /// Only a real change marks the document dirty. The editor reports its value
  /// on mount as well as on a keystroke, and a document that goes dirty by
  /// being looked at would make the unsaved-changes prompt meaningless.
  const edit = useCallback((id: string, text: string) => {
    setList((cur) => ({
      ...cur,
      docs: cur.docs.map((d) =>
        d.id === id && d.text !== text ? { ...d, text, save: "edited" } : d,
      ),
    }));
  }, []);

  /// What a save reports back: the state, and the mtime the write produced,
  /// which the next save has to offer as the expected one.
  const setDocMeta = useCallback(
    (id: string, patch: Partial<Pick<OpenDoc, "text" | "save" | "mtimeMs">>) => {
      setList((cur) => ({
        ...cur,
        docs: cur.docs.map((d) => (d.id === id ? { ...d, ...patch } : d)),
      }));
    },
    [],
  );

  const setSave = useCallback(
    (id: string, save: SaveState) => {
      setDocMeta(id, { save });
    },
    [setDocMeta],
  );

  return {
    docs,
    activeId,
    mode,
    openJob,
    openHistory,
    select,
    closeDoc,
    edit,
    setMode,
    setSave,
    setDocMeta,
  };
}
