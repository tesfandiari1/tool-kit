import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import "./Toast.css";

export type ToastTone = "info" | "danger";

export interface ToastProps extends HTMLAttributes<HTMLDivElement> {
  tone?: ToastTone;
  children?: ReactNode;
}

/// The one channel for a message that reaches no row. Fixed to the foot of the
/// window, so a caller renders it at the app root and nowhere else.
export function Toast({ tone = "info", className, ...rest }: ToastProps) {
  return (
    <div
      role="status"
      className={cx("ui-toast", tone !== "info" && `ui-toast--${tone}`, className)}
      {...rest}
    />
  );
}
