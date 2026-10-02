import { useMemo } from "react";
import type { ReactNode } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { paneLayout, type SplitLayout } from "./splitLayout";
import { cx } from "../cx";
import "./SplitPane.css";

export type { SplitLayout };

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
  className,
}: SplitPaneProps) {
  const intended = useMemo(() => paneLayout(layout, defaultStart), [layout, defaultStart]);

  return (
    <Group
      orientation="horizontal"
      className={cx("ui-split", className)}
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
      <Panel id="start" minSize={minStart} className="ui-split__pane">
        {start}
      </Panel>
      {/* A 1px rule with a 9px grab area. Widening it on hover twitches the
          window whenever the pointer crosses the middle. */}
      <Separator className="ui-split__handle" />
      <Panel id="end" minSize={minEnd} className="ui-split__pane">
        {end}
      </Panel>
    </Group>
  );
}
