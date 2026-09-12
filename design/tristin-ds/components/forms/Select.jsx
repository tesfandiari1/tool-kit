import React, { useState } from 'react';
export function Select({ id, value, defaultValue, onChange, options = [], placeholder, disabled = false, tone = 'ink', style }) {
  const [focus, setFocus] = useState(false);
  const dark = tone === 'bone';
  const border = focus ? (dark ? 'var(--bone)' : 'var(--blue-deep)') : (dark ? 'var(--rule-on-dark)' : 'var(--rule)');
  return (
    <div style={{ position: 'relative', width: '100%', opacity: disabled ? 0.45 : 1, ...style }}>
      <select id={id} value={value} defaultValue={defaultValue ?? (placeholder ? '' : undefined)} onChange={onChange} disabled={disabled} onFocus={() => setFocus(true)} onBlur={() => setFocus(false)}
        style={{ boxSizing: 'border-box', width: '100%', height: 'var(--control-h)', padding: '0 40px 0 14px', fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-md)', color: dark ? 'var(--text-on-dark-strong)' : 'var(--ink)', background: dark ? 'rgba(239,227,214,0.06)' : 'var(--bone-lift)', border: '1px solid ' + border, borderRadius: 0, outline: 'none', appearance: 'none', WebkitAppearance: 'none', cursor: disabled ? 'not-allowed' : 'pointer', transition: 'border-color var(--dur) var(--ease)' }}>
        {placeholder && <option value="" disabled>{placeholder}</option>}
        {options.map(o => typeof o === 'string' ? <option key={o} value={o}>{o}</option> : <option key={o.value} value={o.value}>{o.label}</option>)}
      </select>
      <span aria-hidden style={{ position: 'absolute', right: 14, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none', fontFamily: 'var(--font-subhead)', fontSize: 12, color: dark ? 'var(--text-on-dark-mute)' : 'var(--text-mute)' }}>▾</span>
    </div>
  );
}
