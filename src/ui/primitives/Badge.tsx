import type { HTMLAttributes } from "react";
import { cx } from "../cx";
import "./Badge.css";

export type Tone = "neutral" | "accent" | "live" | "pass" | "fault";

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: Tone;
  /// Square corners instead of a pill, for counts and format tags.
  square?: boolean;
}

export function Badge({ tone = "neutral", square = false, className, ...rest }: BadgeProps) {
  return (
    <span
      className={cx(
        "ui-badge",
        tone !== "neutral" && `ui-badge--${tone}`,
        square && "ui-badge--square",
        className,
      )}
      {...rest}
    />
  );
}

export type StatusTone = "idle" | "queued" | "live" | "pass" | "fault";

export interface StatusProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: StatusTone;
  /// Announced to assistive technology, since the glyph inside carries the
  /// meaning visually.
  label?: string;
}

/// A fixed-width slot that tints the glyph inside it. Use this over
/// `StatusDot` wherever the icon's *shape* carries meaning the colour cannot,
/// such as a spinner for work in flight.
export function Status({ tone = "idle", label, className, children, ...rest }: StatusProps) {
  return (
    <span
      role={label ? "img" : undefined}
      aria-label={label}
      className={cx("ui-status", tone !== "idle" && `ui-status--${tone}`, className)}
      {...rest}
    >
      {children}
    </span>
  );
}

export type DotTone = "idle" | "queued" | "live" | "pass" | "fault";

export interface StatusDotProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: DotTone;
  /// Announced to screen readers, since the dot itself carries no text.
  label?: string;
}

/// The dot occupies the same box at every status, so a list of jobs never
/// reflows as rows change state.
///
/// It deliberately knows nothing about the app's job statuses. Callers map
/// their own domain state onto a tone, which keeps this library free of any
/// dependency on the product's types.
export function StatusDot({ tone = "idle", label, className, ...rest }: StatusDotProps) {
  return (
    <span
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={cx("ui-dot", tone !== "idle" && `ui-dot--${tone}`, className)}
      {...rest}
    />
  );
}
