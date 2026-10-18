// Compose: write a message or a sequence of frames, preview it in split flaps, and send it to
// this board, a saved one, a group or all of them. panel.js draws it like its other views
// (viewCompose).
const blankTr = () => ({ strategy: 'classic', stepIntervalMs: 120, stepSize: 1 });
const newItem = (mode = 'text') => ({ mode, text: '', grid: null, cells: null, hold: 8, tr: blankTr() });
const C = {
  target: 'this',
  msg: newItem(),
  // A sequence: when there are frames, the editor edits frames[sel].
  frames: [], sel: -1,
  name: '', loop: false,
  // 'type' places the cursor; any other value is a flap code to paint with.
  brush: 'type', cursor: { r: 0, c: 0 },
  drafts: [], draftId: '',
  status: null, busy: false,
  preview: null, pseq: 0,
  remote: null, kinds: {},
  recent: [], targetSet: false,
  // The message size for adaptive boards ("Match each message"), and how each board took a send
  size: null, deliveries: [], waiting: {}, lastBody: null, startedRemote: 0
};
const STRATEGY_NAMES = { classic: 'Classic (all at once)', column: 'Column', 'reverse-column': 'Reverse column', 'edges-to-center': 'Edges to centre', row: 'Row', diagonal: 'Diagonal', random: 'Random' };
const PALETTE = [0, ...Array.from({ length: 36 }, (_, i) => i + 1), ...Object.keys(RB.SYM).map(Number), 63, 64, 65, 66, 67, 68, 69, 70, 71];
// Typed characters → flap codes
const CODE = { ' ': 0, '0': 36 };
for (let c = 1; c <= 26; c++) CODE[String.fromCharCode(64 + c)] = c;
for (let i = 1; i <= 9; i++) CODE[String(i)] = 26 + i;
for (const k in RB.SYM) CODE[RB.SYM[k]] = +k;

const preview = document.createElement('split-flap-board');
preview.id = 'cmpBoard';

const cur = () => (C.sel >= 0 && C.frames[C.sel]) || C.msg;
// The saved board, when the target is one board; and every board the target stands for.
const board = () => S.boards.find(b => b.id === C.target);
const dests = () => RB.destinations(S, C.target);
const adaptiveAny = () => dests().some(d => d.adaptive);
// The editor's size: the size picked for adaptive boards, else the (first) board's own size.
const dims = () => {
  if (adaptiveAny()) return C.size || { rows: S.settings.sendRows, cols: S.settings.sendCols };
  const d = dests()[0];
  return d ? { rows: d.rows, cols: d.cols } : RB.sizeOf(S.settings);
};
const blankGrid = () => { const { rows, cols } = dims(); return Array.from({ length: rows }, () => Array(cols).fill(0)); };
// Centres a grid of another size on the target board, as the boards themselves do.
const fitTo = g => {
  const { rows, cols } = dims(), gr = (g || []).length, gc = gr ? g[0].length : 0;
  // floor((board - grid) / 2), as the boards do: an odd extra blank goes after, an odd crop off the start.
  const off = (n, m) => Math.floor((n - m) / 2);
  const dr = off(rows, gr), dc = off(cols, gc);
  return Array.from({ length: rows }, (_, r) => Array.from({ length: cols }, (_, c) => (g && g[r - dr] && g[r - dr][c - dc]) || 0));
};
const layoutOf = it => fitTo(it.mode === 'grid' ? it.cells : it.grid);
// Real Vestaboards can't take messages faster than this, so holds are lengthened to it.
const kindOf = b => C.kinds[b.id] || b.kind;
const holdOf = d => (!d || d.id === 'this' ? 1 : kindOf(d) === 'cloud' ? 15 : kindOf(d) === 'local' ? 5 : 1);
const minHold = () => Math.max(1, ...dests().map(holdOf));
const trBody = tr => (tr.strategy === 'classic' ? {} : { strategy: tr.strategy, step_interval_ms: tr.stepIntervalMs, step_size: tr.stepSize });
// Sequences this computer is playing for the target (one per board, for a group).
const playingHere = () => (S.playing || []).filter(p => p.via === C.target || p.target === C.target);
const isGroup = () => C.target === 'all' || C.target.startsWith('group:');

function relayout(it) {
  const { rows, cols } = dims();
  return RB.invoke('layout_text', { text: it.text, rows, cols }).then(g => { it.grid = g; });
}
const relayoutAll = () => Promise.all([C.msg, ...C.frames].filter(it => it.mode === 'text').map(relayout));

// ---------------------------------------------------------------- view

