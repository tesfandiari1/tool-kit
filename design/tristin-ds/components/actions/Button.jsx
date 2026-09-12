import React, { useState } from 'react';
export function Button({ children, variant = 'primary', size = 'md', disabled = false, href, onClick, type = 'button', style }) {
  const [hover, setHover] = useState(false);
  const [active, setActive] = useState(false);
  const on = hover && !disabled;
  const v = {
    primary: { bg: on ? 'var(--btn-primary-hover-bg)' : 'var(--btn-primary-bg)', fg: 'var(--btn-primary-fg)', border: on ? 'var(--btn-primary-hover-bg)' : 'var(--btn-primary-bg)' },
    secondary: { bg: on ? 'var(--btn-secondary-hover-bg)' : 'transparent', fg: on ? 'var(--btn-secondary-hover-fg)' : 'var(--btn-secondary-fg)', border: 'var(--btn-secondary-border)' },
    inverse: { bg: on ? 'var(--btn-inverse-hover-bg)' : 'transparent', fg: on ? 'var(--btn-inverse-hover-fg)' : 'var(--btn-inverse-fg)', border: 'var(--btn-inverse-border)' },
    ghost: { bg: 'transparent', fg: on ? 'var(--link-hover)' : 'var(--ink)', border: 'transparent' },
  }[variant];
  const s = {
    display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: 10, boxSizing: 'border-box',
    height: size === 'sm' ? 'var(--control-h-sm)' : 'var(--control-h)', padding: '0 ' + (size === 'sm' ? '16px' : variant === 'ghost' ? '0' : 'var(--control-pad-x)'),
    fontFamily: 'var(--font-subhead)', fontWeight: 500, fontSize: size === 'sm' ? 'var(--text-label-sm)' : 'var(--text-label)', letterSpacing: 'var(--track-label)', textTransform: 'uppercase', lineHeight: 1,
    color: v.fg, background: v.bg, border: '1px solid ' + v.border, borderRadius: 0, boxShadow: 'none', cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.4 : active && !disabled ? 0.85 : 1,
    textDecoration: 'none', transition: 'background var(--dur) var(--ease), color var(--dur) var(--ease), opacity var(--dur) var(--ease)', whiteSpace: 'nowrap', ...style,
  };
  const h = { onMouseEnter: () => setHover(true), onMouseLeave: () => { setHover(false); setActive(false); }, onMouseDown: () => setActive(true), onMouseUp: () => setActive(false) };
  if (href) return <a href={disabled ? undefined : href} aria-disabled={disabled} onClick={onClick} style={s} {...h}>{children}</a>;
  return <button type={type} disabled={disabled} onClick={onClick} style={s} {...h}>{children}</button>;
}
