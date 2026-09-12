import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands } from "@/app/commands";
import type { DirListing, ListError, ProjectSummary } from "@/app/types";
import { onWindowFocused } from "@/platform/host";
import type { ToastTone } from "./useToast";

/// One click that fires a thousand `read_dir` calls is a frozen tree.
const DEEP_EXPAND_DEPTH = 6;

/// The depth cap bounds the rounds, not the width, and a JS checkout is
/// thousands of package folders on one level.
const DEEP_EXPAND_BUDGET = 200;

/// A listing is a `read_dir` plus a `stat` per entry, so a whole level at once
/// is a level-sized spike on a network volume.
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

/// Only a folder the host reports as gone may cost the persisted expansion.
export function listFailure(e: unknown): ListError {
  if (typeof e === "object" && e !== null && "gone" in e && "message" in e) {
    return {
      gone: e.gone === true,
      message: typeof e.message === "string" ? e.message : "Could not read that folder",
    };
  }
  return { gone: false, message: String(e) };
}

/// The folders one level of a deep expand hands to the next, within budget.
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

/// Where the selection goes when `doomed` unmounts: its parent, and only when
/// the selection is inside it. Null on a project root, which has no parent row.
export function climbTarget(selected: string | null, doomed: string): string | null {
  if (selected === null) return null;
  if (selected !== doomed && !selected.startsWith(`${doomed}/`)) return null;
  const cut = doomed.lastIndexOf("/");
  return cut > 0 ? doomed.slice(0, cut) : null;
}

/// `Focused(true)` is AppKit's `windowDidBecomeKey:`, so it arrives on every
/// ⌘Tab return, file picker and confirm.
const FOCUS_DEBOUNCE_MS = 250;
const FOCUS_FLOOR_MS = 1500;

/// - `click`: the user asked, so a failure gets a toast.
/// - `refresh`: mount and finished run. Silent, still marks busy.
/// - `reconcile`: window focus. Silent, no busy flag, and the answer is dropped
///   unless the folder's own mtime moved.
type ReadMode = "click" | "refresh" | "reconcile";

/// Every folder worth re-reading. An expanded path whose project is gone is
/// dropped here rather than asked for and failed on every refresh.
function reconcileTargets(projects: ProjectSummary[], expanded: readonly string[]): string[] {
  const roots = projects.map((p) => p.path);
  const targets = new Set(roots);
  for (const rel of expanded) {
    if (roots.some((root) => rel === root || rel.startsWith(`${root}/`))) targets.add(rel);
  }
  return Array.from(targets);
}

export interface ProjectTreeState {
  /// Undefined until read, which makes an open branch render a loading row
  /// rather than an empty group.
  listings: Readonly<Record<string, DirListing | undefined>>;
  busy: ReadonlySet<string>;
  expanded: ReadonlySet<string>;
  selected: string | null;
  select: (rel: string) => void;
  /// `open` is the state asked for. `deep` is Option-click: the whole subtree.
  toggle: (rel: string, open: boolean, deep: boolean) => void;
}