function viewCompose() {
  // Compose starts on the board last sent to.
  if (!C.targetSet) {
    C.targetSet = true; C.size = { rows: S.settings.sendRows, cols: S.settings.sendCols };
    const t = S.settings.sendTarget;
    if (RB.targets(S).some(x => x[0] === t) && t !== 'this') setTimeout(() => setTarget(t));
  }
  const it = cur(), { rows, cols } = dims(), b = board(), min = minHold(), ds = dests();
  const fixedOff = ds.filter(d => !d.adaptive && (d.rows !== rows || d.cols !== cols)).length;
  const where = isGroup() ? `${ds.length} board${ds.length === 1 ? '' : 's'}: ${ds.map(d => d.name).join(', ')}.${fixedOff ? (it.mode === 'text' ? ' Text is laid out for each board’s size.' : ` The grid is centred on ${fixedOff} board${fixedOff > 1 ? 's' : ''} of another size.`) : ''}`
    : !b ? (S.settings.adaptive ? 'Shows on this computer at the size you pick, like a message sent to its API server' : 'Shows on this computer, like a message sent to its API server')
    : kindOf(b) === 'rattleboard' ? `Rattleboard at ${b.host}${b.adaptive ? ', shows each message at its own size' : ''}: plays sequences by itself`
    : kindOf(b) === 'cloud' ? 'Vestaboard Cloud: this computer plays sequences, at least 15 s a frame'
    : `Vestaboard at ${b.host}: this computer plays sequences, at least 5 s a frame`;
  const sizeModel = C.customSize ? 'custom' : (RB.PRESETS.find(p => p[2] === rows && p[3] === cols) || ['custom'])[0];
  const sizePicker = adaptiveAny() ? `<div style="display:flex; align-items:flex-start; gap:10px"><span class="lbl" style="padding-top:4px">Message size</span>
      ${presetSeg(sizeModel, 'cmpSize', rows, cols, ['cmpRows', 'cmpCols'])}</div>` : '';
  const drafts = C.drafts.map(d => [d.id, d.name]);
  const tr = it.tr, classic = tr.strategy === 'classic';
  const showBoard = it.mode === 'text' || C.preview;
  const editor = showBoard ? '' : gridEditor();
  const textBox = it.mode === 'text' && !C.preview
    ? `<textarea class="field" data-cmp="text" rows="3" spellcheck="false" placeholder="Type a message. {r} {o} {y} {g} {b} {v} {w} {k} or {63}–{70} add colour chips." style="width:100%; height:auto; min-height:64px; padding:6px 8px; resize:vertical; line-height:1.4">${esc(it.text)}</textarea>
       <div class="hint" style="margin-top:-6px">Upper-cased, word-wrapped and centred, exactly as the board lays out text it's sent. A new line starts a new row.</div>` : '';
  return `<div class="hdr" style="display:flex; align-items:center; gap:10px; padding:14px 24px 12px; border-bottom:1px solid rgba(0,0,0,.08)">
      <span class="lbl">Send to</span>
      <select class="field" data-cmp="target" style="width:190px">${RB.targetOptions(S, C.target)}</select>
      <span class="hint" style="white-space:nowrap">${adaptiveAny() ? 'Any size' : RB.sizeName(rows, cols)}</span>
      <div style="flex:1"></div>
      <select class="field" data-cmp="draft" style="width:150px"><option value="">${C.draftId ? 'New draft' : 'Drafts…'}</option>${drafts.map(([v, l]) => `<option value="${esc(v)}" ${v === C.draftId ? 'selected' : ''}>${esc(l)}</option>`).join('')}</select>
      <button class="btn" data-act="cmpSaveDraft">${C.status && C.status.saved ? 'Saved' : 'Save draft'}</button>
      ${C.draftId ? `<button class="btn" data-act="cmpDeleteDraft" title="Delete this draft">Delete</button>` : ''}
    </div>
    <div class="scroll"><div style="padding:14px 24px 18px; display:flex; flex-direction:column; gap:12px">
      <div class="hint" style="margin-top:-2px">${esc(where)}</div>
      ${deliveryList()}
      ${sizePicker}
      <div style="display:flex; align-items:center; gap:10px">
        ${seg([['text', 'Text'], ['grid', 'Grid']], it.mode, 'cmpMode')}
        ${C.frames.length ? `<span class="hint">Editing frame ${C.sel + 1} of ${C.frames.length}</span>` : ''}
        <div style="flex:1"></div>
        ${showBoard ? `<button class="btn" data-act="cmpReplay" ${C.preview ? 'disabled' : ''}>Replay</button>` : `<button class="btn" data-act="cmpClear">Clear</button>`}
      </div>
      <div style="display:flex; justify-content:center; padding:14px 0; border-radius:10px; background:rgba(0,0,0,.05); overflow:hidden">
        ${showBoard ? `<div id="cmpPreview" style="min-height:${previewHeight(rows, cols)}px"></div>` : editor}
      </div>
      ${showBoard ? '' : palette()}
      ${textBox}
      <div style="display:flex; align-items:center; gap:10px; flex-wrap:wrap">
        <span class="lbl">Transition</span>
        <select class="field" data-cmp="strategy" style="width:190px">${Object.entries(STRATEGY_NAMES).map(([v, l]) => `<option value="${v}" ${v === tr.strategy ? 'selected' : ''}>${l}</option>`).join('')}</select>
        <span class="hint" style="opacity:${classic ? 0.5 : 1}">every</span>
        <input class="field" type="number" min="1" max="5000" step="10" style="width:76px" value="${tr.stepIntervalMs}" data-cmp="interval" ${classic ? 'disabled' : ''}>
        <span class="hint" style="opacity:${classic ? 0.5 : 1}">ms,</span>
        <input class="field" type="number" min="1" max="22" style="width:56px" value="${tr.stepSize}" data-cmp="size" ${classic ? 'disabled' : ''}>
        <span class="hint" style="opacity:${classic ? 0.5 : 1}">${tr.strategy === 'random' ? 'flaps' : tr.strategy.includes('row') && !tr.strategy.includes('column') ? 'rows' : tr.strategy === 'diagonal' ? 'diagonals' : 'columns'} at a time</span>
      </div>
      ${sequenceSection(min)}
      ${recentSection()}
    </div></div>
    <div class="hdr" style="display:flex; align-items:center; gap:8px; padding:12px 24px 16px; border-top:1px solid rgba(0,0,0,.06)">
      <div id="cmpStatus" style="flex:1; min-width:0; display:flex; align-items:center; gap:6px; color:#55575c; white-space:nowrap; overflow:hidden">${statusLine()}</div>
      <button class="btn" style="font-size:13px; padding:5px 12px" data-act="cmpPreviewPlay">${C.preview ? 'Stop preview' : 'Preview'}</button>
      ${playingHere().length || (C.remote && C.remote.running) || C.startedRemote ? `<button class="btn" style="font-size:13px; padding:5px 12px" data-act="cmpStop">${isGroup() ? 'Stop all' : 'Stop'}</button>` : ''}
      <button class="${C.frames.length ? 'btn' : 'primary'}" style="${C.frames.length ? 'font-size:13px; padding:5px 12px' : ''}" data-act="cmpSend" ${C.busy ? 'disabled' : ''}>${C.frames.length ? 'Send this frame' : 'Send'}</button>
      ${C.frames.length ? `<button class="primary" data-act="cmpPlay" ${C.busy ? 'disabled' : ''}>Play sequence</button>` : ''}
    </div>`;
}

