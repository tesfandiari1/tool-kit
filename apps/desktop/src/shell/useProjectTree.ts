import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands } from "@/app/commands";
import type { DirListing, ListError, ProjectSummary } from "@/app/types";
import { onWindowFocused } from "@/platform/host";

/// How far one Option-click descends. `NSOutlineView` opens the whole subtree,
/// but a workspace can hold a folder nobody meant to walk, and one click that
/// fires a thousand `read_dir` calls is a frozen tree with nothing on screen to
/// explain it.
const DEEP_EXPAND_DEPTH = 6;

/// How many folders one Option-click may open in total.
///
/// The depth cap bounds the rounds, not the width, and width is where the
/// damage is: a project holding a JS checkout has thousands of package folders
/// on one level, and `node_modules` is not a dotfile. Every one of them would
/// be an invoke, a row, and a line in the persisted expansion that every launch
/// and every finished run then re-reads. The walk stops when the budget is
/// spent, the same way it stops at the depth cap.
const DEEP_EXPAND_BUDGET = 200;

/// How many listings are ever in flight at once. Each one is a `read_dir` plus
/// a `stat` per entry on the host's blocking pool, so a whole level issued at
/// once is a level-sized spike on a network volume.
const READ_LIMIT = 8;

/// Run `job` over `items`, never more than `limit` at a time.
async function mapLimit<A, B>(
  items: readonly A[],
  limit: number,
  job: (item: A) => Promise<B>,
): Promise<B[]> {
  const out: B[] = [];
  for (let start = 0; start < items.length; start += limit) {
    out.push(...(await Promise.all(items.slice(start, start + limit).map(job))));
  }
  return out;
}

/// What a failed listing means. The host answers with a `ListError`, and only a
/// folder that is really gone may cost the user their persisted expansion.
/// Anything else — an unknown rejection included — is read as still there.
export function listFailure(e: unknown): ListError {
  if (typeof e === "object" && e !== null && "gone" in e && "message" in e) {
    return {
      gone: e.gone === true,
      message: typeof e.message === "string" ? e.message : "Could not read that folder",
    };
  }
  return { gone: false, message: String(e) };
}

/// The folders a finished level of a deep expand hands to the next one, capped
/// by what is left of the budget.
export function childDirs(level: readonly (DirListing | null)[], budget: number): string[] {
  const dirs: string[] = [];
  for (const listing of level) {
    for (const row of listing?.entries ?? []) {
      if (!row.isDir) continue;
      if (dirs.length === budget) return dirs;
      dirs.push(row.rel);
    }
  }
  return dirs;
}

