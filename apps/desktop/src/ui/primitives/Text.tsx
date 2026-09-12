import type { ElementType, HTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import "./Text.css";

/// Four families, four jobs: Label for every heading and control, Display for
/// panel titles, Text for prose, Meta for anything scanned in a column.

type Base = HTMLAttributes<HTMLElement>;

export type LabelTone = "default" | "strong";

export interface LabelProps extends Base {
  as?: ElementType;
  tone?: LabelTone;
  htmlFor?: string;
  children?: ReactNode;
}

export function Label({ as: As = "span", tone = "default", className, ...rest }: LabelProps) {
  return <As className={cx("ui-label", tone !== "default" && `ui-label--${tone}`, className)} {...rest} />;
}

/// No `sm`: the display face muddies at UI scale.
export type DisplaySize = "lg" | "xl" | "2xl" | "3xl";

export interface DisplayProps extends Base {
  as?: ElementType;
  size?: DisplaySize;
  children?: ReactNode;
}

export function Display({ as: As = "h2", size = "xl", className, ...rest }: DisplayProps) {
  return <As className={cx("ui-display", `ui-display--${size}`, className)} {...rest} />;
}

export type TextSize = "xs" | "sm" | "base" | "lg";
export type TextTone = "default" | "muted" | "faint" | "ghost" | "fault";

export interface TextProps extends Base {
  as?: ElementType;
  size?: TextSize;
  tone?: TextTone;
  /// Truncate to one line with an ellipsis.
  truncate?: boolean;
  children?: ReactNode;
}

export function Text({
  as: As = "p",
  size = "base",
  tone = "default",
  truncate = false,
  className,
  ...rest
}: TextProps) {
  return (
    <As
      className={cx(
        "ui-text",
        `ui-text--${size}`,
        tone !== "default" && `ui-text--${tone}`,
        truncate && "ui-truncate",
        className,
      )}
      {...rest}
    />
  );
}

export type MetaSize = "xs" | "sm";
export type MetaTone = "default" | "ink" | "ghost";

export interface MetaProps extends Base {
  as?: ElementType;
  size?: MetaSize;
  tone?: MetaTone;
  truncate?: boolean;
  children?: ReactNode;
}

/// Names, counts and timers. The subhead face carries `tnum`, so a column of
/// figures holds its width as it ticks.
export function Meta({
  as: As = "span",
  size = "sm",
  tone = "default",
  truncate = false,
  className,
  ...rest
}: MetaProps) {
  return (
    <As
      className={cx(
        "ui-meta",
        `ui-meta--${size}`,
        tone !== "default" && `ui-meta--${tone}`,
        truncate && "ui-truncate",
        className,
      )}
      {...rest}
    />
  );
}