function statusLine() {
  const ps = playingHere(), p = ps[0], r = C.remote, dot = c => `<div class="dot" style="background:${c}"></div>`;
  const label = (name, n, of) => `Playing ${name ? `‘${esc(name)}’` : 'sequence'} ${n}/${of}`;
  const waiting = Object.values(C.waiting);
  if (C.preview) return `${dot('#e0a526')}<span>Previewing here ${C.preview.n}/${C.preview.of}</span>`;
  if (C.busy && waiting.length) return `${dot('#e0a526')}<span>Queued for ${esc(waiting.join(', '))}: a Vestaboard needs a gap between messages</span>`;
  // One combined line for a group: boards this computer plays, plus Rattleboards playing it themselves.
  if (isGroup() && (ps.length || C.startedRemote)) return `${dot('#34b05a')}<span>Playing ${p && p.name ? `‘${esc(p.name)}’` : 'sequence'} on ${ps.length + C.startedRemote} board${ps.length + C.startedRemote > 1 ? 's' : ''}${p ? ` · ${p.frame}/${p.frames} here` : ''}</span>`;
  if (p) return `${dot('#34b05a')}<span>${label(p.name, p.frame, p.frames)} on ${esc(p.targetName)}${p.loop ? ' · loop' : ''}</span>`;
  if (r && r.running) return `${dot('#34b05a')}<span>${label(r.name, r.frame, r.frames)} on ${esc((board() || {}).name || 'the board')}${r.loop ? ' · loop' : ''}</span>`;
  if (C.status && C.status.text) return `${dot(C.status.error ? '#e0553f' : '#34b05a')}<span style="overflow:hidden; text-overflow:ellipsis; color:${C.status.error ? '#c0392b' : '#55575c'}">${esc(C.status.text)}</span>`;
  if (S.playError && C.target === 'this') return `${dot('#e0553f')}<span style="overflow:hidden; text-overflow:ellipsis">${esc(S.playError)}</span>`;
  return '';
}

// How each board took the last send: sent, queued, rate-limited or failed (with Retry).
function deliveryList() {
  const ds = C.deliveries;
  if (!ds.length || (ds.length === 1 && ds[0].status === 'sent')) return '';
  const color = { sent: '#34b05a', queued: '#e0a526', 'rate-limited': '#e0a526', failed: '#e0553f', sending: '#e0a526' };
  const word = { sent: 'Sent', queued: 'Queued', 'rate-limited': 'Rate-limited', failed: 'Failed', sending: 'Sending…' };
  return `<div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1)">${ds.map((d, i) => `
    <div style="display:flex; align-items:center; gap:10px; padding:7px 12px; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}">
      <div class="dot" style="background:${color[d.status] || '#9a9ca1'}"></div>
      <span style="font-weight:600; white-space:nowrap">${esc(d.name)}</span>
      <span class="hint" style="flex:1; min-width:0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis" title="${esc(d.error || '')}">${word[d.status] || esc(d.status)}${d.error ? ` · ${esc(d.error)}` : ''}</span>
      ${d.status === 'failed' || d.status === 'rate-limited' ? `<button class="btn" data-act="cmpRetry" data-v="${esc(d.target)}">Retry</button>` : ''}
    </div>`).join('')}</div>`;
}

