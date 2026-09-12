import { useRef } from "react";
import type { ReactNode } from "react";
import { cx } from "../cx";
import "./Segmented.css";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  icon?: (selected: boolean) => ReactNode;
  /// Holds its slot at every value (UI.md rule 2).
  count?: number;
}

export interface SegmentedProps<T extends string> {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  label?: string;
  size?: "sm" | "md";
  className?: string;
}

/// Mutually exclusive modes, shown side by side.
///
/// A `radiogroup`, never a `tablist`: callers switch a persisted mode, and no
/// panel exists for a tab to control.
///
/// Keyboard model matches the radio pattern: one stop in the tab order (roving
/// tabindex), then arrows move and select. Home and End jump to the ends.
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
  size = "md",
  className,
}: SegmentedProps<T>) {
  const ref = useRef<HTMLDivElement>(null);

  const move = (delta: number, from: number) => {
    const next = (from + delta + options.length) % options.length;
    onChange(options[next].value);
    // Follow focus, which is what makes arrow navigation feel like a radio
    // group rather than a silent state change somewhere off screen.
    ref.current?.querySelectorAll<HTMLButtonElement>("[role='radio']")[next]?.focus();
  };

  return (
    <div
      ref={ref}
      className={cx("ui-seg", size === "sm" && "ui-seg--sm", className)}
      role="radiogroup"
      aria-label={label}
      onKeyDown={(e) => {
        const i = options.findIndex((o) => o.value === value);
        if (i < 0) return;
        if (e.key === "ArrowRight" || e.key === "ArrowDown") {
          e.preventDefault();
          move(1, i);
        } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
          e.preventDefault();
          move(-1, i);
        } else if (e.key === "Home") {
          e.preventDefault();
          move(-i, i);
        } else if (e.key === "End") {
          e.preventDefault();
          move(options.length - 1 - i, i);
        }
      }}
    >
      {options.map((o) => {
        const selected = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={selected}
            /// Roving tabindex: the group is one stop, arrows move within it.
            tabIndex={selected ? 0 : -1}
            className="ui-seg__item"
            onClick={() => {
              onChange(o.value);
            }}
          >
            {o.icon?.(selected)}
            {o.label}
            {o.count !== undefined && (
              <span className="ui-seg__count">{o.count > 0 ? o.count : ""}</span>
            )}
          </button>
        );
      })}
    </div>
  );
}
