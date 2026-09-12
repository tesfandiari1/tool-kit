/// The pure half of the tree's keyboard. Its own module, or fast refresh
/// breaks. Nothing here touches the DOM: `Tree` hands the row order in as
/// plain data, so the primitive holds no model of the tree.

/// How long a type-ahead buffer survives without another key.
export const TYPE_AHEAD_RESET_MS = 1000;

/// One visible row. `Tree` skips the quiet ones.
export interface TreeRowInfo {
  path: string;
  depth: number;
  /// Undefined on a leaf. A branch reports whether it is open.
  open?: boolean;
  /// The row's label, for type-ahead.
  name: string;
}

/// `null` leaves the event alone, so Tab and Escape still reach the app.
export type TreeAction =
  | { kind: "select"; path: string }
  | { kind: "activate"; path: string }
  | { kind: "inspect"; path: string }
  | { kind: "toggle"; path: string; open: boolean; deep: boolean };

/// A plain shape, so the rules are testable with no DOM.
export interface TreeKeyPress {
  key: string;
  altKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}

const NAVIGATION = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"]);

/// The nearest earlier row at a shallower depth. -1 at the top level.
export function parentIndex(rows: TreeRowInfo[], i: number): number {
  for (let j = i - 1; j >= 0; j--) {
    if (rows[j].depth < rows[i].depth) return j;
  }
  return -1;
}

/// Finder's key map. Both halves of Left and Right, or an arrow goes dead on
/// a row already in the state it asks for. Arrows only select, because
/// arrowing down an activating list would open forty tabs.
export function treeAction(
  rows: TreeRowInfo[],
  current: string | null,
  press: TreeKeyPress,
): TreeAction | null {
  if (rows.length === 0) return null;

  const i = current === null ? -1 : rows.findIndex((r) => r.path === current);
  // Nothing selected, so every navigation key lands on the first row.
  if (i < 0) return NAVIGATION.has(press.key) ? { kind: "select", path: rows[0].path } : null;

  const row = rows[i];
  // A list, not a ring, and re-selecting the selected row is not an event.
  const select = (j: number): TreeAction | null =>
    j === i || j < 0 || j >= rows.length ? null : { kind: "select", path: rows[j].path };

  switch (press.key) {
    case "ArrowDown":
      return press.metaKey ? { kind: "activate", path: row.path } : select(i + 1);
    case "ArrowUp":
      return press.metaKey ? select(parentIndex(rows, i)) : select(i - 1);
    case "ArrowRight": {
      if (row.open === undefined) return null;
      // Option opens the subtree, as NSOutlineView.expandItem(_:expandChildren:).
      if (press.altKey) return { kind: "toggle", path: row.path, open: true, deep: true };
      if (!row.open) return { kind: "toggle", path: row.path, open: true, deep: false };
      const child = i + 1;
      return child < rows.length && rows[child].depth > row.depth ? select(child) : null;
    }
    case "ArrowLeft":
      if (row.open !== undefined && press.altKey) {
        return { kind: "toggle", path: row.path, open: false, deep: true };
      }
      if (row.open === true) return { kind: "toggle", path: row.path, open: false, deep: false };
      return select(parentIndex(rows, i));
    case "Home":
      return select(0);
    case "End":
      return select(rows.length - 1);
    case "Enter":
      return { kind: "activate", path: row.path };
    case " ":
      return { kind: "inspect", path: row.path };
    default:
      return null;
  }
}

/// A bare printable character. Space is the inspect binding, not type-ahead.
export function isTypeAheadKey(press: TreeKeyPress): boolean {
  return (
    press.key.length === 1 && press.key !== " " && !press.altKey && !press.metaKey && !press.ctrlKey
  );
}

/// Type-to-select, wrapping. One character starts after the current row so the
/// letter walks matches, and a longer buffer starts at it.
export function typeAheadTarget(
  rows: TreeRowInfo[],
  current: string | null,
  prefix: string,
): string | null {
  if (rows.length === 0 || prefix === "") return null;
  const needle = prefix.toLowerCase();
  const i = current === null ? -1 : rows.findIndex((r) => r.path === current);
  const from = i + (prefix.length > 1 ? 0 : 1);
  for (let n = 0; n < rows.length; n++) {
    const row = rows[(((from + n) % rows.length) + rows.length) % rows.length];
    if (row.name.toLowerCase().startsWith(needle)) return row.path;
  }
  return null;
}
