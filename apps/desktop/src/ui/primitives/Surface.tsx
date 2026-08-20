import type { CSSProperties, HTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import { Label } from "./Text";
import "./Surface.css";

export type PanelTone = "default" | "raised" | "well";

/// `title` is omitted from the DOM attributes because this one is a rendered
/// header, not a tooltip string.
export interface PanelProps extends Omit<HTMLAttributes<HTMLDivElement>, "title"> {
  tone?: PanelTone;
  /// Square corners, for a panel that butts against a window edge or a grid.
  flush?: boolean;
  /// Rendered as a Label in the header rail.
  title?: ReactNode;
  /// Right-aligned header content: counts, buttons, status.
  actions?: ReactNode;
  /// Drop the body's padding, for lists that draw their own row insets.
  bare?: boolean;
  children?: ReactNode;
}

export function Panel({
  tone = "default",
  flush = false,
  title,
  actions,
  bare = false,
  className,
  children,
  ...rest
}: PanelProps) {
  return (
    <div
      className={cx(
        "ui-panel",
        tone !== "default" && `ui-panel--${tone}`,
        flush && "ui-panel--flush",
        className,
      )}
      {...rest}
    >
      {(title ?? actions) && (
        <div className="ui-panel__header">
          {typeof title === "string" ? <Label tone="strong">{title}</Label> : title}
          {actions}
        </div>
      )}
      <div className={cx("ui-panel__body", bare && "ui-panel__body--flush")}>{children}</div>
    </div>
  );
}

export interface CellGridProps extends HTMLAttributes<HTMLDivElement> {
  /// Column count. Cells share hairlines, so this is the lattice width.
  columns?: number;
  children?: ReactNode;
}

export function CellGrid({ columns = 3, className, style, children, ...rest }: CellGridProps) {
  const gridStyle: CSSProperties = {
    gridTemplateColumns: `repeat(${String(columns)}, minmax(0, 1fr))`,
    ...style,
  };
  return (
    <div className={cx("ui-cellgrid", className)} style={gridStyle} {...rest}>
      {children}
    </div>
  );
}

export interface CellProps extends HTMLAttributes<HTMLDivElement> {
  interactive?: boolean;
  /// Tighter padding, for list rows rather than feature cards.
  dense?: boolean;
  children?: ReactNode;
}

export function Cell({ interactive = false, dense = false, className, ...rest }: CellProps) {
  return (
    <div
      className={cx(
        "ui-cell",
        interactive && "ui-cell--interactive",
        dense && "ui-cell--pad-sm",
        className,
      )}
      {...rest}
    />
  );
}

export interface WellProps extends HTMLAttributes<HTMLDivElement> {
  /// Text selection is for output you read. Drop targets and logs that you
  /// operate on should pass false so the well does not show an I-beam.
  selectable?: boolean;
}

/// A recessed area for output the user reads rather than operates: transcripts,
/// markdown, logs. Text selection is granted back here when `selectable`.
export function Well({ className, selectable = true, ...rest }: WellProps) {
  return <div className={cx("ui-well", selectable && "ui-selectable", className)} {...rest} />;
}

export interface DividerProps extends HTMLAttributes<HTMLHRElement> {
  vertical?: boolean;
}

export function Divider({ vertical = false, className, ...rest }: DividerProps) {
  return <hr className={cx("ui-divider", vertical && "ui-divider--vertical", className)} {...rest} />;
}
