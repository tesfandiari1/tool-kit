import type { ReactNode } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { cx } from "../cx";
import "./SplitPane.css";

/// Pane widths as percentages, keyed by pane id: `{ start: 38, end: 62 }`.
export type SplitLayout = Record<string, number>;

export interface SplitPaneProps {
  start: ReactNode;
  end: ReactNode;
  /// Percentage width of the starting pane before the user has dragged. Ignored
  /// when `layout` is supplied.
  defaultStart?: number;
  minStart?: number;
  minEnd?: number;
  /// A previously saved layout, to restore where the user left the divider.
  layout?: SplitLayout;
  /// Fired when a drag settles, not on every frame of it. Persist from here.
  onLayoutChanged?: (layout: SplitLayout) => void;
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
  defaultStart = 38,
  minStart = 26,
  minEnd = 34,
  layout,
  onLayoutChanged,
  className,
}: SplitPaneProps) {
  return (
    <Group
      orientation="horizontal"
      className={cx("ui-split", className)}
      defaultLayout={layout ?? { start: defaultStart, end: 100 - defaultStart }}
      onLayoutChanged={onLayoutChanged}
    >
      <Panel id="start" minSize={`${String(minStart)}%`} className="ui-split__pane">
        {start}
      </Panel>
      {/* A 1px rule with a 9px grab area around it. The seam stays a hairline
          at rest — widening it on hover would make the window twitch every
          time the pointer crossed the middle. */}
      <Separator className="ui-split__handle" />
      <Panel id="end" minSize={`${String(minEnd)}%`} className="ui-split__pane">
        {end}
      </Panel>
    </Group>
  );
}
