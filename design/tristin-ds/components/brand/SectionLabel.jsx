import React from 'react';
export function SectionLabel({ children, tone = 'ink', align = 'center', style }) {
  const color = tone === 'bone' ? 'var(--text-on-dark-mute)' : tone === 'slate' ? 'var(--text-on-accent-mute)' : 'var(--text-mute)';
  return <div style={{ fontFamily: 'var(--font-display)', fontWeight: 400, fontSize: 'var(--text-label)', letterSpacing: 'var(--track-label)', textTransform: 'uppercase', lineHeight: 'var(--leading-label)', color, textAlign: align, paddingLeft: align === 'center' ? 'var(--track-label)' : 0, ...style }}>{children}</div>;
}