/// The library tree's session state, keyed on the workspace-relative path and
/// never on an index: a refresh reorders rows. It owns freshness too, and both
/// re-reads merge into the same cache rather than replace it.
export function useProjectTree({
  projects,
  expandedPaths,
  onExpandedChange,
  refreshKey,
  showToast,
}: {
  projects: ProjectSummary[];
  /// Persisted, workspace-relative. The tree reads it, Settings stores it.
  expandedPaths: string[];
  onExpandedChange: (paths: string[]) => void;
  /// The finished-run counter plus every setting the host's pairing rule reads.
  refreshKey: string;
  showToast: (message: string, tone?: ToastTone) => void;
}): ProjectTreeState {
  const [listings, setListings] = useState<Record<string, DirListing>>({});
  const [busy, setBusy] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  /// The cache `listings` renders. `read` compares against what it holds, which
  /// a `setListings` updater cannot answer: React decides when to run one.
  const cache = useRef<Record<string, DirListing>>({});
  /// A ref rather than `busy`: two clicks in one frame read the same state. A
  /// read already running is handed back, so a deep expand can await it rather
  /// than skip the folder.
  const inFlight = useRef(new Map<string, Promise<DirListing | null>>());
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

  /// A row that unmounts holding focus drops it on `<body>`, which silently
  /// ends keyboard navigation.
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

  /// The selection climbs before a reconcile replaces the children.
  const keepFocusInside = useCallback(
    (rel: string, listing: DirListing) => {
      const current = selectedRef.current;
      if (!current?.startsWith(`${rel}/`)) return;
      // A folder deleted in Finder cannot answer for its own rows, so this is
      // the last listing that sees the whole subtree go.
      const child = `${rel}/${current.slice(rel.length + 1).split("/")[0]}`;
      if (listing.entries.some((entry) => entry.rel === child)) return;
      climbFrom(child);
    },
    [climbFrom],
  );

  /// Read one folder. Only a `gone` answer drops its children, its expansion
  /// and its focus: a volume asleep is not a folder deleted.
  const read = useCallback(
    (rel: string, mode: ReadMode): Promise<DirListing | null> => {
      const running = inFlight.current.get(rel);
      if (running) return running;
      const marks = mode !== "reconcile";
      if (marks) setBusy((cur) => (cur.includes(rel) ? cur : [...cur, rel]));
      const pending = (async () => {
        try {
          const listing = await commands.listProjectFiles(rel);
          // `in` rather than a truth test: the index signature types every key
          // as present, so a bare read takes a property off undefined.
          const moved =
            !(rel in cache.current) || cache.current[rel].modifiedMs !== listing.modifiedMs;
          // An untouched folder keeps its object: a new identity re-renders
          // every row under it and buys nothing.
          if (moved || mode !== "reconcile") {
            if (moved) keepFocusInside(rel, listing);
            cache.current = { ...cache.current, [rel]: listing };
            setListings(cache.current);
          }
          return listing;
        } catch (e) {
          const failure = listFailure(e);
          // A branch with nothing to render must say why. Reconcile stays
          // quiet: it fires on every window focus.
          if (mode === "click" || (mode === "refresh" && !(rel in cache.current))) {
            showToast(failure.message, "danger");
          }
          // Dropping these on every failure lets one sleeping volume empty
          // `expandedPaths` for good.
          if (!failure.gone) {
            if (mode === "click") setExpanded((cur) => cur.filter((p) => p !== rel));
            return null;
          }
          climbFrom(rel);
          cache.current = Object.fromEntries(
            Object.entries(cache.current).filter(([path]) => path !== rel),
          );
          setListings(cache.current);
          setExpanded((cur) => cur.filter((p) => p !== rel));
          return null;
        } finally {
          if (marks) setBusy((cur) => cur.filter((p) => p !== rel));
        }
      })();
      inFlight.current.set(rel, pending);
      void pending.finally(() => {
        // Only this read: a later one already replaced the entry.
        if (inFlight.current.get(rel) === pending) inFlight.current.delete(rel);
      });
      return pending;
    },
    [climbFrom, keepFocusInside, setExpanded, showToast],
  );

  /// Unseen folders plus the ones the host says moved. A failure answers all
  /// of them, because the stat is only an optimisation.
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

  /// Level by level rather than depth first, so a deep open costs one round of
  /// reads per level.
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
      // A click that collapses `rel` during these awaits must not persist six
      // levels of its descendants.
      if (opened.length > 0 && expandedRef.current.includes(rel)) {
        setExpanded((cur) => Array.from(new Set([...cur, ...opened])));
      }
    },
    [read, setExpanded],
  );

  // Keyed on `projects`, which is also the gate: App reads that list behind the
  // pending settings save, which is the file the host answers from.
  useEffect(() => {
    const targets = reconcileTargets(projects, expandedRef.current);
    void mapLimit(targets, READ_LIMIT, (rel) => read(rel, "refresh"));
  }, [projects, read, refreshKey]);

  // The reconcile the app runs instead of a watcher. Ask the cheap question
  // first, or a cancelled file picker pays for a full walk to learn nothing.
  // Never on `job-updated`: a 200-file run emits hundreds of those.
  useEffect(() => {
    let timer = 0;
    let last = 0;
    const un = onWindowFocused(() => {
      // One pending reconcile is enough: a focus burst queues one per event.
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
      // See `climbFrom`: a row unmounting with focus ends keyboard navigation.
      setSelected((cur) => (cur !== null && cur !== rel && inside(cur) ? rel : cur));
    },
    [load, setExpanded],
  );

  return { listings, busy: busySet, expanded, selected, select: setSelected, toggle };
}
