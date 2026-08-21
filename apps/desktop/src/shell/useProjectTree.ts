import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands } from "@/app/commands";
import type { DirListing, ProjectSummary } from "@/app/types";

/// How far one Option-click descends. `NSOutlineView` opens the whole subtree,
/// but a workspace can hold a folder nobody meant to walk, and one click that
/// fires a thousand `read_dir` calls is a frozen tree with nothing on screen to
/// explain it.
const DEEP_EXPAND_DEPTH = 6;

export interface ProjectTreeState {
  /// One directory level per workspace-relative path. Undefined until it is
  /// read, which is what makes an open branch render a loading row instead of
  /// an empty group.
  listings: Readonly<Record<string, DirListing | undefined>>;
  /// Folders whose children were asked for and have not arrived.
  busy: ReadonlySet<string>;
  expanded: ReadonlySet<string>;
  selected: string | null;
  select: (rel: string) => void;
  /// `open` is the state asked for. `deep` is Option-click: the whole subtree.
  toggle: (rel: string, open: boolean, deep: boolean) => void;
}

/// The library tree's session state: which folders are open, what each one
/// holds, and which row is selected.
///
/// It lives here beside `useDocuments` rather than inside the tree component,
/// because App picks the left column with a switch on `view`, so the tree
/// unmounts on every Library-to-Run trip. Expansion would survive that on its
/// own (it persists through Settings), but the children cache and the selection
/// would not, and every open folder would re-list on the way back.
///
/// Everything is keyed on the workspace-relative path, never on an array index.
/// A refresh reorders rows, and an index-keyed selection then lands on a
/// different file with nothing to say so.
export function useProjectTree({
  projects,
  expandedPaths,
  onExpandedChange,
  refreshKey,
  showToast,
}: {
  projects: ProjectSummary[];
  /// Persisted, workspace-relative. The tree is the reader; Settings is the
  /// store.
  expandedPaths: string[];
  onExpandedChange: (paths: string[]) => void;
  /// Bumped once per finished run. Re-lists the roots and everything open.
  refreshKey: number;
  showToast: (message: string) => void;
}): ProjectTreeState {
  const [listings, setListings] = useState<Record<string, DirListing>>({});
  const [busy, setBusy] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  /// Dedupes concurrent reads of one folder. A ref rather than `busy`, because
  /// two clicks in one frame both read the same stale state.
  const inFlight = useRef(new Set<string>());
  const expandedRef = useRef(expandedPaths);

  useEffect(() => {
    expandedRef.current = expandedPaths;
  }, [expandedPaths]);

  const expanded = useMemo(() => new Set(expandedPaths), [expandedPaths]);
  const busySet = useMemo(() => new Set(busy), [busy]);

  const setExpanded = useCallback(
    (next: (current: string[]) => string[]) => {
      const paths = next(expandedRef.current);
      expandedRef.current = paths;
      onExpandedChange(paths);
    },
    [onExpandedChange],
  );

  /// Read one folder, and on failure drop it rather than leaving stale children
  /// under an open row. `announce` is on for a click the user made and off for
  /// a background refresh, so a folder deleted in Finder does not toast once
  /// per finished run.
  const read = useCallback(
    async (rel: string, announce: boolean): Promise<DirListing | null> => {
      if (inFlight.current.has(rel)) return null;
      inFlight.current.add(rel);
      setBusy((cur) => (cur.includes(rel) ? cur : [...cur, rel]));
      try {
        const listing = await commands.listProjectFiles(rel);
        setListings((cur) => ({ ...cur, [rel]: listing }));
        return listing;
      } catch (e) {
        if (announce) showToast(String(e));
        setListings((cur) =>
          Object.fromEntries(Object.entries(cur).filter(([path]) => path !== rel)),
        );
        setExpanded((cur) => cur.filter((p) => p !== rel));
        return null;
      } finally {
        inFlight.current.delete(rel);
        setBusy((cur) => cur.filter((p) => p !== rel));
      }
    },
    [setExpanded, showToast],
  );

  /// One level, or the whole subtree. Level by level rather than depth first,
  /// so a deep open costs one round of reads per level instead of one per
  /// folder.
  const load = useCallback(
    async (rel: string, deep: boolean, announce: boolean) => {
      let level = [rel];
      const opened: string[] = [];
      for (let depth = 0; level.length > 0; depth += 1) {
        const results = await Promise.all(level.map((path) => read(path, announce)));
        if (!deep || depth >= DEEP_EXPAND_DEPTH) break;
        const next = results.flatMap((listing) =>
          (listing?.entries ?? []).filter((row) => row.isDir).map((row) => row.rel),
        );
        opened.push(...next);
        level = next;
      }
      if (opened.length > 0) {
        setExpanded((cur) => Array.from(new Set([...cur, ...opened])));
      }
    },
    [read, setExpanded],
  );

  // Every project root, one shallow read each: it fills the count on every
  // project row and warms the cache for the first expansion, and it re-lists
  // whatever the user left open.
  //
  // Keyed on `projects`, which is also the gate. App reads the project list
  // behind the pending settings save, so by the time a project exists here the
  // host can answer `list_project_files` from that same settings.json. Asking
  // ahead of the write returns "No workspace configured".
  useEffect(() => {
    const roots = projects.map((p) => p.path);
    const targets = new Set(roots);
    for (const rel of expandedRef.current) {
      // An expanded path whose project is gone would read as an error on every
      // refresh. Drop it here instead of asking.
      if (roots.some((root) => rel === root || rel.startsWith(`${root}/`))) targets.add(rel);
    }
    // eslint-disable-next-line react-hooks/set-state-in-effect -- the external system is the filesystem, and `read` marks the folder busy before it goes
    for (const rel of targets) void read(rel, false);
  }, [projects, read, refreshKey]);

  const toggle = useCallback(
    (rel: string, open: boolean, deep: boolean) => {
      if (open) {
        setExpanded((cur) => (cur.includes(rel) ? cur : [...cur, rel]));
        void load(rel, deep, true);
        return;
      }
      const inside = (path: string) => path === rel || path.startsWith(`${rel}/`);
      setExpanded((cur) => cur.filter((p) => (deep ? !inside(p) : p !== rel)));
      // A row that unmounts while it holds focus drops focus on <body>, which
      // silently ends keyboard navigation. Move the selection up to the folder
      // being closed before its children go.
      setSelected((cur) => (cur !== null && cur !== rel && inside(cur) ? rel : cur));
    },
    [load, setExpanded],
  );

  return { listings, busy: busySet, expanded, selected, select: setSelected, toggle };
}
