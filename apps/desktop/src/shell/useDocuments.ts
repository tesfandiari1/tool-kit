import { useCallback, useState } from "react";
import { commands } from "@/app/commands";
import { basename } from "@/app/format";
import type { FileRow, HistoryEntry, Job } from "@/app/types";
import { isDirty, type DocMode, type OpenDoc, type SaveState } from "@/domains/thread/model";
import { confirm } from "@/platform/host";
import { activateOrInsert, NO_DOCS, removeDoc } from "./documents";

/// Every result the user has opened, and which one the document pane is
/// showing. Documents are a list beside the queue, not a view that replaces it,
/// so nothing here touches `view`: reading a result and changing a setting are
/// independent, and one must not close the other.
///
/// Both doors — a finished job row and a history entry — read the file from
/// disk, which is the only copy there is: the job row carries a path, not the
/// converted text.
export function useDocuments({ showToast }: { showToast: (msg: string) => void }) {
  const [{ docs, activeId }, setList] = useState(NO_DOCS);
  const [mode, setMode] = useState<DocMode>("read");
  /// The file the inspector card is reporting on, layered over the pane. It
  /// lives here because this hook already owns what the pane shows, which makes
  /// "opening a document dismisses the card" an invariant of the pane's owner
  /// rather than three call sites that have to remember.
  const [preview, setPreview] = useState<FileRow | null>(null);

  /// Reports whether the pane ended up showing the file. A caller with somewhere
  /// else to put it can then act: the library falls back to the inspector card,
  /// because `openable` is decided from the extension and the size and only a
  /// decode can answer for the encoding.
  const open = useCallback(
    async (title: string, outputPath: string | null): Promise<boolean> => {
      if (!outputPath) {
        showToast("No result file to open");
        return false;
      }
      setPreview(null);
      // An open document is already the file on disk, plus any unsaved edit.
      // Re-reading it here would throw that edit away to learn nothing.
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

  /// The library tree's door. The doc id is the path and `activateOrInsert`
  /// dedupes on it, so a file opened from the tree, a job row and the history
  /// is one tab, with the autosave, ⌘S, the mtime handshake and the close
  /// confirm all coming free.
  const openPath = useCallback((path: string) => open(basename(path), path), [open]);

  const select = useCallback((id: string) => {
    setPreview(null);
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
    preview,
    showPreview: setPreview,
    openJob,
    openHistory,
    openPath,
    select,
    closeDoc,
    edit,
    setMode,
    setSave,
    setDocMeta,
  };
}
