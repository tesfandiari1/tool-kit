import React from 'react';
export function Radio({ name, value, label, checked = false, onChange, disabled = false, tone = 'ink' }) {
  const dark = tone === 'bone';
  const fg = dark ? 'var(--bone)' : 'var(--ink)';
  return (
    <label style={{ display: 'inline-flex', alignItems: 'center', gap: 12, cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.45 : 1, fontFamily: 'var(--font-body)', fontWeight: 300, fontSize: 'var(--text-body-md)', color: fg, minHeight: 44 }}>
      <input type="radio" name={name} value={value} checked={checked} onChange={e => onChange && onChange(value, e)} disabled={disabled} style={{ position: 'absolute', opacity: 0, width: 0, height: 0 }} />
      <span aria-hidden style={{ width: 18, height: 18, boxSizing: 'border-box', border: '1px solid ' + fg, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flex: 'none' }}><span style={{ width: 8, height: 8, background: fg, opacity: checked ? 1 : 0, transition: 'opacity var(--dur) var(--ease)' }} /></span>
      {label && <span>{label}</span>}
    </label>
  );
}
export function RadioGroup({ name, options = [], value, onChange, direction = 'column', tone }) {
  return <div role="radiogroup" style={{ display: 'flex', flexDirection: direction, gap: direction === 'row' ? 32 : 0 }}>{options.map(o => { const v = typeof o === 'string' ? o : o.value; const l = typeof o === 'string' ? o : o.label; return <Radio key={v} name={name} value={v} label={l} checked={value === v} onChange={onChange} tone={tone} />; })}</div>;
}
