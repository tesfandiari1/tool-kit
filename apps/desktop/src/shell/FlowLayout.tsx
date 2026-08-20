import type { ReactNode } from "react";

/// Left-column layout: head, body, optional foot in a vertical stack.
///
/// Compact mode: the native window grows to fit (`useFitWindow`); no internal
/// scroll. Workspace variant passes children through flat so `.flow--workspace`
/// rules can rank the queue and compress controls inside a fixed 100vh frame.
export function FlowLayout({
  variant = "default",
  head,
  foot,
  children,
  className,
}: {
  variant?: "default" | "workspace";
  head?: ReactNode;
  foot?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  const mainClass = ["flow", variant === "workspace" ? "flow--workspace" : "", className]
    .filter(Boolean)
    .join(" ");

  if (variant === "workspace") {
    return <main className={mainClass}>{children}</main>;
  }

  return (
    <main className={mainClass}>
      {head && <div className="flow-head">{head}</div>}
      <div className="flow-scroll">{children}</div>
      {foot && <div className="flow-foot">{foot}</div>}
    </main>
  );
}
