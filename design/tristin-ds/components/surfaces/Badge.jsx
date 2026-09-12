import React from 'react';
export function Badge({ children, status = 'neutral', tone = 'ink' }) {
  const c = { neutral: tone === 'bone' ? 'var(--bone-dim)' : 'var(--ink-mute)', info: 'var(--status-info)', success: 'var(--status-success)', warning: 'var(--status-warning)', danger: 'var(--status-danger)' }[status];
  return <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8, fontFamily: 'var(--font-subhead)', fontWeight: 500, fontSize: 'var(--text-label-sm)', letterSpacing: 'var(--track-eyebrow)', textTransform: 'uppercase', lineHeight: 1, color: c }}><span aria-hidden style={{ width: 6, height: 6, background: c, flex: 'none' }} />{children}</span>;
}
