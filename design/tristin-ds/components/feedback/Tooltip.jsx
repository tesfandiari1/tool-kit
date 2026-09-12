import React, { useState } from 'react';
export function Tooltip({ content, children, side = 'top' }) {
  const [show, setShow] = useState(false);
  const pos = { top: { bottom: '100%', left: '50%', transform: 'translate(-50%, -8px)' }, bottom: { top: '100%', left: '50%', transform: 'translate(-50%, 8px)' }, left: { right: '100%', top: '50%', transform: 'translate(-8px, -50%)' }, right: { left: '100%', top: '50%', transform: 'translate(8px, -50%)' } }[side];
  return (
    <span style={{ position: 'relative', display: 'inline-flex' }} onMouseEnter={() => setShow(true)} onMouseLeave={() => setShow(false)} onFocus={() => setShow(true)} onBlur={() => setShow(false)}>
      {children}
      <span role="tooltip" style={{ position: 'absolute', ...pos, background: 'var(--ink)', color: 'var(--bone)', padding: '8px 12px', fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 12, lineHeight: 1.4, whiteSpace: 'nowrap', borderRadius: 0, pointerEvents: 'none', opacity: show ? 1 : 0, transition: 'opacity var(--dur) var(--ease)', zIndex: 10 }}>{content}</span>
    </span>
  );
}