function gridEditor() {
  const it = cur(), { rows, cols } = dims(), fin = RB.fin(S.settings.finish);
  if (!it.cells) it.cells = blankGrid();
  it.cells = fitTo(it.cells);
  // Cells fill the editor's width, whatever the board's shape.
  const w = Math.max(12, Math.min(40, Math.floor((640 - (cols - 1) * 3) / cols * 0.92))), h = Math.round(w * 1.36), fs = Math.round(h * 0.56);
  const html = it.cells.map((row, r) => `<div style="display:flex; gap:3px">${row.map((code, c) => {
    const at = C.cursor.r === r && C.cursor.c === c;
    return `<div class="cell flap-font" data-cell="${r},${c}" style="width:${w}px; height:${h}px; font-size:${fs}px; color:${fin.ink}; ${cellBg(code, fin)}${at ? `; box-shadow:inset 0 0 0 2px ${RB.ACC}` : ''}">${code >= 63 ? '' : esc(RB.glyph(code))}</div>`;
  }).join('')}</div>`).join('');
  return `<div id="cmpGrid" style="display:flex; flex-direction:column; gap:4px; padding:${Math.round(w * 0.9)}px; border-radius:${Math.round(w * 0.4)}px; background:${fin.frame}; touch-action:none">${html}</div>`;
}
const cellBg = (code, fin) => `background:${code >= 63 ? (code === 71 ? fin.ink : RB.HEX[code]) : fin.tile}`;

function palette() {
  const fin = RB.fin(S.settings.finish);
  const chip = code => {
    const on = C.brush === code;
    const title = code === 0 ? 'Blank' : code === 71 ? 'Filled' : code >= 63 ? 'Colour chip' : RB.glyph(code);
    return `<button class="pal flap-font ${on ? 'on' : ''}" data-act="cmpBrush" data-v="${code}" title="${esc(title)}" style="${cellBg(code, fin)}; color:${fin.ink}">${code >= 63 || code === 0 ? '' : esc(RB.glyph(code))}</button>`;
  };
  return `<div style="display:flex; flex-direction:column; gap:6px">
    <div style="display:flex; flex-wrap:wrap; gap:3px; align-items:center">
      <button class="pal tool ${C.brush === 'type' ? 'on' : ''}" data-act="cmpBrush" data-v="type" title="Click a cell, then type">Aa</button>
      ${PALETTE.map(chip).join('')}
    </div>
    <div class="hint">${C.brush === 'type' ? 'Click a cell and type. Arrow keys move, Backspace erases. Pick a flap or colour above to paint with it.' : 'Click or drag across cells to paint. Typing still fills from the highlighted cell.'}</div>
  </div>`;
}

function sequenceSection(min) {
  const fin = RB.fin(S.settings.finish);
  const rows = C.frames.map((f, i) => {
    const g = layoutOf(f), on = i === C.sel, short = f.hold < min;
    const thumb = g.map(r => `<div style="display:flex; gap:1px">${r.map(c => `<div style="width:4px; height:6px; border-radius:.5px; background:${c >= 63 ? (c === 71 ? fin.ink : RB.HEX[c]) : c ? fin.dim : fin.tile}"></div>`).join('')}</div>`).join('');
    const name = f.tr.strategy === 'classic' ? '' : ` · ${STRATEGY_NAMES[f.tr.strategy]}`;
    return `<div data-act="cmpFrame" data-v="${i}" style="display:grid; grid-template-columns:20px auto minmax(0,1fr) auto auto; gap:12px; align-items:center; padding:8px 12px; cursor:pointer; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}; background:${on ? 'oklch(0.58 0.17 255 / .09)' : 'transparent'}">
      <span style="color:#6b6d72; font-variant-numeric:tabular-nums">${i + 1}</span>
      <div style="background:${fin.frame}; padding:4px; border-radius:3px; display:flex; flex-direction:column; gap:1px">${thumb}</div>
      <div style="display:flex; flex-direction:column; gap:2px; min-width:0">
        <span class="flap-font" style="font-size:13.5px; letter-spacing:.04em; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(RB.decode(g) || '(blank)')}</span>
        <span style="font-size:11.5px; color:#6b6d72">${f.mode === 'grid' ? 'Grid' : 'Text'}${name}${short ? ` · held ${min} s on this board` : ''}</span>
      </div>
      <div style="display:flex; align-items:center; gap:6px"><span class="hint">Hold</span><input class="field" type="number" min="1" max="3600" style="width:64px" value="${f.hold}" data-hold="${i}"><span class="hint">s</span></div>
      <div style="display:flex; gap:3px">
        <button class="btn ico" data-act="cmpUp" data-v="${i}" title="Move up" ${i ? '' : 'disabled'}>↑</button>
        <button class="btn ico" data-act="cmpDown" data-v="${i}" title="Move down" ${i < C.frames.length - 1 ? '' : 'disabled'}>↓</button>
        <button class="btn ico" data-act="cmpDup" data-v="${i}" title="Duplicate">⧉</button>
        <button class="btn ico" data-act="cmpDel" data-v="${i}" title="Delete">✕</button>
      </div>
    </div>`;
  }).join('');
  return `<div style="display:flex; align-items:center; gap:10px; margin-top:6px">
      <span style="font-weight:600">Sequence</span>
      ${C.frames.length ? `<input class="field" style="width:180px" value="${esc(C.name)}" data-cmp="name" placeholder="Name, e.g. Morning">
        <div style="display:flex; align-items:center; gap:8px">${tog(C.loop, 'cmpLoop')}<span class="hint">Loop</span></div>` : '<span class="hint">Add frames to play several messages in turn, each held for a while after its flaps settle.</span>'}
      <div style="flex:1"></div>
      <button class="btn" data-act="cmpAdd">Add frame</button>
    </div>
    ${C.frames.length ? `<div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); overflow:hidden">${rows}</div>
      ${min > 1 ? `<div class="hint">${board() ? `${esc(board().name)} is a real Vestaboard, so this computer sends each frame` : 'This computer sends each frame to the real Vestaboards here'} and waits at least ${min} s between them.</div>` : ''}` : ''}`;
}

