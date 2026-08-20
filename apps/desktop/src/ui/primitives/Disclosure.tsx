import type { ReactNode } from "react";
import { cx } from "../cx";
import "./Disclosure.css";

export interface DisclosureProps {
  open: boolean;
  onToggle: (open: boolean) => void;
  children: ReactNode;
  className?: string;
}

/// The trigger half of a disclosure. It deliberately does not own the content:
/// callers render the section themselves, so the collapsed branch can skip
/// mounting expensive children rather than hiding them with CSS.
export function Disclosure({ open, onToggle, children, className }: DisclosureProps) {
  return (
    <button
      type="button"
      aria-expanded={open}
      className={cx("ui-disclosure", className)}
      onClick={() => {
        onToggle(!open);
      }}
    >
      {children}
    </button>
  );
}
