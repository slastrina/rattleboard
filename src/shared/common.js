// Helpers shared by the board, tray and panel windows.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const RB = {
  ACC: 'oklch(0.58 0.17 255)',
  SYM: { 37: '!', 38: '@', 39: '#', 40: '$', 41: '(', 42: ')', 44: '-', 46: '+', 47: '&', 48: '=', 49: ';', 50: ':', 52: "'", 53: '"', 54: '%', 55: ',', 56: '.', 59: '/', 60: '?', 62: '°' },
  HEX: { 63: '#d93a2f', 64: '#ec7a26', 65: '#f2c230', 66: '#2f9a52', 67: '#2b6bd1', 68: '#7d4cc2', 69: '#f3f1ec', 70: '#0d0d0d' },
  posNames: { tl: 'Top left', tr: 'Top right', bl: 'Bottom left', br: 'Bottom right' },

  invoke, listen,

  glyph(c) { return c >= 1 && c <= 26 ? String.fromCharCode(64 + c) : c >= 27 && c <= 35 ? String(c - 26) : c === 36 ? '0' : RB.SYM[c] || ''; },
  decode(g) { return g.map(r => r.map(c => (c >= 63 ? ' ' : RB.glyph(c) || ' ')).join('').trim()).filter(Boolean).join('  /  '); },
  dims(model) { return model === 'note' ? { rows: 3, cols: 15 } : { rows: 6, cols: 22 }; },
  fin(finish) {
    return finish === 'white'
      ? { frame: '#dcdad4', tile: '#fbfaf7', ink: '#1c1c1c', dim: '#9a9893' }
      : { frame: '#0a0a0a', tile: '#1c1c1c', ink: '#f0eee8', dim: '#6a6a6a' };
  },
  fit(grid, model) {
    const { rows, cols } = RB.dims(model);
    return Array.from({ length: rows }, (_, r) => Array.from({ length: cols }, (_, c) => (grid[r] && grid[r][c]) || 0));
  },

  esc(s) { return String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]); },
  ago(ms) { const s = Math.max(0, Math.round(ms / 1000)); return s < 5 ? 'just now' : s < 60 ? s + 's ago' : s < 3600 ? Math.round(s / 60) + 'm ago' : Math.round(s / 3600) + 'h ago'; },
  fmtTime(t) { return new Date(t).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' }); },
  mask(k) { return k ? '•'.repeat(8) + k.slice(-4) : 'not set'; },

  // The small read-only board used in the tray menu and setup ("mini" in the design).
  mini(grid, S) {
    const s = S.settings, flag = s.model !== 'note', fin = RB.fin(s.finish);
    const w = flag ? 10 : 15, h = flag ? 13 : 20, f = flag ? '8.5px' : '12px';
    const bg = c => (c >= 63 ? (c === 71 ? fin.ink : RB.HEX[c]) : fin.tile);
    return RB.fit(grid, s.model).map(r => `<div style="display:flex; gap:2px">${r.map(c =>
      `<div style="width:${w}px; height:${h}px; border-radius:1.5px; background:${bg(c)}; color:${fin.ink}; font-family:'Barlow Semi Condensed', sans-serif; font-weight:600; font-size:${f}; display:flex; align-items:center; justify-content:center; line-height:1">${RB.esc(c >= 63 ? '' : RB.glyph(c))}</div>`).join('')}</div>`).join('');
  },

  // Calls cb with every state snapshot from the backend, starting with the current one.
  onState(cb) {
    listen('rb://state', e => cb(e.payload));
    invoke('get_state').then(cb);
  },

  // Small DOM event delegation: data-act="name" -> handlers[name](el, event)
  delegate(root, type, handlers) {
    root.addEventListener(type, e => {
      const el = e.target.closest('[data-act]');
      if (el && root.contains(el) && handlers[el.dataset.act]) handlers[el.dataset.act](el, e);
    });
  }
};
window.RB = RB;