// Messages sent from Compose, newest first; each can be sent again or edited.
function recentSection() {
  if (!C.recent.length) return '';
  const rows = C.recent.map((r, i) => `<div style="display:grid; grid-template-columns:auto minmax(0,1fr) auto; gap:12px; align-items:center; padding:8px 12px; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}">
      ${thumb(r.layout, 4)}
      <div style="display:flex; flex-direction:column; gap:2px; min-width:0">
        <span class="flap-font" style="font-size:13.5px; letter-spacing:.04em; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(RB.decode(r.layout) || '(blank)')}</span>
        <span style="font-size:11.5px; color:#6b6d72; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${RB.ago(Date.now() - r.at)} · to ${esc(r.targetName)}${centredNote(r.layout)}</span>
      </div>
      <div style="display:flex; gap:4px; align-items:center">${sendActions('r' + i, r.layout)}</div>
    </div>`).join('');
  return `<div style="display:flex; align-items:center; gap:10px; margin-top:6px"><span style="font-weight:600">Recent</span><span class="hint">Sent from Compose</span></div>
    <div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); max-height:260px; overflow:auto">${rows}</div>`;
}
const loadRecent = () => RB.invoke('get_recent').then(r => { C.recent = r || []; if (L.view === 'compose') render(); });

// Message History → Edit: the exact grid, in Grid mode.
function composeEdit(grid) {
  if (C.preview) C.preview.stop = true;
  Object.assign(C, { msg: Object.assign(newItem('grid'), { cells: JSON.parse(JSON.stringify(grid)) }), frames: [], sel: -1, draftId: '', name: '', loop: false, status: null, deliveries: [], brush: 'type', cursor: { r: 0, c: 0 } });
}

// The preview's tile size, and its height (the same sums as split-flap.js), so the window is
// sized for the board before the board has drawn itself.
const previewTile = cols => Math.max(8, Math.min(40, Math.round(600 / (1.17 * cols))));
function previewHeight(rows, cols) {
  const t = previewTile(cols), h = Math.round(t * 1.42);
  return rows * h + (rows - 1) * Math.max(3, Math.round(t * 0.3)) + 2 * Math.max(22, Math.round(t * 1.15));
}

// After each render: put the persistent preview board back (it keeps flipping across renders).
function composeAfter() {
  const slot = document.getElementById('cmpPreview');
  if (!slot) return;
  const { rows, cols } = dims(), s = S.settings;
  const set = (k, v) => { if (preview.getAttribute(k) !== String(v)) preview.setAttribute(k, v); };
  set('rows', rows); set('cols', cols); set('tile', previewTile(cols)); set('finish', s.finish);
  set('speed', s.speed); set('sound', s.sound ? 'on' : 'off');
  slot.appendChild(preview);
  if (!C.preview) showCurrent();
}
function showCurrent() {
  const it = cur();
  preview.setAttribute('transition', JSON.stringify(it.tr));
  preview.setAttribute('message', JSON.stringify(layoutOf(it)));
}
function refreshStatus() { const el = document.getElementById('cmpStatus'); if (el) el.innerHTML = statusLine(); }

// ---------------------------------------------------------------- editing

