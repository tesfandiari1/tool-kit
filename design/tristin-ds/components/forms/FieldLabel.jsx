import React from 'react';
export function FieldLabel({ children, htmlFor, hint }) {
  return <label htmlFor={htmlFor} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', fontFamily: 'var(--font-subhead)', fontWeight: 500, fontSize: 'var(--text-label)', letterSpacing: 'var(--track-eyebrow)', textTransform: 'uppercase', color: 'var(--text-soft)', marginBottom: 8 }}><span>{children}</span>{hint && <span style={{ fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 12, letterSpacing: 0, textTransform: 'none', color: 'var(--text-mute)' }}>{hint}</span>}</label>;
}
