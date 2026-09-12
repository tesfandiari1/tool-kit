import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import { Label } from "./Text";
import "./Surface.css";

export type PanelTone = "default" | "raised";

/// `title` here is a rendered header, not the DOM tooltip.
export interface PanelProps extends Omit<HTMLAttributes<HTMLDivElement>, "title"> {
  tone?: PanelTone;
  title?: ReactNode;
  actions?: ReactNode;
  /// Drop the body's padding, for lists that draw their own row insets.
  bare?: boolean;
  children?: ReactNode;
}

export function Panel({
  tone = "default",
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

export interface WellProps extends HTMLAttributes<HTMLDivElement> {
  /// False on a target you operate rather than read, so no I-beam shows.
  selectable?: boolean;
}

/// A recessed area for output the user reads. See `selectable`.
export function Well({ className, selectable = true, ...rest }: WellProps) {
  return <div className={cx("ui-well", selectable && "ui-selectable", className)} {...rest} />;
}

export interface DividerProps extends HTMLAttributes<HTMLHRElement> {
  vertical?: boolean;
}

export function Divider({ vertical = false, className, ...rest }: DividerProps) {
  return <hr className={cx("ui-divider", vertical && "ui-divider--vertical", className)} {...rest} />;
}
