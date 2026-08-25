import { useEffect, useId, useRef, type ReactNode } from "react";
import { Label } from "./Text";
import "./Sheet.css";

export interface SheetProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  /// A strip across the band the card leaves clear at the top. A modal dialog
  /// makes the rest of the document inert, and window dragging on macOS is a
  /// hit test on an attribute rather than a CSS state, so the app passes its
  /// drag region here or the window cannot be moved while the sheet is up.
  head?: ReactNode;
  /// Controls for the title row, pushed to its trailing edge. A sheet's close
  /// control belongs here rather than inside the panel: every sheet needs one,
  /// and a panel that draws its own leaves a second header band under the
  /// title with nothing else in it. Icons arrive as elements, because nothing
  /// in this library picks an icon set.
  titleActions?: ReactNode;
  footer?: ReactNode;
  /// Rendered inside the dialog element, beside the card. The top layer sits
  /// above every z-index, so a fixed toast outside the dialog is invisible
  /// while the sheet is open, and saving an API key is the most common thing
  /// Settings does.
  overlay?: ReactNode;
}

/// A macOS sheet over the whole window.
///
/// A native `<dialog>` driven by `showModal()`, which buys the focus trap,
/// Escape, inertness for everything behind it, and the top layer with no
/// dependency at all. That is UI.md's rule 5 answered by the platform.
///
/// The scrim is a real child rather than `::backdrop`, because custom-property
/// resolution inside `::backdrop` is not dependable across WebKit versions and
/// every colour in this system is a token.
///
/// The card floats clear of the window on all four sides. It clears `--bar-h`,
/// the app's title-bar height, so the traffic lights stay over live chrome.
/// Hanging it off that edge the way an AppKit sheet does read as clipped
/// instead: a square top with no border cannot be told apart from a card whose
/// top has scrolled out of the window.
///
/// Clicking the scrim does nothing, which is what an AppKit sheet does. Escape
/// and the card's own close control are the two ways out.
export function Sheet({
  open,
  onClose,
  title,
  children,
  head,
  titleActions,
  footer,
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
    // Closing a dialog drops focus on <body>, which silently ends keyboard
    // navigation wherever the user was. Put it back on the opener.
    restoreTo.current?.focus();
    restoreTo.current = null;
  }, [open]);

  return (
    <dialog
      ref={ref}
      className="ui-sheet"
      aria-labelledby={titleId}
      /// Escape arrives as `cancel`. Route it to the caller rather than letting
      /// the element close itself, or `open` and the DOM disagree and the next
      /// press of the opener does nothing.
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
            {footer !== undefined && <div className="ui-sheet__foot">{footer}</div>}
          </div>
          {head !== undefined && <div className="ui-sheet__head">{head}</div>}
          {/* Outside the card deliberately: the card animates with a transform,
              and a transform makes it the containing block for anything fixed
              inside it, which is exactly what a toast is. */}
          {overlay}
        </>
      )}
    </dialog>
  );
}