const clone = it => JSON.parse(JSON.stringify(it));
function edited() { if (C.status && !C.busy) C.status = null; }
function setCell(r, c, code) {
  const it = cur(); it.cells = fitTo(it.cells);
  it.cells[r][c] = code; edited();
}
function moveCursor(dr, dc) {
  const { rows, cols } = dims();
  let { r, c } = C.cursor; c += dc; r += dr;
  if (c >= cols) { c = 0; r++; } if (c < 0) { c = cols - 1; r--; }
  C.cursor = { r: Math.min(rows - 1, Math.max(0, r)), c: Math.min(cols - 1, Math.max(0, c)) };
}
function paintEl(el) {
  const [r, c] = el.dataset.cell.split(',').map(Number);
  C.cursor = { r, c };
  if (C.brush === 'type') return;
  setCell(r, c, C.brush);
  const fin = RB.fin(S.settings.finish);
  el.style.background = C.brush >= 63 ? (C.brush === 71 ? fin.ink : RB.HEX[C.brush]) : fin.tile;
  el.textContent = C.brush >= 63 ? '' : RB.glyph(C.brush);
}

let painting = false;
root.addEventListener('pointerdown', e => {
  const el = L.view === 'compose' && e.button === 0 && e.target.closest('[data-cell]');
  if (!el) return;
  if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur();
  painting = true; paintEl(el); e.preventDefault();
});
root.addEventListener('pointermove', e => {
  if (!painting || C.brush === 'type') return;
  const el = document.elementFromPoint(e.clientX, e.clientY);
  const cell = el && el.closest('[data-cell]');
  if (cell) paintEl(cell);
});
document.addEventListener('pointerup', () => { painting = false; });

document.addEventListener('keydown', e => {
  if (L.view !== 'compose' || cur().mode !== 'grid' || C.preview || e.metaKey || e.ctrlKey || e.altKey) return;
  const a = document.activeElement;
  if (a && /^(INPUT|TEXTAREA|SELECT|BUTTON)$/.test(a.tagName)) return;
  const k = e.key, { r, c } = C.cursor;
  if (k === 'ArrowLeft') moveCursor(0, -1);
  else if (k === 'ArrowRight') moveCursor(0, 1);
  else if (k === 'ArrowUp') moveCursor(-1, 0);
  else if (k === 'ArrowDown') moveCursor(1, 0);
  else if (k === 'Enter') C.cursor = { r: Math.min(dims().rows - 1, r + 1), c: 0 };
  else if (k === 'Backspace') { moveCursor(0, -1); setCell(C.cursor.r, C.cursor.c, 0); }
  else if (k === 'Delete') setCell(r, c, 0);
  else if (k.length === 1 && CODE[k.toUpperCase()] != null) { setCell(r, c, CODE[k.toUpperCase()]); moveCursor(0, 1); }
  else return;
  e.preventDefault(); render(true);
});

let textTimer = 0;
root.addEventListener('input', e => {
  const k = e.target.dataset.cmp;
  if (k !== 'text') return;
  const it = cur(); it.text = e.target.value; edited();
  clearTimeout(textTimer);
  textTimer = setTimeout(() => relayout(it).then(() => { if (!C.preview) showCurrent(); }), 120);
});
root.addEventListener('change', e => {
  const t = e.target, k = t.dataset.cmp, it = cur();
  if (t.dataset.hold != null) {
    const f = C.frames[+t.dataset.hold], v = Math.round(+t.value);
    if (f) f.hold = Math.min(3600, Math.max(1, v || 1));
    return render(true);
  }
  if (!k) return;
  if (k === 'target') return setTarget(t.value);
  if (t.dataset.field === 'cmpRows' || t.dataset.field === 'cmpCols') {
    const [lo, hi] = t.dataset.field === 'cmpRows' ? [2, 12] : [8, 40], n = Math.round(+t.value);
    if (n >= lo && n <= hi) setSize({ ...dims(), [t.dataset.field === 'cmpRows' ? 'rows' : 'cols']: n }); else render(true);
    return;
  }
  if (k === 'draft') return t.value ? loadDraft(t.value) : newDraft();
  if (k === 'strategy') it.tr.strategy = t.value;
  else if (k === 'interval') it.tr.stepIntervalMs = Math.min(5000, Math.max(1, Math.round(+t.value) || 120));
  else if (k === 'size') it.tr.stepSize = Math.min(22, Math.max(1, Math.round(+t.value) || 1));
  else if (k === 'name') C.name = t.value.trim();
  else return;
  edited(); render(true);
  if (k !== 'name' && !C.preview && preview.isConnected) { preview.setAttribute('seq', ++C.pseq); }
});

function setTarget(id) {
  Object.assign(C, { target: id, remote: null, status: null, deliveries: [], startedRemote: 0 });
  relayoutAll().then(() => render(true));
  // An older Rattleboard may have been updated since it was saved, so look again.
  dests().filter(d => d.id !== 'this').forEach(d => RB.invoke('check_board', { id: d.id }).then(x => { C.kinds[x.id] = x.kind; render(true); }).catch(() => {}));
}
// The message size for adaptive boards; remembered for next time.
function setSize(size) {
  C.size = { rows: size.rows, cols: size.cols };
  RB.invoke('update_settings', { patch: { sendRows: size.rows, sendCols: size.cols } });
  relayoutAll().then(() => render(true));
}

// ---------------------------------------------------------------- sending

