import type { ElementType, HTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import "./Text.css";

/// Three families, three jobs: Label for every heading and control, Display
/// for panel titles, Text for prose, Mono for anything scanned in a column.

type Base = HTMLAttributes<HTMLElement>;

export type LabelTone = "default" | "strong" | "accent";

export interface LabelProps extends Base {
  as?: ElementType;
  tone?: LabelTone;
  htmlFor?: string;
  children?: ReactNode;
}

export function Label({ as: As = "span", tone = "default", className, ...rest }: LabelProps) {
  return <As className={cx("ui-label", tone !== "default" && `ui-label--${tone}`, className)} {...rest} />;
}

/// No `sm`: Instrument Serif muddies at UI scale.
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

export type MonoSize = "xs" | "sm";
export type MonoTone = "default" | "ink" | "ghost";

export interface MonoProps extends Base {
  as?: ElementType;
  size?: MonoSize;
  tone?: MonoTone;
  truncate?: boolean;
  children?: ReactNode;
}

export function Mono({
  as: As = "span",
  size = "sm",
  tone = "default",
  truncate = false,
  className,
  ...rest
}: MonoProps) {
  return (
    <As
      className={cx(
        "ui-mono",
        `ui-mono--${size}`,
        tone !== "default" && `ui-mono--${tone}`,
        truncate && "ui-truncate",
        className,
      )}
      {...rest}
    />
  );
}
