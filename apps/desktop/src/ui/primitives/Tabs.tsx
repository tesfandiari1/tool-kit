import { useRef, type ReactNode } from "react";
import { cx } from "../cx";
import "./Tabs.css";

export interface TabItem {
  id: string;
  label: string;
  /// Unsaved changes. The mark's slot is held at every state, so a document
  /// going dirty never resizes the tab or shifts the strip beside it.
  dirty?: boolean;
}

export interface TabsProps {
  items: TabItem[];
  value: string;
  onChange: (id: string) => void;
  /// Omit to make tabs permanent. When present, each tab gets a close control
  /// on hover and answers Delete and Backspace.
  onClose?: (id: string) => void;
  /// Names the strip for assistive technology.
  label?: string;
  /// Rendered after the last tab: an overflow chip, a count, a new-doc button.
  end?: ReactNode;
  className?: string;
}

/// A strip of open documents.
///
/// A real `tablist`, unlike `Segmented` — the distinction is whether a panel
/// exists. `Segmented` switches a persisted mode and shows no panel, so it
/// announces itself as a radio group. These tabs each control a document
/// panel, so the ARIA tabs pattern is the honest description.
///
/// The caller owns the panel and must connect it:
///
///     <Tabs items={docs} value={id} onChange={setId} />
///     <div role="tabpanel" id={`ui-tabpanel-${id}`} aria-labelledby={`ui-tab-${id}`}>
///
/// Keyboard follows the APG: one stop in the tab order, arrows move and
/// activate, Home and End jump to the ends, Delete closes when closable.
/// Activation follows focus because switching a document is cheap and
/// instant — the rule only bends when a panel costs a network call.
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
          // Focus does not survive the closed tab unmounting. Land on the tab
          // that slides into its place, or the last one if this was the end.
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
                  /// Out of the tab order on purpose: Delete on the tab is the
                  /// keyboard route, so this is one less stop per open document.
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