/// Where the selection goes when `doomed` and everything under it is about to
/// unmount: to its parent, and only when the selection is inside it. Null when
/// there is nothing to move, or when `doomed` is a project root, whose parent
/// is the workspace and has no row.
export function climbTarget(selected: string | null, doomed: string): string | null {
  if (selected === null) return null;
  if (selected !== doomed && !selected.startsWith(`${doomed}/`)) return null;
  const cut = doomed.lastIndexOf("/");
  return cut > 0 ? doomed.slice(0, cut) : null;
}

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
///   the answer is dropped unless the folder's own mtime moved. Only folders
///   the host has already named as moved are read this way, so a ⌘Tab round
///   trip over an untouched workspace costs one `stat` a folder and no render.
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
  /// Re-lists the roots and everything open when it changes. It carries the
  /// finished-run counter and every setting the host's pairing rule reads, so
  /// changing the output format cannot leave a converted file marked
  /// unconverted until the folder itself moves.
  refreshKey: string;
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
  /// ends keyboard navigation with nothing on screen to say so. Move the
  /// selection, and the focus with it, to the parent of the row that is going.
  const climbFrom = useCallback((doomed: string) => {
    const target = climbTarget(selectedRef.current, doomed);
    if (target === null) return;
    const active = document.activeElement;
    const path = active instanceof HTMLElement ? (active.dataset.path ?? null) : null;
    if (path !== null && (path === doomed || path.startsWith(`${doomed}/`))) {
      document.querySelector<HTMLElement>(`[data-path="${CSS.escape(target)}"]`)?.focus();
    }
    selectedRef.current = target;
    setSelected(target);
  }, []);

  /// Deleting the selected file in Finder unmounts its row on the next
  /// reconcile, so the selection climbs before the children are replaced.
  const keepFocusInside = useCallback(
    (rel: string, listing: DirListing) => {
      const current = selectedRef.current;
      if (!current?.startsWith(`${rel}/`)) return;
      // The direct child that carries the selection, whether that is the row
      // itself or something deeper inside it. A deeper row's own folder
      // usually answers for it, but a folder deleted in Finder never gets that
      // far: its own listing fails, and this is the last one that can see the
      // whole subtree go.
      const child = `${rel}/${current.slice(rel.length + 1).split("/")[0]}`;
      if (listing.entries.some((entry) => entry.rel === child)) return;
      climbFrom(child);
    },
    [climbFrom],
  );

  /// Read one folder. A folder the host reports as gone loses its children, its
  /// place in the persisted expansion and, if the selection was inside it, the
  /// focus; every other failure changes nothing, because a volume asleep is not
  /// a folder deleted. A background read stays silent either way, so a folder
  /// deleted in Finder does not toast once per finished run.
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
        const failure = listFailure(e);
        if (mode === "click") showToast(failure.message);
        // A folder that is still there keeps its children and its place in the
        // persisted expansion. Dropping both on every failure meant one
        // sleeping volume, or one read that raced the settings write, emptied
        // `expandedPaths` for good — silently, on a background read the user
        // never asked for.
        if (!failure.gone) {
          // The user clicked and it did not open, so the row closes again. The
          // toast above says why.
          if (mode === "click") setExpanded((cur) => cur.filter((p) => p !== rel));
          return null;
        }
        // Gone. Its whole subtree is about to unmount, focus and all.
        climbFrom(rel);
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
    [climbFrom, keepFocusInside, setExpanded, showToast],
  );

  /// Which of these folders are worth a listing: everything the cache has
  /// never seen, plus everything the host says has moved.
  ///
  /// A failure answers "all of them". The stat is an optimisation, and a
  /// reconcile that stops happening because the cheap call failed is a tree
  /// that quietly stops telling the truth.
  const staleAmong = useCallback(async (targets: string[]): Promise<string[]> => {
    const known = targets
      .filter((rel) => rel in cache.current)
      .map((rel) => ({ rel, modifiedMs: cache.current[rel].modifiedMs }));
    if (known.length === 0) return targets;
    try {
      const moved = new Set(await commands.changedProjectDirs(known));
      return targets.filter((rel) => !(rel in cache.current) || moved.has(rel));
    } catch {
      return targets;
    }
  }, []);

  /// One level, or the whole subtree. Level by level rather than depth first,
  /// so a deep open costs one round of reads per level instead of one per
  /// folder.
  const load = useCallback(
    async (rel: string, deep: boolean, mode: ReadMode) => {
      let level = [rel];
      const opened: string[] = [];
      let budget = DEEP_EXPAND_BUDGET;
      for (let depth = 0; level.length > 0; depth += 1) {
        const results = await mapLimit(level, READ_LIMIT, (path) => read(path, mode));
        if (!deep || depth >= DEEP_EXPAND_DEPTH || budget === 0) break;
        const next = childDirs(results, budget);
        budget -= next.length;
        opened.push(...next);
        level = next;
      }
      // Only if the user still has the branch open. These awaits can run for a
      // second over a network volume, and a click that collapses `rel` in the
      // middle of one used to be answered by writing all six levels of its
      // descendants into the persisted expansion anyway — invisibly, because
      // `rel` itself is gone from the list, until the next click opened the
      // whole subtree at once.
      if (opened.length > 0 && expandedRef.current.includes(rel)) {
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
    const targets = reconcileTargets(projects, expandedRef.current);
    void mapLimit(targets, READ_LIMIT, (rel) => read(rel, "refresh"));
  }, [projects, read, refreshKey]);

  // The window came back to the front, so re-read what changed. This is the
  // reconcile the north star asks for instead of a watcher: a change the user
  // made in Finder while the app was in the background shows up the moment they
  // come back to it, and nothing has to run while it is not frontmost.
  //
  // Ask the cheap question first. A listing is a `read_dir` plus a `stat` per
  // entry, and the answer is thrown away when the folder did not move, so a
  // cancelled file picker over a large folder on a network volume was paying
  // for a full walk to learn nothing. `changed_project_dirs` costs one `stat`
  // a folder and names the ones worth reading.
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
        const targets = reconcileTargets(projectsRef.current, expandedRef.current);
        void (async () => {
          const stale = await staleAmong(targets);
          await mapLimit(stale, READ_LIMIT, (rel) => read(rel, "reconcile"));
        })();
      }, wait);
    });
    return () => {
      window.clearTimeout(timer);
      void un.then((f) => f());
    };
  }, [read, staleAmong]);

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
