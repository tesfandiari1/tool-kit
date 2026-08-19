import { useEffect, useMemo, useRef } from "react";
import type { ReactNode } from "react";
import { Group, Panel, Separator, useGroupRef } from "react-resizable-panels";
import { paneLayout, type SplitLayout } from "./splitLayout";
import { cx } from "../cx";
import "./SplitPane.css";

export type { SplitLayout };

/// Frame budget for the restore below. Roughly 1.5s, because it waits out a
/// native window resize, not just a render.
const RESTORE_FRAMES = 90;
/// Percentage points within which a restored layout counts as applied.
const RESTORE_EPSILON = 0.5;

export interface SplitPaneProps {
  start: ReactNode;
  end: ReactNode;
  /// Percentage width of the starting pane before the user has dragged. Ignored
  /// when `layout` is supplied.
  defaultStart?: number;
  /// Floors for the two panes, as CSS lengths.
  ///
  /// They deliberately carry different units. The start pane holds controls
  /// whose width is a fact about their content, so its floor is in pixels and
  /// does not move when the window does — a percentage floor is only wide
  /// enough at some window sizes, and silently clips at the rest. The end pane
  /// holds the document, so its floor is a share of the window: half is the
  /// least worth opening a reading pane for.
  minStart?: string;
  minEnd?: string;
  /// A previously saved layout, to restore where the user left the divider.
  layout?: SplitLayout;
  /// Fired when the user settles a drag or resizes with the keyboard, and at no
  /// other time. Persist from here.
  onLayoutChanged?: (layout: SplitLayout) => void;
  /// Drop the seam and the end pane, leaving the start pane the whole width.
  ///
  /// A prop rather than the caller rendering `start` on its own, because those
  /// are two different positions in the tree and React answers a move by
  /// remounting: whatever the start pane had in local state — a half-typed key,
  /// a search box — is thrown away every time the split opens or closes. Here
  /// the start pane stays the first child either way and keeps its instance.
  collapsed?: boolean;
  className?: string;
}

/// Two panes and a divider you can drag.
///
/// The library is here for the parts that are tedious rather than hard:
/// pointer capture that survives leaving the window, a divider that answers
/// arrow keys, and double-click to reset. The hairline itself is ours.
///
/// Persistence is deliberately not delegated. The library can save to
/// localStorage; this app already has `settings.json` and a `save_settings`
/// command, and a desktop app keeping half its window state in the webview's
/// storage is how you get a layout that survives a reload but not a reinstall.
/// So the layout comes in as a prop and goes out as a callback.
export function SplitPane({
  start,
  end,
  // These three repeat `SPLIT` in src/shell/geometry.ts by hand, because
  // nothing under src/ui may import from the app. App.tsx passes the canonical
  // values, so these only cover a caller that passes none. Change one, change
  // the other.
  defaultStart = 33,
  minStart = "300px",
  minEnd = "50%",
  layout,
  onLayoutChanged,
  collapsed = false,
  className,
}: SplitPaneProps) {
  const groupRef = useGroupRef();
  const wasCollapsed = useRef(collapsed);
  const owedRestore = useRef(false);
  // Never spread from `layout`: `setLayout` is positional, so key order picks
  // which pane gets which width. See `paneLayout`.
  const intended = useMemo(() => paneLayout(layout, defaultStart), [layout, defaultStart]);

  /* Restore the seam when the split opens. `defaultLayout` is only honored when
     its ids match the panels present at mount, and collapsed there is one, so
     `end` arrives to an even split instead of the saved layout.

     It retries because two things arrive late: the group registers its second
     panel a render after this runs, and the window is still growing from
     launcher width, where 33% falls under `minStart` and gets clamped there for
     good. So it re-applies until it reads back what it asked for.

     Only on the collapsed -> expanded edge, or it would pull the seam out from
     under a drag. Nothing here persists: the library marks imperative layouts
     `isUserInteraction: false` and the callback below drops those. */
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
      /* Only a real drag or a keyboard resize. The library also fires this on
         mount, on a constraint recompute and after any imperative call, all
         with `isUserInteraction: false` — forwarding those lets a layout the
         window merely clamped get saved as the one the user chose. */
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
      {/* A 1px rule with a 9px grab area around it. The seam stays a hairline
          at rest — widening it on hover would make the window twitch every
          time the pointer crossed the middle. */}
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
