import React from 'react';
export function Dialog({ open = false, onClose, label, title, children, actions, width = 520 }) {
  if (!open) return null;
  return (
    <div role="presentation" onClick={onClose} style={{ position: 'fixed', inset: 0, background: 'var(--photo-wash-strong)', display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 24, zIndex: 100 }}>
      <div role="dialog" aria-modal="true" aria-label={typeof title === 'string' ? title : undefined} onClick={e => e.stopPropagation()} style={{ width: '100%', maxWidth: width, background: 'var(--bone-lift)', color: 'var(--ink)', border: '1px solid var(--rule)', borderRadius: 0, boxShadow: 'none', display: 'flex', flexDirection: 'column' }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', padding: '28px 32px 0', gap: 24 }}>
          <div>{label && <div style={{ fontFamily: 'var(--font-display)', fontSize: 'var(--text-label)', letterSpacing: 'var(--track-label)', textTransform: 'uppercase', color: 'var(--text-mute)', marginBottom: 12 }}>{label}</div>}{title && <h3 style={{ fontFamily: 'var(--font-display)', fontWeight: 400, fontSize: 'var(--text-display-sm)', lineHeight: 1.1, margin: 0 }}>{title}</h3>}</div>
          {onClose && <button type="button" aria-label="Close" onClick={onClose} style={{ all: 'unset', cursor: 'pointer', width: 36, height: 36, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', border: '1px solid var(--rule)', fontFamily: 'var(--font-subhead)', fontSize: 16, flex: 'none' }}>×</button>}
        </div>
        {children && <div style={{ padding: '20px 32px 28px', fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-md)', lineHeight: 1.6, color: 'var(--text-soft)' }}>{children}</div>}
        {actions && <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 12, padding: '20px 32px', borderTop: '1px solid var(--rule)' }}>{actions}</div>}
      </div>
    </div>
  );
}
