import React from 'react';
export function Rule({ orientation = 'horizontal', tone = 'ink', soft = false, style }) {
  const color = soft ? (tone === 'bone' ? 'var(--rule-on-dark)' : tone === 'slate' ? 'var(--rule-on-accent)' : 'var(--rule-soft)') : (tone === 'bone' ? 'var(--bone)' : 'var(--rule)');
  const h = orientation === 'horizontal';
  return <div role="separator" aria-orientation={orientation} style={{ background: color, width: h ? '100%' : 'var(--rule-w)', height: h ? 'var(--rule-w)' : '100%', alignSelf: 'stretch', flex: 'none', ...style }} />;
}
