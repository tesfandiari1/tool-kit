import { useEffect, useRef, type CSSProperties, type ReactNode } from "react";
import { cx } from "../cx";
import {
  TYPE_AHEAD_RESET_MS,
  isTypeAheadKey,
  treeAction,
  typeAheadTarget,
  type TreeAction,
  type TreeRowInfo,
} from "./treeKeys";
import "./Tree.css";

export interface TreeProps {
  label: string;
  /// `TreeRow` elements. Their order is read off `[data-path]`, so this holds
  /// no data model.
  children: ReactNode;
  className?: string;
  /// Nothing opens and nothing runs: a source list selects as you arrow.
  onSelect: (path: string) => void;
  onActivate: (path: string) => void;
  onInspect: (path: string) => void;
  /// `open` is the state asked for. `deep` is Option: the whole subtree.
  onToggle: (path: string, open: boolean, deep: boolean) => void;
}

export interface TreeRowProps {
  /// Identity, and the handle every event comes back on. A path, never an
  /// index: a refresh reorders rows.
  path: string;
  depth: number;
  /// Undefined on a leaf, never false: `aria-expanded="false"` announces a
  /// file as an unopened folder.
  open?: boolean;
  /// Requested, not arrived. An open branch with no `group` gets a loading
  /// row either way.
  busy?: boolean;
  selected?: boolean;
  /// Not focusable, selectable or counted by type-ahead.
  quiet?: boolean;
  icon?: ReactNode;
  /// The trailing slot. Anything focusable in here belongs at `tabIndex={-1}`,
  /// or Tab costs two stops a row.
  end?: ReactNode;
  children: ReactNode;
  /// Wrapped in the nested `ul[role=group]`, so pass it only while open.
  group?: ReactNode;
  title?: string;
}

/// A disclosure tree with Finder's key map, hand-rolled per UI.md. The rules
/// live in `treeKeys.ts`.
///
/// Rows are `li` and `span`, never `<button>`: a button's Space and Enter
/// fight the inspect binding, and the trailing slot would nest one button in
/// another. The caller owns expansion, selection and data.
export function Tree({
  label,
  children,
  className,
  onSelect,
  onActivate,
  onInspect,
  onToggle,
}: TreeProps) {
  const ref = useRef<HTMLUListElement>(null);
  const typed = useRef({ buffer: "", at: 0 });

  /// The rows on screen, in draw order. Quiet rows answer no key.
  const rowEls = () =>
    Array.from(ref.current?.querySelectorAll<HTMLElement>("[data-path]:not([data-quiet])") ?? []);

  const rowInfo = (els: HTMLElement[]): TreeRowInfo[] =>
    els.map((el) => {
      const expanded = el.getAttribute("aria-expanded");
      return {
        path: el.dataset.path ?? "",
        depth: Number(el.dataset.depth ?? "0"),
        open: expanded === null ? undefined : expanded === "true",
        name: el.querySelector(".ui-tree__name")?.textContent ?? "",
      };
    });

  const selectedPath = () =>
    ref.current?.querySelector<HTMLElement>('[data-path][aria-selected="true"]')?.dataset.path ??
    null;

  /// Selection and focus move together, without waiting for a re-render.
  const focusRow = (path: string) => {
    ref.current?.querySelector<HTMLElement>(`[data-path="${CSS.escape(path)}"]`)?.focus();
  };

  const apply = (action: TreeAction) => {
    switch (action.kind) {
      case "select":
        onSelect(action.path);
        focusRow(action.path);
        break;
      case "activate":
        onActivate(action.path);
        break;
      case "inspect":
        onInspect(action.path);
        break;
      case "toggle":
        onToggle(action.path, action.open, action.deep);
        break;
    }
  };

  /// One stop in the tab order, which a row cannot decide for itself. No
  /// dependency list: every render can add, drop or reorder rows.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rows = rowEls();
    const stop = el.querySelector<HTMLElement>('[data-path][aria-selected="true"]') ?? rows[0];
    for (const row of rows) row.tabIndex = row === stop ? 0 : -1;
  });

  return (
    <ul
      ref={ref}
      role="tree"
      aria-label={label}
      className={cx("ui-tree", className)}
      onKeyDown={(e) => {
        if (e.target instanceof HTMLElement && e.target.closest(".ui-tree__end")) return;
        const rows = rowInfo(rowEls());
        const current = selectedPath();
        const action = treeAction(rows, current, e);
        if (action) {
          // Space and the arrows scroll the pane out from under the row.
          e.preventDefault();
          apply(action);
          return;
        }
        if (!isTypeAheadKey(e)) return;
        e.preventDefault();
        const now = Date.now();
        const buffer = now - typed.current.at > TYPE_AHEAD_RESET_MS ? e.key : typed.current.buffer + e.key;
        typed.current = { buffer, at: now };
        const hit = typeAheadTarget(rows, current, buffer);
        if (hit !== null) apply({ kind: "select", path: hit });
      }}
      onClick={(e) => {
        if (!(e.target instanceof HTMLElement)) return;
        // The slot also holds inert labels, and bailing on those makes a dead
        // strip on the row.
        if (e.target.closest(".ui-tree__end :is(button, a, input)")) return;
        // Bail before looking for a path: a quiet row carries none, so the
        // search would climb to the branch and activate it.
        if (e.target.closest("[data-quiet]")) return;
        const row = e.target.closest<HTMLElement>("[data-path]");
        const path = row?.dataset.path;
        if (!row || path === undefined) return;
        // Branches only: the slot is reserved on leaves, and unqualified the
        // blank box would ask the host to list a file.
        if (e.target.closest(".ui-tree__twisty.is-branch")) {
          onToggle(path, row.getAttribute("aria-expanded") !== "true", e.altKey);
          return;
        }
        onSelect(path);
        focusRow(path);
        onActivate(path);
      }}
    >
      {children}
    </ul>
  );
}

