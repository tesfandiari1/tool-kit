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
  /// Floors for the two panes, as CSS lengths.
  ///
  /// They deliberately carry different units. The start pane holds controls
  /// whose width is a fact about their content, so its floor is in pixels and
  /// does not move when the window does — a percentage floor is only wide
  /// enough at some window sizes, and silently clips at the rest. The end pane
  /// holds prose, which wants a share of whatever room there is, so it is a
  /// percentage.
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
  defaultStart = 38,
  minStart = "280px",
  minEnd = "34%",
  layout,
  onLayoutChanged,
  collapsed = false,
  className,
}: SplitPaneProps) {
  return (
    <Group
      orientation="horizontal"
      className={cx("ui-split", collapsed && "ui-split--collapsed", className)}
      defaultLayout={layout ?? { start: defaultStart, end: 100 - defaultStart }}
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