// A message as boards take it. Text goes as text, so each fixed-size board lays it out for
// itself; adaptive boards get the picked size as a hint. A grid goes exactly as drawn.
const body = it => (it.mode === 'text'
  ? { text: it.text, ...(adaptiveAny() ? dims() : {}), ...trBody(it.tr) }
  : { characters: layoutOf(it), ...trBody(it.tr) });
const sequenceBody = () => ({ name: C.name, loop: C.loop, frames: C.frames.map(f => ({ ...body(f), hold: f.hold })) });
function done(p, ok) {
  C.busy = true; C.status = { text: 'Sending…' }; C.waiting = {}; render(true);
  return p.then(r => { C.status = { text: ok(r) }; })
    .catch(e => { C.status = { text: String(e), error: true }; })
    .finally(() => { C.busy = false; C.waiting = {}; render(true); });
}
const targetName = () => RB.targetName(S, C.target);
// "Sent to TV", or "Sent to 2 of 3 boards" with the details in the list above.
function summary(res, verb) {
  C.deliveries = res;
  const ok = res.filter(d => d.status === 'sent').length;
  if (res.length === 1) {
    if (ok) return `${verb} ${res[0].name}`;
    throw new Error(`${res[0].name}: ${res[0].error}`);
  }
  if (!ok) throw new Error(`Nothing was sent; see the list above`);
  return `${verb} ${ok} of ${res.length} boards`;
}
RB.listen('rb://delivery', e => {
  if (!C.busy) return;
  C.waiting[e.payload.target] = e.payload.name;
  refreshStatus();
});

// Asks another Rattleboard what it's playing, until it stops.
let remoteTimer = 0;
function watchRemote() {
  clearTimeout(remoteTimer);
  const b = board();
  if (!b || kindOf(b) !== 'rattleboard' || L.view !== 'compose') return;
  const id = b.id;
  RB.invoke('remote_sequence', { target: id }).then(r => {
    if (C.target !== id) return;
    C.remote = r; refreshStatus();
    if (r.running) remoteTimer = setTimeout(watchRemote, 2000); else render(true);
  }).catch(() => {});
}

// ---------------------------------------------------------------- local preview

const sleep = (ms, run) => new Promise(res => {
  const end = Date.now() + ms;
  const t = setInterval(() => { if (run.stop || Date.now() >= end) { clearInterval(t); res(); } }, 100);
});
function flip(grid, tr) {
  return new Promise(res => {
    const fin = () => { preview.removeEventListener('flapdone', fin); res(); };
    preview.addEventListener('flapdone', fin);
    preview.setAttribute('transition', JSON.stringify(tr));
    preview.setAttribute('seq', ++C.pseq);
    preview.setAttribute('message', JSON.stringify(grid));
  });
}
async function previewPlay() {
  const frames = C.frames.length ? C.frames : [C.msg];
  const run = { n: 0, of: frames.length, stop: false };
  C.preview = run; render(true);
  do {
    for (let i = 0; i < frames.length && !run.stop; i++) {
      run.n = i + 1; refreshStatus();
      await flip(layoutOf(frames[i]), frames[i].tr);
      if (frames.length > 1 || C.loop) await sleep(frames[i].hold * 1000, run);
    }
  } while (C.loop && C.frames.length && !run.stop);
  if (C.preview === run) { C.preview = null; render(true); }
}

// ---------------------------------------------------------------- drafts

const strip = it => ({ mode: it.mode, text: it.text, cells: it.cells, hold: it.hold, tr: it.tr });
function draftName() {
  const first = [C.msg, ...C.frames].map(it => (it.mode === 'text' ? it.text : RB.decode(fitTo(it.cells)))).find(t => t && t.trim());
  return C.name || (first ? first.trim().split('\n')[0].slice(0, 32) : 'Untitled');
}
function saveDraft() {
  const d = { id: C.draftId || 'd-' + Date.now().toString(36), name: draftName(), target: C.target, msg: strip(C.msg), frames: C.frames.map(strip), seqName: C.name, loop: C.loop, savedAt: Date.now() };
  C.draftId = d.id;
  C.drafts = [d, ...C.drafts.filter(x => x.id !== d.id)];
  RB.invoke('save_drafts', { drafts: C.drafts })
    .then(() => { C.status = { text: `Saved draft “${d.name}”`, saved: true }; })
    .catch(e => { C.status = { text: String(e), error: true }; })
    .finally(() => render(true));
}
function loadDraft(id) {
  const d = C.drafts.find(x => x.id === id); if (!d) return;
  const item = x => Object.assign(newItem(), clone(x));
  Object.assign(C, { draftId: d.id, msg: item(d.msg || {}), frames: (d.frames || []).map(item), name: d.seqName || '', loop: !!d.loop, status: null });
  C.sel = C.frames.length ? 0 : -1;
  if (d.target && (d.target === 'this' || S.boards.some(b => b.id === d.target))) C.target = d.target;
  relayoutAll().then(() => render(true));
}
function newDraft() {
  Object.assign(C, { draftId: '', msg: newItem(), frames: [], sel: -1, name: '', loop: false, status: null });
  relayoutAll().then(() => render(true));
}