export function TreeRow({
  path,
  depth,
  open,
  busy = false,
  selected = false,
  quiet = false,
  icon,
  end,
  children,
  group,
  title,
}: TreeRowProps) {
  // `aria-expanded="true"` over nothing is the one shape the tree pattern has
  // no answer for, and a failed read never sends a group.
  const body = group ?? (open === true ? loadingRow(depth + 1) : undefined);
  return (
    <li
      role="treeitem"
      data-path={path}
      data-depth={depth}
      data-quiet={quiet ? "" : undefined}
      aria-expanded={open}
      aria-selected={quiet ? undefined : selected}
      aria-disabled={quiet || undefined}
      aria-busy={busy || undefined}
      tabIndex={quiet ? undefined : selected ? 0 : -1}
      title={title}
      className={cx("ui-tree__item", selected && "is-selected", quiet && "is-quiet")}
      style={{ "--tree-depth": depth } as CSSProperties}
    >
      <span className="ui-tree__row">
        {/* Reserved on leaves as well as branches, or sibling labels sit out
            of line. */}
        <span className={cx("ui-tree__twisty", open !== undefined && "is-branch")} aria-hidden />
        {/* Not hidden: the slot carries a status dot mid-run, which names
            itself. A file glyph in here has no accessible name to leak. */}
        {icon !== undefined && <span className="ui-tree__icon">{icon}</span>}
        <span className="ui-tree__name">{children}</span>
        {end !== undefined && <span className="ui-tree__end">{end}</span>}
      </span>
      {body !== undefined && (
        <ul role="group" className="ui-tree__group">
          {body}
        </ul>
      )}
    </li>
  );
}

/// Stands in for children not yet arrived, so no branch is an empty group.
function loadingRow(depth: number) {
  return (
    <li
      role="treeitem"
      data-quiet=""
      aria-disabled
      className="ui-tree__item is-quiet"
      style={{ "--tree-depth": depth } as CSSProperties}
    >
      <span className="ui-tree__row">
        <span className="ui-tree__twisty" aria-hidden />
        <span className="ui-tree__name">Loading…</span>
      </span>
    </li>
  );
}
