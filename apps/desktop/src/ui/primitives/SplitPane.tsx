import { useEffect, useMemo, useRef } from "react";
import type { ReactNode } from "react";
import { Group, Panel, Separator, useGroupRef } from "react-resizable-panels";
import { paneLayout, type SplitLayout } from "./splitLayout";
import { cx } from "../cx";
import "./SplitPane.css";

export type { SplitLayout };

/// Roughly 1.5s: the restore waits out a native window resize.
const RESTORE_FRAMES = 90;
/// Percentage points within which a restored layout counts as applied.
const RESTORE_EPSILON = 0.5;

export interface SplitPaneProps {
  start: ReactNode;
  end: ReactNode;
  /// Ignored when `layout` is supplied.
  defaultStart?: number;
  /// The units differ on purpose: the start pane's floor is a fact about its
  /// content, the end pane's a share of the window.
  minStart?: string;
  minEnd?: string;
  layout?: SplitLayout;
  /// Only a settled drag or a keyboard resize. Persist from here.
  onLayoutChanged?: (layout: SplitLayout) => void;
  /// Drop the seam and the end pane. A prop rather than the caller rendering
  /// `start` alone, because moving it in the tree remounts it.
  collapsed?: boolean;
  className?: string;
}

/// Two panes and a divider you can drag. The library covers pointer capture,
/// arrow keys and double-click to reset. Persistence is not delegated: its
/// localStorage survives a reload but not a reinstall, so the layout comes in
/// as a prop and goes out as a callback.
export function SplitPane({
  start,
  end,
  // These three repeat `SPLIT` in src/shell/geometry.ts by hand, because nothing
  // under src/ui may import from the app. Change one, change the other.
  defaultStart = 26,
  minStart = "240px",
  minEnd = "50%",
  layout,
  onLayoutChanged,
  collapsed = false,
  className,
}: SplitPaneProps) {
  const groupRef = useGroupRef();
  const wasCollapsed = useRef(collapsed);
  /// Owed at mount too: `defaultLayout` is validated against the panels then
  /// registered, and the second arrives a render later.
  const owedRestore = useRef(true);
  // Never spread from `layout`: `setLayout` is positional, so key order picks
  // which pane gets which width. See `paneLayout`.
  const intended = useMemo(() => paneLayout(layout, defaultStart), [layout, defaultStart]);

  /* Restore the seam when the split opens: `defaultLayout` is honored only
     when its ids match the panels present at mount. It retries because the
     second panel registers a render later and the window is still growing,
     where the start percentage is clamped under `minStart` for good. Only on
     the collapsed to expanded edge, or it pulls the seam out from a drag. */
  useEffect(() => {
    if (wasCollapsed.current && !collapsed) owedRestore.current = true;
    wasCollapsed.current = collapsed;
    if (collapsed || !owedRestore.current) return;

    let left = RESTORE_FRAMES;
    let frame = 0;
    const settled = (applied: SplitLayout) =>
      Object.keys(intended).every(
        (id) => Math.abs((applied[id] ?? 0) - (intended[id] ?? 0)) < RESTORE_EPSILON,
      );

    const restore = () => {
      const group = groupRef.current;
      const applied = group?.getLayout() ?? {};
      if (group && Object.keys(applied).length === Object.keys(intended).length) {
        if (settled(applied)) {
          owedRestore.current = false;
          return;
        }
        group.setLayout(intended);
      }
      if (--left > 0) frame = requestAnimationFrame(restore);
      else owedRestore.current = false;
    };
    frame = requestAnimationFrame(restore);
    return () => {
      cancelAnimationFrame(frame);
    };
  }, [collapsed, intended, groupRef]);

  return (
    <Group
      orientation="horizontal"
      groupRef={groupRef}
      className={cx("ui-split", collapsed && "ui-split--collapsed", className)}
      defaultLayout={intended}
      /* The library also fires on mount, on a recompute and after any
         imperative call, and saving those records a clamp as a choice. */
      onLayoutChanged={
        onLayoutChanged &&
        ((next, meta) => {
          if (meta.isUserInteraction) onLayoutChanged(next);
        })
      }
    >
      <Panel id="start" minSize={collapsed ? "0%" : minStart} className="ui-split__pane">
        {start}
      </Panel>
      {/* A 1px rule with a 9px grab area. Widening it on hover twitches the
          window whenever the pointer crosses the middle. */}
      {!collapsed && (
        <>
          <Separator className="ui-split__handle" />
          <Panel id="end" minSize={minEnd} className="ui-split__pane">
            {end}
          </Panel>
        </>
      )}
    </Group>
  );
}
