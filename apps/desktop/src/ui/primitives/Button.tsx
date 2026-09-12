import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import "./Button.css";

export type ButtonVariant = "primary" | "quiet" | "ghost" | "link";
/// `lg` is reserved for a view's single actuator. See Button.css.
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "type"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /// Fill the container's width.
  block?: boolean;
  /// Work is in flight. Disables the button but keeps it at full opacity in the
  /// live colour, because a greyed-out button reads as unavailable, not busy.
  busy?: boolean;
  /// Renders square with no text padding. Requires `aria-label`.
  iconOnly?: boolean;
  icon?: ReactNode;
  type?: "button" | "submit" | "reset";
}

export function Button({
  variant = "quiet",
  size = "md",
  block = false,
  busy = false,
  iconOnly = false,
  icon,
  disabled,
  className,
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      disabled={disabled ?? busy}
      aria-busy={busy || undefined}
      className={cx(
        "ui-btn",
        `ui-btn--${variant}`,
        `ui-btn--${size}`,
        block && "ui-btn--block",
        busy && "ui-btn--busy",
        iconOnly && "ui-btn--icon",
        className,
      )}
      {...rest}
    >
      {icon && <span className="ui-btn__icon">{icon}</span>}
      {!iconOnly && children}
    </button>
  );
}
