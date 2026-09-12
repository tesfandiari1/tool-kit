import { useRef, type ReactNode } from "react";
import { cx } from "../cx";
import "./Tabs.css";

export interface TabItem {
  id: string;
  label: string;
  /// The mark's slot is held at every state (UI.md rule 2).
  dirty?: boolean;
}

export interface TabsProps {
  items: TabItem[];
  value: string;
  onChange: (id: string) => void;
  /// Omit for permanent tabs. Present, each answers Delete and Backspace.
  onClose?: (id: string) => void;
  /// Names the strip for assistive technology.
  label?: string;
  /// Rendered after the last tab.
  end?: ReactNode;
  className?: string;
}

/// A strip of open documents, and a real `tablist` unlike `Segmented`, because
/// each tab controls a panel. The caller owns that panel and connects it with
/// `id={`ui-tabpanel-${id}`}` and `aria-labelledby={`ui-tab-${id}`}`.
///
/// Keyboard follows the APG, and activation follows focus because switching a
/// document is instant.
export function Tabs({ items, value, onChange, onClose, label, end, className }: TabsProps) {
  const ref = useRef<HTMLDivElement>(null);

  const focusAt = (i: number) => {
    ref.current?.querySelectorAll<HTMLButtonElement>("[role='tab']")[i]?.focus();
  };

  const move = (delta: number, from: number) => {
    const next = (from + delta + items.length) % items.length;
    onChange(items[next].id);
    focusAt(next);
  };

  return (
    <div
      ref={ref}
      className={cx("ui-tabs", className)}
      role="tablist"
      aria-label={label}
      onKeyDown={(e) => {
        const i = items.findIndex((t) => t.id === value);
        if (i < 0) return;
        if (e.key === "ArrowRight") {
          e.preventDefault();
          move(1, i);
        } else if (e.key === "ArrowLeft") {
          e.preventDefault();
          move(-1, i);
        } else if (e.key === "Home") {
          e.preventDefault();
          move(-i, i);
        } else if (e.key === "End") {
          e.preventDefault();
          move(items.length - 1 - i, i);
        } else if ((e.key === "Delete" || e.key === "Backspace") && onClose) {
          e.preventDefault();
          onClose(items[i].id);
          // Focus does not survive the unmount: land on the next tab.
          const land = Math.min(i, items.length - 2);
          if (land >= 0) requestAnimationFrame(() => { focusAt(land); });
        }
      }}
    >
      {items.map((t) => {
        const active = t.id === value;
        return (
          <div key={t.id} className={cx("ui-tabs__tab", active && "is-active")}>
            <button
              type="button"
              role="tab"
              id={`ui-tab-${t.id}`}
              aria-selected={active}
              aria-controls={`ui-tabpanel-${t.id}`}
              /// Roving tabindex: the strip is one stop, arrows move within it.
              tabIndex={active ? 0 : -1}
              className="ui-tabs__label"
              title={t.label}
              onClick={() => { onChange(t.id); }}
            >
              {t.label}
            </button>
            {/* One slot, two occupants. The dot is the resting state and the
                close control takes its place on hover, so the tab's width is
                identical whether it is clean, dirty, or being closed. */}
            <span className="ui-tabs__mark" aria-hidden={!t.dirty}>
              <span
                className={cx("ui-tabs__dot", t.dirty && "is-dirty")}
                role={t.dirty ? "img" : undefined}
                aria-label={t.dirty ? "Unsaved changes" : undefined}
              />
              {onClose && (
                <button
                  type="button"
                  className="ui-tabs__x"
                  /// Out of the tab order: Delete on the tab does this.
                  tabIndex={-1}
                  aria-label={`Close ${t.label}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onClose(t.id);
                  }}
                >
                  <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" strokeWidth="2.25" strokeLinecap="round" aria-hidden>
                    <path d="M5 5l14 14M19 5L5 19" />
                  </svg>
                </button>
              )}
            </span>
          </div>
        );
      })}
      {end && <div className="ui-tabs__end">{end}</div>}
    </div>
  );
}
