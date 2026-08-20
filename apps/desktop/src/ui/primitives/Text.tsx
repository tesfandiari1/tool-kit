import type { ElementType, HTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import "./Text.css";

/// Typography primitives. Three families, three jobs, no overlap:
///   <Label>   mono uppercase tracked — every section heading and control label
///   <Display> Instrument Serif — panel titles and headline numbers
///   <Text>    SF Pro — all prose
///   <Mono>    JetBrains Mono — paths, counts, timers, identifiers

type Base = HTMLAttributes<HTMLElement>;

export type LabelTone = "default" | "strong" | "accent" | "live" | "fault";

export interface LabelProps extends Base {
  as?: ElementType;
  tone?: LabelTone;
  /// Set when rendering `as="label"`, to bind the label to its control.
  htmlFor?: string;
  children?: ReactNode;
}

export function Label({ as: As = "span", tone = "default", className, ...rest }: LabelProps) {
  return <As className={cx("ui-label", tone !== "default" && `ui-label--${tone}`, className)} {...rest} />;
}

/// Serif sizes start at `lg`. Instrument Serif is drawn for display and muddies
/// at UI scale, so there is deliberately no `sm`.
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
export type TextTone = "default" | "muted" | "faint" | "ghost" | "pass" | "fault";

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

export type MonoSize = "xs" | "sm" | "base";
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
