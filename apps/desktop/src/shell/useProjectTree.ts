import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands } from "@/app/commands";
import type { DirListing, ProjectSummary } from "@/app/types";
import { onWindowFocused } from "@/platform/host";

/// How far one Option-click descends. `NSOutlineView` opens the whole subtree,
/// but a workspace can hold a folder nobody meant to walk, and one click that
/// fires a thousand `read_dir` calls is a frozen tree with nothing on screen to
/// explain it.
const DEEP_EXPAND_DEPTH = 6;

/// How long the focus reconcile waits, and how long it then stays quiet.
/// `Focused(true)` is AppKit's `windowDidBecomeKey:`, so it arrives on every
/// ⌘Tab return, after every file picker, and after every confirm this app
/// raises itself. The debounce coalesces one burst and the floor keeps a run of
/// them from becoming a run of `read_dir` calls.
const FOCUS_DEBOUNCE_MS = 250;
const FOCUS_FLOOR_MS = 1500;

/// Why a folder is being read.
///
/// - `click`: the user asked, so a failure gets a toast.
/// - `refresh`: mount, and once per finished run. Silent, still marks busy.
/// - `reconcile`: the window came back to the front. Silent, no busy flag, and
///   the answer is dropped unless the folder's own mtime moved, so a ⌘Tab round
///   trip over an untouched workspace costs no render at all.
type ReadMode = "click" | "refresh" | "reconcile";

/// Every folder worth re-reading: each project root, plus whatever the user
/// left open inside one. An expanded path whose project is gone would read as
/// an error on every refresh, so it is dropped here rather than asked for.
function reconcileTargets(projects: ProjectSummary[], expanded: readonly string[]): string[] {
  const roots = projects.map((p) => p.path);
  const targets = new Set(roots);
  for (const rel of expanded) {
    if (roots.some((root) => rel === root || rel.startsWith(`${root}/`))) targets.add(rel);
  }
  return Array.from(targets);
}

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
///
/// It also owns freshness. The tree re-reads on a finished run and when the
/// window comes back to the front, and both merge into the same cache, so a
/// refresh never remounts the tree or throws away what the user has open.
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
  /// The cache `listings` renders. `read` is its only writer and has to compare
  /// against what it already holds, which a `setListings` updater cannot answer:
  /// React decides when to run one, so a read cannot get its answer back out.
  const cache = useRef<Record<string, DirListing>>({});
  /// Dedupes concurrent reads of one folder. A ref rather than `busy`, because
  /// two clicks in one frame both read the same stale state.
  const inFlight = useRef(new Set<string>());
  const expandedRef = useRef(expandedPaths);
  const projectsRef = useRef(projects);
  const selectedRef = useRef(selected);

  useEffect(() => {
    expandedRef.current = expandedPaths;
  }, [expandedPaths]);

  useEffect(() => {
    projectsRef.current = projects;
    selectedRef.current = selected;
  }, [projects, selected]);

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

  /// A row that unmounts while it holds focus drops focus on `<body>`, which
  /// ends keyboard navigation with nothing on screen to say so. Deleting the
  /// selected file in Finder does exactly that on the next reconcile, so the
  /// selection and the focus both climb to the folder before its children are
  /// replaced.
  const keepFocusInside = useCallback((rel: string, listing: DirListing) => {
    const current = selectedRef.current;
    if (!current?.startsWith(`${rel}/`)) return;
    // Direct children only. A deeper row's own folder answers for it.
    if (current.slice(rel.length + 1).includes("/")) return;
    if (listing.entries.some((entry) => entry.rel === current)) return;
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.dataset.path === current) {
      document.querySelector<HTMLElement>(`[data-path="${CSS.escape(rel)}"]`)?.focus();
    }
    selectedRef.current = rel;
    setSelected(rel);
  }, []);

  /// Read one folder, and on failure drop it rather than leaving stale children
  /// under an open row. A background read stays silent, so a folder deleted in
  /// Finder does not toast once per finished run.
  const read = useCallback(
    async (rel: string, mode: ReadMode): Promise<DirListing | null> => {
      if (inFlight.current.has(rel)) return null;
      inFlight.current.add(rel);
      const marks = mode !== "reconcile";
      if (marks) setBusy((cur) => (cur.includes(rel) ? cur : [...cur, rel]));
      try {
        const listing = await commands.listProjectFiles(rel);
        // `in` rather than a truth test on the lookup: the index signature
        // types every key as present, so a bare read of a folder nobody has
        // listed yet would take a property off undefined.
        const moved =
          !(rel in cache.current) || cache.current[rel].modifiedMs !== listing.modifiedMs;
        // An untouched folder keeps the object it already has. A new identity
        // would re-render every row under it and buy nothing, which is the
        // whole reason the listing carries the directory's own mtime.
        if (moved || mode !== "reconcile") {
          if (moved) keepFocusInside(rel, listing);
          cache.current = { ...cache.current, [rel]: listing };
          setListings(cache.current);
        }
        return listing;
      } catch (e) {
        if (mode === "click") showToast(String(e));
        cache.current = Object.fromEntries(
          Object.entries(cache.current).filter(([path]) => path !== rel),
        );
        setListings(cache.current);
        setExpanded((cur) => cur.filter((p) => p !== rel));
        return null;
      } finally {
        inFlight.current.delete(rel);
        if (marks) setBusy((cur) => cur.filter((p) => p !== rel));
      }
    },
    [keepFocusInside, setExpanded, showToast],
  );

  /// One level, or the whole subtree. Level by level rather than depth first,
  /// so a deep open costs one round of reads per level instead of one per
  /// folder.
  const load = useCallback(
    async (rel: string, deep: boolean, mode: ReadMode) => {
      let level = [rel];
      const opened: string[] = [];
      for (let depth = 0; level.length > 0; depth += 1) {
        const results = await Promise.all(level.map((path) => read(path, mode)));
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
    for (const rel of reconcileTargets(projects, expandedRef.current)) void read(rel, "refresh");
  }, [projects, read, refreshKey]);

  // The window came back to the front, so re-read what is open. This is the
  // reconcile the north star asks for instead of a watcher: a change the user
  // made in Finder while the app was in the background shows up the moment they
  // come back to it, and nothing has to run while it is not frontmost.
  //
  // Deliberately not on `job-updated`: a 200-file run emits hundreds of those
  // events, and the tree would walk the disk hundreds of times per run. The
  // run-finished door is `refreshKey` above.
  useEffect(() => {
    let timer = 0;
    let last = 0;
    const un = onWindowFocused(() => {
      // One pending reconcile is enough. The burst that follows a file picker
      // would otherwise queue one per event.
      if (timer !== 0) return;
      const wait = Math.max(FOCUS_DEBOUNCE_MS, FOCUS_FLOOR_MS - (Date.now() - last));
      timer = window.setTimeout(() => {
        timer = 0;
        last = Date.now();
        for (const rel of reconcileTargets(projectsRef.current, expandedRef.current)) {
          void read(rel, "reconcile");
        }
      }, wait);
    });
    return () => {
      window.clearTimeout(timer);
      void un.then((f) => f());
    };
  }, [read]);

  const toggle = useCallback(
    (rel: string, open: boolean, deep: boolean) => {
      if (open) {
        setExpanded((cur) => (cur.includes(rel) ? cur : [...cur, rel]));
        void load(rel, deep, "click");
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
