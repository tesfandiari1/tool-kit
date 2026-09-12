import React, { useState } from 'react';
export function Card({ label, title, children, image, meta, href, tone = 'bone', padding = 32, style }) {
  const [hover, setHover] = useState(false);
  const dark = tone === 'ink';
  const slate = tone === 'slate';
  const bg = dark ? 'var(--ink)' : slate ? 'var(--blue)' : 'var(--bone)';
  const fg = dark ? 'var(--text-on-dark)' : 'var(--ink)';
  const mute = dark ? 'var(--text-on-dark-mute)' : slate ? 'var(--text-on-accent-mute)' : 'var(--text-mute)';
  const border = dark ? 'var(--rule-on-dark)' : 'var(--rule)';
  const Tag = href ? 'a' : 'div';
  return (
    <Tag href={href} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)} style={{ display: 'flex', flexDirection: 'column', background: bg, color: fg, border: '1px solid ' + border, borderRadius: 0, boxShadow: 'none', textDecoration: 'none', overflow: 'hidden', ...style }}>
      {image && <div style={{ position: 'relative', aspectRatio: '16 / 10', background: '#1a1b1f url(' + image + ') center/cover', borderBottom: '1px solid ' + border }}><div style={{ position: 'absolute', inset: 0, background: hover && href ? 'rgba(20,20,22,0.4)' : 'var(--photo-wash)', transition: 'background var(--dur) var(--ease)' }} /></div>}
      <div style={{ padding, display: 'flex', flexDirection: 'column', gap: 12, flex: 1 }}>
        {label && <div style={{ fontFamily: 'var(--font-display)', fontSize: 'var(--text-label)', letterSpacing: 'var(--track-label)', textTransform: 'uppercase', color: mute }}>{label}</div>}
        {title && <div style={{ fontFamily: 'var(--font-subhead)', fontWeight: 500, fontSize: 'var(--text-subhead-md)', lineHeight: 1.3, color: hover && href ? (dark ? 'var(--bone-lift)' : 'var(--blue-deep)') : fg, transition: 'color var(--dur) var(--ease)' }}>{title}</div>}
        {children && <div style={{ fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-sm)', lineHeight: 1.6, color: dark ? 'var(--text-on-dark)' : slate ? 'var(--ink)' : 'var(--text-soft)' }}>{children}</div>}
        {meta && <div style={{ marginTop: 'auto', paddingTop: 8, fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 12, letterSpacing: '0.04em', color: mute }}>{meta}</div>}
      </div>
    </Tag>
  );
}
