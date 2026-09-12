import React from 'react';
export function PhotoBand({ src, wash = 'normal', height = 480, align = 'center', children, style }) {
  const overlay = wash === 'strong' ? 'var(--photo-wash-strong)' : wash === 'none' ? 'transparent' : 'var(--photo-wash)';
  const justify = align === 'center' ? 'center' : align === 'end' ? 'flex-end' : 'flex-start';
  return (
    <section style={{ position: 'relative', minHeight: height, background: '#1a1b1f url(' + src + ') center/cover no-repeat', color: 'var(--text-on-dark)', display: 'flex', ...style }}>
      <div style={{ position: 'absolute', inset: 0, background: overlay }} />
      <div style={{ position: 'relative', flex: 1, display: 'flex', flexDirection: 'column', alignItems: align === 'center' ? 'center' : 'flex-start', justifyContent: justify, textAlign: align === 'center' ? 'center' : 'left', padding: 'var(--section-pad-y) var(--section-pad-x)' }}>{children}</div>
    </section>
  );
}
