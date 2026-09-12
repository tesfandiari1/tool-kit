import React, { useState } from 'react';
export function Input({ id, value, defaultValue, onChange, placeholder, type = 'text', multiline = false, rows = 4, disabled = false, invalid = false, tone = 'ink', style }) {
  const [focus, setFocus] = useState(false);
  const dark = tone === 'bone';
  const border = invalid ? 'var(--status-danger)' : focus ? (dark ? 'var(--bone)' : 'var(--blue-deep)') : (dark ? 'var(--rule-on-dark)' : 'var(--rule)');
  const s = { boxSizing: 'border-box', width: '100%', height: multiline ? 'auto' : 'var(--control-h)', minHeight: multiline ? rows * 24 + 20 : undefined, padding: multiline ? '10px 14px' : '0 14px', fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-md)', lineHeight: 1.5, color: dark ? 'var(--text-on-dark-strong)' : 'var(--ink)', background: dark ? 'rgba(239,227,214,0.06)' : 'var(--bone-lift)', border: '1px solid ' + border, borderRadius: 0, outline: 'none', boxShadow: 'none', opacity: disabled ? 0.45 : 1, resize: 'vertical', transition: 'border-color var(--dur) var(--ease)', ...style };
  const p = { id, value, defaultValue, onChange, placeholder, disabled, 'aria-invalid': invalid || undefined, onFocus: () => setFocus(true), onBlur: () => setFocus(false), style: s };
  return multiline ? <textarea rows={rows} {...p} /> : <input type={type} {...p} />;
}
