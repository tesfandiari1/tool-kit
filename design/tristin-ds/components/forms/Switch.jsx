import React, { useState } from 'react';
export function Switch({ id, label, checked, defaultChecked = false, onChange, disabled = false, tone = 'ink' }) {
  const [inner, setInner] = useState(defaultChecked);
  const on = checked ?? inner;
  const dark = tone === 'bone';
  const fg = dark ? 'var(--bone)' : 'var(--ink)';
  const toggle = () => { if (disabled) return; setInner(!on); onChange && onChange(!on); };
  return (
    <label htmlFor={id} style={{ display: 'inline-flex', alignItems: 'center', gap: 12, cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.45 : 1, fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-md)', color: fg, minHeight: 44 }}>
      <button id={id} type="button" role="switch" aria-checked={on} disabled={disabled} onClick={toggle} style={{ width: 40, height: 22, boxSizing: 'border-box', padding: 2, border: '1px solid ' + fg, background: on ? fg : 'transparent', borderRadius: 0, cursor: 'inherit', display: 'flex', justifyContent: on ? 'flex-end' : 'flex-start', transition: 'background var(--dur) var(--ease)' }}>
        <span style={{ width: 16, height: 16, background: on ? (dark ? 'var(--ink)' : 'var(--bone)') : fg, display: 'block' }} />
      </button>
      {label && <span>{label}</span>}
    </label>
  );
}
