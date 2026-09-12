import React from 'react';
export function Wordmark({ size = 'md', tagline, tone = 'ink', align = 'center', as = 'div' }) {
  const sizes = { sm: 'var(--text-wordmark-sm)', md: 'clamp(22px, 3.4vw, var(--text-wordmark))', lg: 'clamp(28px, 5.4vw, var(--text-wordmark-lg))' };
  const color = tone === 'bone' ? 'var(--text-on-dark-strong)' : 'var(--ink)';
  const mute = tone === 'bone' ? 'var(--text-on-dark-mute)' : 'var(--ink-soft)';
  const Tag = as;
  return (
    <Tag style={{ display: 'flex', flexDirection: 'column', alignItems: align === 'center' ? 'center' : 'flex-start', textAlign: align, color }}>
      <span style={{ fontFamily: 'var(--font-display)', fontWeight: 400, fontSize: sizes[size], letterSpacing: 'var(--track-wordmark)', textTransform: 'uppercase', lineHeight: 1, paddingLeft: align === 'center' ? 'var(--track-wordmark)' : 0, whiteSpace: 'nowrap' }}>Tristin Esfandiari</span>
      {tagline && <span style={{ fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: size === 'lg' ? 15 : size === 'md' ? 12 : 10, letterSpacing: 'var(--track-eyebrow)', marginTop: size === 'lg' ? 16 : 10, color: mute, textAlign: 'center', textWrap: 'balance' }}>{tagline}</span>}
    </Tag>
  );
}
