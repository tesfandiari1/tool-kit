import React, { useState } from 'react';
export function IconButton({ children, label, variant = 'secondary', size = 'md', disabled = false, onClick, style }) {
  const [hover, setHover] = useState(false);
  const on = hover && !disabled;
  const d = size === 'sm' ? 36 : 44;
  const v = {
    primary: { bg: on ? 'var(--btn-primary-hover-bg)' : 'var(--ink)', fg: 'var(--bone)', border: on ? 'var(--btn-primary-hover-bg)' : 'var(--ink)' },
    secondary: { bg: on ? 'var(--ink)' : 'transparent', fg: on ? 'var(--bone)' : 'var(--ink)', border: 'var(--ink)' },
    inverse: { bg: on ? 'var(--bone)' : 'transparent', fg: on ? 'var(--ink)' : 'var(--bone)', border: 'var(--bone)' },
    ghost: { bg: 'transparent', fg: on ? 'var(--link-hover)' : 'var(--ink)', border: 'transparent' },
  }[variant];
  return <button type="button" aria-label={label} title={label} disabled={disabled} onClick={onClick} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
    style={{ width: d, height: d, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', padding: 0, background: v.bg, color: v.fg, border: '1px solid ' + v.border, borderRadius: 0, cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.4 : 1, transition: 'background var(--dur) var(--ease), color var(--dur) var(--ease)', fontFamily: 'var(--font-subhead)', fontSize: 16, lineHeight: 1, ...style }}>{children}</button>;
}
