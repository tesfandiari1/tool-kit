import { basename } from "@/app/format";
import type { OpenDoc } from "@/domains/thread/model";

/// The open documents and the active one. Correct only together: an active id
/// outside the list is a blank pane with tabs above it.
export interface DocList {
  docs: OpenDoc[];
  activeId: string | null;
}

export const NO_DOCS: DocList = { docs: [], activeId: null };

/// Open, or activate what is open. The id is the path, so two doors give one
/// tab rather than two that disagree about its text.
export function activateOrInsert(docs: OpenDoc[], doc: OpenDoc): DocList {
  if (docs.some((d) => d.id === doc.id)) return { docs, activeId: doc.id };
  return { docs: [...docs, doc], activeId: doc.id };
}

/// Close, and hand the slot to the tab that takes its place.
export function removeDoc(docs: OpenDoc[], activeId: string | null, id: string): DocList {
  const at = docs.findIndex((d) => d.id === id);
  if (at < 0) return { docs, activeId };
  const rest = docs.filter((d) => d.id !== id);
  if (rest.length === 0) return NO_DOCS;
  if (activeId !== id) return { docs: rest, activeId };
  return { docs: rest, activeId: rest[Math.min(at, rest.length - 1)].id };
}

/// Follow a file the host moved: every field is the path taken at open time,
/// so a stale tab goes on saving to the folder the file left.
export function renameDoc(
  docs: OpenDoc[],
  activeId: string | null,
  from: string,
  to: string,
): DocList {
  if (!docs.some((d) => d.id === from)) return { docs, activeId };
  return {
    activeId: activeId === from ? to : activeId,
    docs: docs.map((d) =>
      d.id === from ? { ...d, id: to, title: basename(to), subtitle: to, revealPath: to } : d,
    ),
  };
}
