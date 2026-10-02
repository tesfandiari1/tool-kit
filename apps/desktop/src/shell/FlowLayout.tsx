import type { ReactNode } from "react";

/// Left-column layout: head, body, optional foot in a vertical stack. The head
/// and foot hold still and the body scrolls between them.
export function FlowLayout({
  head,
  foot,
  children,
}: {
  head?: ReactNode;
  foot?: ReactNode;
  children: ReactNode;
}) {
  return (
    <main className="flow">
      {head && <div className="flow-head">{head}</div>}
      <div className="flow-scroll">{children}</div>
      {foot && <div className="flow-foot">{foot}</div>}
    </main>
  );
}
