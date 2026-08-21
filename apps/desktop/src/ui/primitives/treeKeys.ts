/// The pure half of the tree's keyboard: given the rows on screen and one key
/// press, which row is next and what happens to it.
///
/// Its own module because a file that exports a component cannot also export
/// this without breaking fast refresh, the same reason `splitLayout.ts` and
/// `pathCrumbs.ts` are separate files.
///
/// Nothing here touches the DOM. `Tree` reads its visible row order off
/// `[data-path]` in its own markup and hands it in as plain data, so the
/// primitive holds no model of the tree and cannot acquire one.

/// How long a type-ahead buffer survives without another key.
export const TYPE_AHEAD_RESET_MS = 1000;

/// One visible row. `Tree` skips the quiet rows ("Empty", "Loading…", "N more
/// files") when it builds these: they are neither selectable nor counted by
/// type-ahead.
export interface TreeRowInfo {
  path: string;
  depth: number;
  /// Undefined on a leaf. A branch reports whether it is open.
  open?: boolean;
  /// The row's label, for type-ahead.
  name: string;
}

/// What one press does. `null` means the key was not ours, and the event is
/// left alone so Tab and Escape still reach the app.
export type TreeAction =
  | { kind: "select"; path: string }
  | { kind: "activate"; path: string }
  | { kind: "inspect"; path: string }
  | { kind: "toggle"; path: string; open: boolean; deep: boolean };

/// The parts of a keyboard event these rules read. A plain shape, so the rules
/// are testable with no DOM.
export interface TreeKeyPress {
  key: string;
  altKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}

const NAVIGATION = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"]);

/// The row that encloses `i`: the nearest earlier row at a shallower depth.
/// -1 at the top level.
export function parentIndex(rows: TreeRowInfo[], i: number): number {
  for (let j = i - 1; j >= 0; j--) {
    if (rows[j].depth < rows[i].depth) return j;
  }
  return -1;
}

/// Finder's key map.
///
/// Both halves of Left and Right are implemented, not just the expand and
/// collapse halves. An arrow that goes dead the moment the row is already in
/// the state the key asks for reads as broken rather than as wrong.
///
/// Arrows move the selection and open nothing: a macOS source list selects as
/// you arrow, and arrowing down a folder that activated every row would open
/// forty tabs. Enter and Cmd+Down activate.
export function treeAction(
  rows: TreeRowInfo[],
  current: string | null,
  press: TreeKeyPress,
): TreeAction | null {
  if (rows.length === 0) return null;

  const i = current === null ? -1 : rows.findIndex((r) => r.path === current);
  // Nothing selected yet. Every navigation key lands on the first row, so the
  // tree answers the first press rather than swallowing it.
  if (i < 0) return NAVIGATION.has(press.key) ? { kind: "select", path: rows[0].path } : null;

  const row = rows[i];
  // A list, not a ring: the ends stop. Selecting the row already selected is
  // not an event, so it reports nothing.
  const select = (j: number): TreeAction | null =>
    j === i || j < 0 || j >= rows.length ? null : { kind: "select", path: rows[j].path };

  switch (press.key) {
    case "ArrowDown":
      return press.metaKey ? { kind: "activate", path: row.path } : select(i + 1);
    case "ArrowUp":
      return press.metaKey ? select(parentIndex(rows, i)) : select(i - 1);
    case "ArrowRight": {
      if (row.open === undefined) return null;
      // Option opens the whole subtree, matching
      // NSOutlineView.expandItem(_:expandChildren:).
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

/// A bare printable character. A modifier makes it a command rather than text,
/// and every named key ("Tab", "F5", "Escape") is longer than one character.
/// Space is excluded because it is the inspect binding.
export function isTypeAheadKey(press: TreeKeyPress): boolean {
  return (
    press.key.length === 1 && press.key !== " " && !press.altKey && !press.metaKey && !press.ctrlKey
  );
}

/// Type-to-select. Case-insensitive prefix over the visible rows, wrapping.
///
/// A one-character buffer starts the search after the current row, so pressing
/// the same letter walks the matches. A longer one starts at the current row,
/// so refining "p" to "pr" can keep the row it already found.
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
