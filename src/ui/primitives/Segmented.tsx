import { useRef } from "react";
import type { ReactNode } from "react";
import { cx } from "../cx";
import "./Segmented.css";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  /// Optional leading glyph. Receives whether this option is selected, so an
  /// icon set with filled and outline weights can reflect the state.
  icon?: (selected: boolean) => ReactNode;
  /// Trailing count. Its slot is held at every value, so selecting an option
  /// never shifts the label beside it.
  count?: number;
}

export interface SegmentedProps<T extends string> {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  /// Names the group for assistive technology.
  label?: string;
  className?: string;
}

/// Mutually exclusive modes, shown side by side.
///
/// A `radiogroup`, deliberately not a `tablist`. The ARIA tabs pattern promises
/// a tabpanel each tab controls, and nothing here has one: callers use this to
/// switch a persisted mode, not to reveal a panel. Announcing "tab 1 of 2" for
/// something that shows no panel is a worse lie than the extra keystroke a
/// radio group costs.
///
/// Keyboard model matches the radio pattern: one stop in the tab order (roving
/// tabindex), then arrows move and select. Home and End jump to the ends.
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
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
      className={cx("ui-seg", className)}
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