// ---------------------------------------------------------------- actions

RB.delegate(root, 'click', {
  cmpMode: el => {
    const it = cur(); if (it.mode === el.dataset.v) return;
    // Switching to Grid starts from what the text lays out to, so it can be touched up.
    if (el.dataset.v === 'grid' && (!it.cells || !it.cells.flat().some(Boolean))) it.cells = fitTo(it.grid);
    it.mode = el.dataset.v; edited();
    (it.mode === 'text' ? relayout(it) : Promise.resolve()).then(() => render(true));
  },
  cmpBrush: el => { C.brush = el.dataset.v === 'type' ? 'type' : +el.dataset.v; render(true); },
  cmpClear: () => { cur().cells = blankGrid(); C.cursor = { r: 0, c: 0 }; edited(); render(true); },
  cmpReplay: () => { preview.setAttribute('seq', ++C.pseq); },
  cmpLoop: () => { C.loop = !C.loop; render(true); },
  cmpFrame: (el, e) => { if (e.target.closest('input')) return; C.sel = +el.dataset.v; render(true); },
  cmpAdd: () => {
    const base = cur(), f = Object.assign(newItem(base.mode), { hold: Math.max(base.hold, minHold()), tr: clone(base.tr) });
    if (base.mode === 'grid') f.cells = blankGrid();
    if (!C.frames.length) C.frames.push(C.msg);
    C.frames.push(f); C.sel = C.frames.length - 1; C.msg = newItem();
    relayout(f).then(() => render(true));
  },
  cmpUp: el => { const i = +el.dataset.v; [C.frames[i - 1], C.frames[i]] = [C.frames[i], C.frames[i - 1]]; C.sel = i - 1; render(true); },
  cmpDown: el => { const i = +el.dataset.v; [C.frames[i + 1], C.frames[i]] = [C.frames[i], C.frames[i + 1]]; C.sel = i + 1; render(true); },
  cmpDup: el => { const i = +el.dataset.v; C.frames.splice(i + 1, 0, clone(C.frames[i])); C.sel = i + 1; render(true); },
  cmpDel: el => {
    const i = +el.dataset.v, [gone] = C.frames.splice(i, 1);
    if (!C.frames.length) { C.msg = gone; C.sel = -1; } else C.sel = Math.min(C.sel, C.frames.length - 1);
    render(true);
  },
  cmpPreviewPlay: () => { if (C.preview) C.preview.stop = true; else previewPlay(); },
  cmpSend: () => {
    C.lastBody = body(cur());
    done(RB.invoke('send_message', { target: C.target, message: C.lastBody, recent: true }), res => { loadRecent(); return summary(res, 'Sent to'); });
  },
  cmpRetry: el => {
    const target = el.dataset.v, row = C.deliveries.find(d => d.target === target);
    if (!row || !C.lastBody) return;
    row.status = 'sending'; row.error = null; render(true);
    const send = C.lastBody.frames ? RB.invoke('play_sequence', { target, sequence: C.lastBody }) : RB.invoke('send_message', { target, message: C.lastBody });
    send.then(([d]) => Object.assign(row, { status: d.status, error: d.error }))
      .catch(e => Object.assign(row, { status: 'failed', error: String(e) }))
      .finally(() => render(true));
  },
  cmpPlay: () => {
    const b = board();
    C.lastBody = sequenceBody();
    done(RB.invoke('play_sequence', { target: C.target, sequence: C.lastBody }), res => {
      // Rattleboards play it by themselves, so they don't show up in this computer's players.
      C.startedRemote = res.filter(d => d.status === 'sent' && (S.boards.find(x => x.id === d.target) || {}).kind === 'rattleboard').length;
      if (b && kindOf(b) === 'rattleboard') setTimeout(watchRemote, 300);
      return summary(res, 'Started on');
    });
  },
  cmpStop: () => done(RB.invoke('stop_sequence', { target: C.target }), () => { C.remote = null; C.startedRemote = 0; return 'Stopped'; }),
  cmpSize: el => {
    const p = RB.PRESETS.find(x => x[0] === el.dataset.v);
    C.customSize = !p;
    setSize(p ? { rows: p[2], cols: p[3] } : { ...dims() });
  },
  cmpSaveDraft: saveDraft,
  cmpDeleteDraft: () => {
    C.drafts = C.drafts.filter(x => x.id !== C.draftId);
    RB.invoke('save_drafts', { drafts: C.drafts }).finally(newDraft);
  }
});

RB.invoke('get_drafts').then(d => { C.drafts = Array.isArray(d) ? d : []; if (L.view === 'compose') render(true); });
loadRecent();
// The board's size or the saved boards can change while Compose is open.
let lastDims = '';
RB.listen('rb://state', () => {
  if (!S) return;
  // A removed board or group falls back to this board.
  if (!RB.targets(S).some(t => t[0] === C.target)) C.target = 'this';
  const d = JSON.stringify(dims());
  if (d !== lastDims) { lastDims = d; relayoutAll().then(() => { if (L.view === 'compose') render(); }); }
});
