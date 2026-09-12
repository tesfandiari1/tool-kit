import { useEffect, useId, useRef, type ReactNode } from "react";
import { Label } from "./Text";
import "./Sheet.css";

export interface SheetProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  /// A strip across the band the card leaves clear. A modal dialog makes the
  /// rest inert, so the window's drag region has to come in here.
  head?: ReactNode;
  /// The title row's trailing controls, where a sheet's close belongs. Icons
  /// arrive as elements: nothing in this library picks an icon set.
  titleActions?: ReactNode;
  /// Inside the dialog, because the top layer sits above every z-index and a
  /// fixed toast outside it is invisible.
  overlay?: ReactNode;
}

/// A macOS sheet, on a native `<dialog>` with `showModal()`: the focus trap,
/// Escape, inertness and the top layer, with no dependency. The scrim is a real
/// child, because custom properties do not resolve inside `::backdrop`.
/// Clicking it does nothing, as in AppKit.
export function Sheet({
  open,
  onClose,
  title,
  children,
  head,
  titleActions,
  overlay,
}: SheetProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const restoreTo = useRef<HTMLElement | null>(null);
  const titleId = useId();

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (open) {
      if (el.open) return;
      restoreTo.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      el.showModal();
      return;
    }
    if (!el.open) return;
    el.close();
    // Closing drops focus on <body>, which ends keyboard navigation.
    restoreTo.current?.focus();
    restoreTo.current = null;
  }, [open]);

  return (
    <dialog
      ref={ref}
      className="ui-sheet"
      aria-labelledby={titleId}
      /// Route `cancel` to the caller, or `open` and the DOM disagree and the
      /// opener stops working.
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      {open && (
        <>
          <div className="ui-sheet__scrim" />
          <div className="ui-sheet__card">
            <div className="ui-sheet__title">
              <h2 id={titleId} className="ui-sheet__heading">
                <Label tone="strong">{title}</Label>
              </h2>
              {titleActions !== undefined && (
                <div className="ui-sheet__actions">{titleActions}</div>
              )}
            </div>
            <div className="ui-sheet__body">{children}</div>
          </div>
          {head !== undefined && <div className="ui-sheet__head">{head}</div>}
          {/* Outside the card: its transform would be the containing block
              for a fixed toast. */}
          {overlay}
        </>
      )}
    </dialog>
  );
}
