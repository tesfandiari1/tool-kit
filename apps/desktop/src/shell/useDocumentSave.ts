import { useCallback, useEffect, useRef } from "react";
import { commands } from "@/app/commands";
import { isDirty, type OpenDoc } from "@/domains/thread/model";

/// How long after the last keystroke the document goes to disk. Long enough
/// that a burst of typing is one write, short enough that the user never has
/// to think about saving.
const AUTOSAVE_MS = 800;

/// Getting an edit onto disk, by every route the user expects: on a debounce
/// while they type, on ⌘S, and before the document closes.
///
/// Saving is separate from `useDocuments` because it is the only part of the
/// document story that talks to the host. The list hook owns what is open and
/// what has changed; this one owns the write and the mtime the next write has
/// to offer back.
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
  /// A save reads the document at the moment it runs, not at the moment its
  /// caller was created: a debounce timer and a keydown handler both outlive
  /// the render that made them, and writing a render-old `text` would put the
  /// user's second-to-last keystroke on disk.
  const docsRef = useRef(docs);
  useEffect(() => {
    docsRef.current = docs;
  }, [docs]);

  /// Reports whether the document is now on disk, so a caller that is about to
  /// close it or leave it can stop at the one that refused.
  const saveDoc = useCallback(
    async (id: string): Promise<boolean> => {
      const doc = docsRef.current.find((d) => d.id === id);
      // Nothing to write, or a write already in flight — `saving` reads as
      // clean here, which is what stops ⌘S from racing the autosave timer.
      if (!doc || !isDirty(doc.save)) return true;
      setDocMeta(id, { save: "saving" });
      try {
        const mtimeMs = await commands.writeDocument(doc.id, doc.text, doc.mtimeMs);
        // Typing during the write leaves newer text in the document than the
        // bytes that landed. Take the new mtime regardless — the next save
        // has to offer it or the host refuses a write we caused ourselves —
        // but leave the state dirty so the debounce comes back for the rest.
        const now = docsRef.current.find((d) => d.id === id);
        setDocMeta(id, now?.text === doc.text ? { save: "saved", mtimeMs } : { mtimeMs });
        return true;
      } catch {
        // The reason is the host's — almost always the file changing under us.
        // `error` is the red dot and "Could not save" in the header; a toast
        // would say the same thing somewhere the document is not.
        setDocMeta(id, { save: "error" });
        return false;
      }
    },
    [setDocMeta],
  );

  // Autosave. `docs` gets a new identity on every keystroke, so this effect's
  // cleanup cancels the pending timer and the next one starts over: that is
  // the debounce, with no timer bookkeeping of its own.
  //
  // Only `edited`, never `error`: a write the host refused will be refused
  // again for the same reason, and retrying it every 800ms would be an
  // unkillable loop against the disk behind a red dot the user cannot clear.
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

  // ⌘S saves what you are looking at. Bound only while a document is open, so
  // an app showing the launcher never swallows the key.
  //
  // Matched on `code`, the physical key, rather than `key`, the character it
  // produced: with Caps Lock on or Shift held `e.key` is "S", and comparing to
  // "s" silently drops the shortcut exactly for the user typing in caps.
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

  /// Close, saving first. Reports whether the document actually closed.
  ///
  /// A refused save stops the close: the red dot and its reason are worth
  /// seeing before the tab goes. Asking again closes it — by then the state is
  /// `error`, so this skips the doomed retry and hands over to `closeDoc`,
  /// whose unsaved-changes confirm is the way out. Without that step a file
  /// changed on disk would be a tab that can never be closed.
  const requestClose = useCallback(
    async (id: string): Promise<boolean> => {
      const doc = docsRef.current.find((d) => d.id === id);
      if (doc?.save === "edited" && !(await saveDoc(id))) return false;
      return closeDoc(id);
    },
    [closeDoc, saveDoc],
  );

  return { saveDoc, requestClose };
}
