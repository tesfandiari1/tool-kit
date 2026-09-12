import { useCallback, useEffect, useRef } from "react";
import { commands } from "@/app/commands";
import { isDirty, type OpenDoc } from "@/domains/thread/model";

/// A burst of typing is one write.
const AUTOSAVE_MS = 800;

/// Every route an edit takes to disk: the debounce, ⌘S and the close. This
/// owns the write and the mtime the next one has to offer back.
export function useDocumentSave({
  docs,
  activeId,
  closeDoc,
  setDocMeta,
}: {
  docs: OpenDoc[];
  activeId: string | null;
  closeDoc: (id: string) => Promise<boolean>;
  setDocMeta: (
    id: string,
    patch: Partial<Pick<OpenDoc, "text" | "save" | "mtimeMs">>,
  ) => void;
}) {
  /// A debounce timer outlives its render, and a render-old `text` puts the
  /// second-to-last keystroke on disk.
  const docsRef = useRef(docs);
  useEffect(() => {
    docsRef.current = docs;
  }, [docs]);

  /// The write each document has in flight, so a close can wait for it.
  const writes = useRef(new Map<string, Promise<boolean>>());

  /// Reports whether the document is on disk, so a caller can stop.
  const saveDoc = useCallback(
    (id: string): Promise<boolean> => {
      const write = (async () => {
        const doc = docsRef.current.find((d) => d.id === id);
        // `saving` reads as clean, which stops ⌘S racing the autosave.
        if (!doc || !isDirty(doc.save)) return true;
        setDocMeta(id, { save: "saving" });
        try {
          const mtimeMs = await commands.writeDocument(doc.id, doc.text, doc.mtimeMs);
          // Take the new mtime even when typing outran the write, or the next
          // save offers a stale one and the host refuses it.
          const now = docsRef.current.find((d) => d.id === id);
          setDocMeta(id, now?.text === doc.text ? { save: "saved", mtimeMs } : { mtimeMs });
          // The ref lags the render, and a close that waited on this write
          // saves again inside that gap.
          docsRef.current = docsRef.current.map((d) => (d.id === id ? { ...d, mtimeMs } : d));
          return true;
        } catch {
          // The header says it, where a toast would not.
          setDocMeta(id, { save: "error" });
          return false;
        }
      })();
      writes.current.set(id, write);
      void write.finally(() => {
        // Only this one: a later save already replaced the entry.
        if (writes.current.get(id) === write) writes.current.delete(id);
      });
      return write;
    },
    [setDocMeta],
  );

  // `docs` changes identity on every keystroke, so the cleanup is the debounce.
  // Only `edited`, never `error`: a refused write is refused again, and
  // retrying every 800ms is a loop the user cannot clear.
  useEffect(() => {
    const pending = docs.filter((d) => d.save === "edited").map((d) => d.id);
    if (pending.length === 0) return;
    const t = window.setTimeout(() => {
      for (const id of pending) void saveDoc(id);
    }, AUTOSAVE_MS);
    return () => {
      window.clearTimeout(t);
    };
  }, [docs, saveDoc]);

  // Matched on `code`: with Caps Lock on, `e.key` is "S" and a `key` test
  // drops the shortcut for the user typing in caps.
  useEffect(() => {
    if (!activeId) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== "KeyS" || e.altKey || !(e.metaKey || e.ctrlKey)) return;
      e.preventDefault();
      void saveDoc(activeId);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [activeId, saveDoc]);

  /// Close, saving first, and report whether it closed. A write already in
  /// flight is waited on, or an edit that fails after the tab closed is lost
  /// silently. A refused save stops the first attempt only: the second skips
  /// the doomed retry and hands over to `closeDoc`, or a file changed on disk
  /// is a tab that never closes.
  const requestClose = useCallback(
    async (id: string): Promise<boolean> => {
      const pending = writes.current.get(id);
      if (pending && !(await pending)) return false;
      const doc = docsRef.current.find((d) => d.id === id);
      if (doc?.save === "edited" && !(await saveDoc(id))) return false;
      return closeDoc(id);
    },
    [closeDoc, saveDoc],
  );

  return { saveDoc, requestClose };
}
