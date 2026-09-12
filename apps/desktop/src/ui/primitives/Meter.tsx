import type { HTMLAttributes } from "react";
import { cx } from "../cx";
import "./Meter.css";

export interface MeterProps extends Omit<HTMLAttributes<HTMLDivElement>, "role"> {
  /// Completion from 0 to 1. Omit when the real figure is unknown: an
  /// invented percentage is a promise the job cannot keep, so leaving this
  /// undefined renders the indeterminate sweep instead.
  value?: number;
  label?: string;
}

export function Meter({ value, label, className, ...rest }: MeterProps) {
  const indeterminate = value === undefined;
  const clamped = indeterminate ? 0 : Math.min(1, Math.max(0, value));

  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={indeterminate ? undefined : 0}
      aria-valuemax={indeterminate ? undefined : 100}
      aria-valuenow={indeterminate ? undefined : Math.round(clamped * 100)}
      className={cx(
        "ui-meter",
        indeterminate && "ui-meter--indeterminate",
        className,
      )}
      {...rest}
    >
      <div
        className="ui-meter__fill"
        style={indeterminate ? undefined : { transform: `scaleX(${String(clamped)})` }}
      />
    </div>
  );
}
