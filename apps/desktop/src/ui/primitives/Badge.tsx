import type { HTMLAttributes } from "react";
import { cx } from "../cx";
import "./Badge.css";

export type Tone = "neutral" | "pass";

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

export type DotTone = "idle" | "queued" | "live" | "pass" | "fault";

export interface StatusDotProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: DotTone;
  /// Announced to screen readers, since the dot itself carries no text.
  label?: string;
}

/// The dot occupies the same box at every status, so a list of jobs never
/// reflows as rows change state. It knows nothing about the app's job
/// statuses: callers map their own domain state onto a tone.
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
