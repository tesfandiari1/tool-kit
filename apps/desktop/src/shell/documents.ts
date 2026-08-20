import type { OpenDoc } from "@/domains/thread/model";

/// The open-document list and which one the pane is showing. They live in one
/// value because they are only ever correct together: activating a document
/// that is not in the list, or dropping the one the pane is drawing, is a blank
/// pane with tabs above it.
export interface DocList {
  docs: OpenDoc[];
  activeId: string | null;
}

export const NO_DOCS: DocList = { docs: [], activeId: null };

/// Open a document, or activate it if it is already open. The id is the output
/// path, so the same result reached from the queue and from the history is one
/// document with one tab — not two tabs that can disagree about its text.
export function activateOrInsert(docs: OpenDoc[], doc: OpenDoc): DocList {
  if (docs.some((d) => d.id === doc.id)) return { docs, activeId: doc.id };
  return { docs: [...docs, doc], activeId: doc.id };
}

/// Close a document and decide what the pane shows next. Closing the active
/// one hands the slot to the tab that took its place, or to the one before it
/// when it was last, so the pane only falls back to its empty state when
/// nothing is left open.
export function removeDoc(docs: OpenDoc[], activeId: string | null, id: string): DocList {
  const at = docs.findIndex((d) => d.id === id);
  if (at < 0) return { docs, activeId };
  const rest = docs.filter((d) => d.id !== id);
  if (rest.length === 0) return NO_DOCS;
  if (activeId !== id) return { docs: rest, activeId };
  return { docs: rest, activeId: rest[Math.min(at, rest.length - 1)].id };
}
