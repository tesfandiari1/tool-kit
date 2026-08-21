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
  /// Names the tree for assistive technology.
  label: string;
  /// `TreeRow` elements. The tree reads its visible order off `[data-path]` in
  /// its own DOM, so it holds no data model and cannot acquire one.
  children: ReactNode;
  className?: string;
  /// The arrows, type-ahead or a click moved the selection. Nothing opens and
  /// nothing runs: a macOS source list selects as you arrow.
  onSelect: (path: string) => void;
  /// Enter, Cmd+Down, or a click.
  onActivate: (path: string) => void;
  /// Space.
  onInspect: (path: string) => void;
  /// `open` is the state asked for, not the state left behind. `deep` is
  /// Option+click on the twisty and Option+Right: the whole subtree, matching
  /// NSOutlineView.expandItem(_:expandChildren:).
  onToggle: (path: string, open: boolean, deep: boolean) => void;
}

export interface TreeRowProps {
  /// Identity, and the handle every event comes back on. Keyed on a path
  /// rather than an index: a refresh reorders rows, and an index-keyed
  /// selection then lands on a different file with nothing to say so.
  path: string;
  depth: number;
  /// Undefined on a leaf, never false. `aria-expanded="false"` on a file makes
  /// every file in the tree announce as a folder nobody has opened.
  open?: boolean;
  /// Children requested, not arrived. Marks the row busy and, with no `group`
  /// of its own, stands one loading row in for them.
  busy?: boolean;
  selected?: boolean;
  /// Not focusable, not selectable, not counted by type-ahead. The "Empty",
  /// "Loading…" and "N more files" rows.
  quiet?: boolean;
  icon?: ReactNode;
  /// The trailing slot: the paired result's name, a Convert control, or a
  /// reserved ghost. Anything focusable in here belongs at `tabIndex={-1}`, or
  /// Tab through the pane costs two stops a row.
  end?: ReactNode;
  /// The label.
  children: ReactNode;
  /// The rows one level down. Wrapped in the nested `ul[role=group]` here, so
  /// pass it only while the branch is open.
  group?: ReactNode;
  title?: string;
}

/// A disclosure tree with Finder's key map.
///
/// Hand-rolled rather than taken from a library, because what this needs is
/// list navigation and not accessible overlay behaviour, which is the escape
/// hatch UI.md actually names. The keyboard rules live in `treeKeys.ts` and are
/// tested there.
///
/// Rows are `li` and `span`, never `<button>`: a button's own Space and Enter
/// semantics would fight the inspect binding, and a control in the trailing
/// slot would nest one button inside another.
///
/// The caller owns expansion, selection and the data. This owns the keyboard,
/// the focus, and the shape.
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

  /// The rows on screen, in the order they are drawn, read from the tree's own
  /// markup. The same mechanism the run queue uses for its roving tabindex.
  /// Quiet rows are skipped: they answer no key and hold no selection.
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

  /// Selection and focus move together, which is what makes Left and Right
  /// keep working after they land. The row is already on screen, so this does
  /// not wait for the caller's re-render.
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

  /// One stop in the tab order. The selected row is the way back in, and with
  /// nothing selected it is the first row — which a row cannot decide for
  /// itself, so it is decided here. No dependency list on purpose: every
  /// render can add, drop or reorder rows.
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
        // A control in the trailing slot answers its own keys.
        if (e.target instanceof HTMLElement && e.target.closest(".ui-tree__end")) return;
        const rows = rowInfo(rowEls());
        const current = selectedPath();
        const action = treeAction(rows, current, e);
        if (action) {
          // Space would scroll the pane out from under the row, and the arrows
          // would scroll it out from under the selection.
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
        if (e.target.closest(".ui-tree__end")) return;
        // Bail on the quiet row itself, before looking for a path. The loading
        // row carries none, so the search would walk past it to the branch that
        // owns it and activate a folder the user did not click.
        if (e.target.closest("[data-quiet]")) return;
        const row = e.target.closest<HTMLElement>("[data-path]");
        const path = row?.dataset.path;
        if (!row || path === undefined) return;
        if (e.target.closest(".ui-tree__twisty")) {
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
  const body = group ?? (busy && open === true ? loadingRow(depth + 1) : undefined);
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
        {/* Reserved on leaves as well as branches, or sibling labels sit out of
            line and the whole column reads as broken. */}
        <span className={cx("ui-tree__twisty", open !== undefined && "is-branch")} aria-hidden />
        {icon !== undefined && (
          <span className="ui-tree__icon" aria-hidden>
            {icon}
          </span>
        )}
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

/// Stands in for children that were asked for and have not arrived. Written
/// here rather than left to the caller so an expanded branch is never an empty
/// group, which is the one shape the tree pattern has no answer for.
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
