import React, { useState } from 'react';
export function Tabs({ items = [], value, defaultValue, onChange, tone = 'ink', style }) {
  const [inner, setInner] = useState(defaultValue ?? (items[0] && (items[0].value ?? items[0])));
  const cur = value ?? inner;
  const dark = tone === 'bone';
  const fg = dark ? 'var(--bone)' : 'var(--ink)';
  const mute = dark ? 'var(--text-on-dark-mute)' : 'var(--text-mute)';
  return (
    <div role="tablist" style={{ display: 'flex', borderBottom: '1px solid ' + (dark ? 'var(--rule-on-dark)' : 'var(--rule)'), ...style }}>
      {items.map(it => { const v = it.value ?? it; const l = it.label ?? it; const on = v === cur; return (
        <button key={v} role="tab" aria-selected={on} type="button" onClick={() => { setInner(v); onChange && onChange(v); }} style={{ all: 'unset', cursor: 'pointer', padding: '14px 20px', marginBottom: -1, fontFamily: 'var(--font-subhead)', fontWeight: 500, fontSize: 'var(--text-label)', letterSpacing: 'var(--track-eyebrow)', textTransform: 'uppercase', lineHeight: 1, color: on ? fg : mute, borderBottom: '1px solid ' + (on ? fg : 'transparent'), transition: 'color var(--dur) var(--ease)' }}>{l}</button>); })}
    </div>
  );
}
