import React, { useState } from 'react';
export function Checkbox({ id, label, checked, defaultChecked = false, onChange, disabled = false, tone = 'ink' }) {
  const [inner, setInner] = useState(defaultChecked);
  const isOn = checked ?? inner;
  const dark = tone === 'bone';
  const fg = dark ? 'var(--bone)' : 'var(--ink)';
  const toggle = e => { if (disabled) return; setInner(!isOn); onChange && onChange(!isOn, e); };
  return (
    <label htmlFor={id} style={{ display: 'inline-flex', alignItems: 'center', gap: 12, cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.45 : 1, fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-md)', color: fg, minHeight: 44 }}>
      <input id={id} type="checkbox" checked={isOn} onChange={toggle} disabled={disabled} style={{ position: 'absolute', opacity: 0, width: 0, height: 0 }} />
      <span aria-hidden style={{ width: 18, height: 18, boxSizing: 'border-box', border: '1px solid ' + fg, background: isOn ? fg : 'transparent', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', color: dark ? 'var(--ink)' : 'var(--bone)', fontSize: 12, lineHeight: 1, transition: 'background var(--dur) var(--ease)', flex: 'none' }}>{isOn ? '✓' : ''}</span>
      {label && <span>{label}</span>}
    </label>
  );
}
