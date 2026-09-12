import React from 'react';
export function Toast({ children, status = 'neutral', action, onDismiss, tone = 'ink', style }) {
  const dark = tone === 'ink';
  const c = { neutral: dark ? 'var(--bone-dim)' : 'var(--ink-mute)', info: 'var(--blue-pale)', success: 'var(--status-success)', warning: 'var(--status-warning)', danger: 'var(--status-danger)' }[status];
  return (
    <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 16, minHeight: 52, padding: '0 20px', background: dark ? 'var(--ink)' : 'var(--bone-lift)', color: dark ? 'var(--bone)' : 'var(--ink)', border: '1px solid ' + (dark ? 'var(--ink)' : 'var(--rule)'), borderLeft: '4px solid ' + c, borderRadius: 0, boxShadow: 'none', fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-sm)', lineHeight: 1.4, maxWidth: 480, ...style }}>
      <span style={{ flex: 1 }}>{children}</span>
      {action && <span style={{ fontFamily: 'var(--font-subhead)', fontWeight: 500, fontSize: 'var(--text-label-sm)', letterSpacing: 'var(--track-eyebrow)', textTransform: 'uppercase' }}>{action}</span>}
      {onDismiss && <button type="button" aria-label="Dismiss" onClick={onDismiss} style={{ all: 'unset', cursor: 'pointer', fontFamily: 'var(--font-subhead)', fontSize: 16, lineHeight: 1, padding: '0 0 0 4px' }}>×</button>}
    </div>
  );
}
