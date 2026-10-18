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
  // The board's size, from a state snapshot (any size up to 12 × 40).
  dims(S) { return { rows: S.rows || 6, cols: S.cols || 22 }; },
  PRESETS: [['standard', 'Standard', 6, 22], ['compact', 'Compact', 3, 15], ['large', 'Large', 8, 30]],
  // The size set in Settings → Display: fixed, or the default for an adaptive board.
  sizeOf(s) {
    const p = RB.PRESETS.find(x => x[0] === s.model);
    return p ? { rows: p[2], cols: p[3] } : s.model === 'custom' ? { rows: s.rows, cols: s.cols } : { rows: 6, cols: 22 };
  },
  // Where a message can go: this board, all boards, a group or one saved board.
  targets(S) {
    return [['this', 'This board'], ...(S.boards.length ? [['all', 'All boards']] : []),
      ...(S.groups || []).map(g => ['group:' + g.id, g.name + ' (group)']), ...S.boards.map(b => [b.id, b.name])];
  },
  targetName(S, t) { const x = RB.targets(S).find(x => x[0] === t); return x ? x[1].replace(/ \(group\)$/, '') : 'this board'; },
  // The boards a target stands for, with their sizes, mirroring send.rs.
  destinations(S, t) {
    const here = { id: 'this', name: 'This board', ...RB.sizeOf(S.settings), adaptive: S.settings.adaptive };
    const one = id => (id === 'this' ? here : (b => b && { id: b.id, name: b.name, rows: b.rows, cols: b.cols, adaptive: !!b.adaptive, kind: b.kind })(S.boards.find(b => b.id === id)));
    const ids = t === 'all' ? ['this', ...S.boards.map(b => b.id)]
      : t.startsWith('group:') ? ((S.groups || []).find(g => 'group:' + g.id === t) || { members: [] }).members : [t];
    return [...new Set(ids)].map(one).filter(Boolean);
  },
  // A <select> of targets, grouped the way people think of them.
  targetOptions(S, cur) {
    const opt = ([v, l]) => `<option value="${RB.esc(v)}" ${v === cur ? 'selected' : ''}>${RB.esc(l.replace(/ \(group\)$/, ''))}</option>`;
    const t = RB.targets(S), groups = t.filter(x => x[0].startsWith('group:')), boards = t.filter(x => S.boards.some(b => b.id === x[0]));
    return t.filter(x => x[0] === 'this' || x[0] === 'all').map(opt).join('')
      + (groups.length ? `<optgroup label="Groups">${groups.map(opt).join('')}</optgroup>` : '')
      + (boards.length ? `<optgroup label="Boards">${boards.map(opt).join('')}</optgroup>` : '');
  },
  sizeName(rows, cols) {
    const p = RB.PRESETS.find(x => x[2] === rows && x[3] === cols);
    return `${p ? p[1] : 'Custom'} · ${rows} × ${cols}`;
  },
  fin(finish) {
    return finish === 'white'
      ? { frame: '#dcdad4', tile: '#fbfaf7', ink: '#1c1c1c', dim: '#9a9893' }
      : { frame: '#0a0a0a', tile: '#1c1c1c', ink: '#f0eee8', dim: '#6a6a6a' };
  },
  fit(grid, { rows, cols }) {
    return Array.from({ length: rows }, (_, r) => Array.from({ length: cols }, (_, c) => (grid[r] && grid[r][c]) || 0));
  },

  esc(s) { return String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]); },
  ago(ms) { const s = Math.max(0, Math.round(ms / 1000)); return s < 5 ? 'just now' : s < 60 ? s + 's ago' : s < 3600 ? Math.round(s / 60) + 'm ago' : Math.round(s / 3600) + 'h ago'; },
  fmtTime(t) { return new Date(t).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' }); },
  sourceLabel(S) {
    const s = S.settings;
    return s.source === 'local' ? 'Local · ' + s.host : s.source === 'demo' ? 'Demo mode' : s.source === 'virtual' ? 'API server' : 'Cloud';
  },
  // Clipboard with a fallback for webviews that don't expose the async API.
  copy(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) return navigator.clipboard.writeText(text).catch(() => RB._copy(text));
    return Promise.resolve(RB._copy(text));
  },
  _copy(text) {
    const t = document.createElement('textarea'); t.value = text; t.style.position = 'fixed'; t.style.opacity = '0';
    document.body.appendChild(t); t.select(); document.execCommand('copy'); t.remove();
  },
  mask(k) { return k ? '•'.repeat(8) + k.slice(-4) : 'not set'; },

  // The small read-only board used in the tray menu and setup ("mini" in the design).
  mini(grid, S) {
    const s = S.settings, d = RB.dims(S), fin = RB.fin(s.finish);
    // Tiles fill the popover's width, whatever the board's shape.
    const w = Math.max(4, Math.min(15, Math.floor((262 - (d.cols - 1) * 2) / d.cols))), h = Math.round(w * 1.32), f = (w * 0.85).toFixed(1) + 'px';
    const bg = c => (c >= 63 ? (c === 71 ? fin.ink : RB.HEX[c]) : fin.tile);
    return RB.fit(grid, d).map(r => `<div style="display:flex; gap:2px">${r.map(c =>
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
