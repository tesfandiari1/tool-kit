import React, { useState } from 'react';
export function Tag({ children, selected = false, onClick, onRemove, tone = 'ink' }) {
  const [hover, setHover] = useState(false);
  const dark = tone === 'bone';
  const fg = dark ? 'var(--bone)' : 'var(--ink)';
  const on = selected || (hover && onClick);
  return <span onClick={onClick} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)} style={{ display: 'inline-flex', alignItems: 'center', gap: 10, height: 28, padding: '0 12px', border: '1px solid ' + (dark ? 'var(--rule-on-dark)' : 'var(--rule)'), background: on ? fg : 'transparent', color: on ? (dark ? 'var(--ink)' : 'var(--bone)') : fg, fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-sm)', lineHeight: 1, cursor: onClick ? 'pointer' : 'default', transition: 'background var(--dur) var(--ease), color var(--dur) var(--ease)', userSelect: 'none' }}>{children}{onRemove && <button type="button" aria-label="Remove" onClick={e => { e.stopPropagation(); onRemove(); }} style={{ all: 'unset', cursor: 'pointer', fontFamily: 'var(--font-subhead)', fontSize: 14, lineHeight: 1 }}>×</button>}</span>;
}
