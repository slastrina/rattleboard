// The floating split-flap replica. Visibility, the hide countdown and dragging live here;
// the backend decides *when* to present it and owns window placement.
const stage = document.getElementById('stage');
const sfb = document.getElementById('sfb');
const $ = id => document.getElementById(id);

let S = null;
const ui = { visible: false, flipping: false, remaining: 0, total: 0, hover: false, dragging: false, resizing: null };
let lastSize = '';

const floating = () => !!S && S.settings.mode === 'float';
const MIN_SCALE = 0.4, MAX_SCALE = 3;

function tileSize(s) {
  const scale = ui.resizing ? ui.resizing.scale : s.scale || 1;
  return Math.max(7, Math.round((s.model === 'note' ? 34 : 24) * scale));
}

// The resize grip sits on the corner facing away from the screen edge the board is parked against.
function gripCorner(s) {
  if (s.mode === 'float') return 'br';
  return { tl: 'br', tr: 'bl', bl: 'tr', br: 'tl' }[s.position] || 'tl';
}

function render() {
  if (!S) return;
  const s = S.settings, { rows, cols } = RB.dims(s.model), tile = tileSize(s), blk = s.finish !== 'white';
  const pad = Math.max(22, Math.round(tile * 1.15));
  const set = (k, v) => { if (sfb.getAttribute(k) !== String(v)) sfb.setAttribute(k, v); };
  set('rows', rows); set('cols', cols); set('tile', tile); set('speed', s.speed); set('finish', s.finish);
  set('sound', s.sound ? 'on' : 'off'); set('seq', S.seq); set('message', JSON.stringify(RB.fit(S.display, s.model)));

  const st = stage.style;
  st.setProperty('--pad', pad + 'px');
  st.setProperty('--bar-bottom', Math.round(pad / 2 - 1) + 'px');
  st.setProperty('--band', blk ? 'rgba(240,238,232,.75)' : 'rgba(28,28,28,.7)');
  st.setProperty('--track', blk ? 'rgba(255,255,255,.12)' : 'rgba(0,0,0,.1)');
  st.setProperty('--bb', blk ? 'rgba(255,255,255,.1)' : 'rgba(0,0,0,.07)');
  st.setProperty('--bb-on', blk ? 'rgba(255,255,255,.24)' : 'rgba(0,0,0,.16)');

  const float = s.mode === 'float';
  const topish = float ? s.custom.y < screen.availHeight * 0.375 * devicePixelRatio : s.position[0] === 't';
  const g = gripCorner(s), grip = $('grip');
  grip.className = g;
  grip.style.opacity = ui.hover || ui.resizing ? 1 : 0;
  stage.classList.toggle('top', topish);
  stage.classList.toggle('on', ui.visible);
  stage.classList.toggle('dragging', ui.dragging);

  const counting = ui.visible && !float && !ui.flipping && ui.remaining > 0;
  const paused = counting && s.pauseOnHover && ui.hover;
  const name = s.boardName || 'Vestaboard';
  $('statusText').textContent = ui.resizing ? `${Math.round(ui.resizing.scale * 100)}%`
    : ui.dragging ? (float ? 'Release to place it here' : 'Release to snap to the nearest corner')
    : ui.flipping ? `${name} · updating`
    : float ? `${name} · pinned`
    : counting ? `${name} · hides in ${Math.ceil(ui.remaining / 1000)}s${paused ? ' · paused' : ''}`
    : S.conn === 'error' ? `${name} · offline` : name;
  $('dot').style.background = ui.flipping ? '#f2c230' : S.conn === 'error' ? '#e0553f' : '#3fbf6a';
  $('chrome').style.opacity = ui.hover || ui.dragging || ui.resizing ? 1 : 0;
  $('pinBtn').textContent = float ? 'Unpin' : 'Pin';
  $('pinBtn').title = float ? 'Go back to popping up in a corner' : 'Keep it on screen, wherever you put it';
  $('pinBtn').classList.toggle('on', float);
  $('bar').style.opacity = counting ? 1 : 0;
  $('fill').style.width = (ui.total ? (ui.remaining / ui.total) * 100 : 0) + '%';
}

// Keep the native window exactly as large as the board (plus shadow room).
function measure() {
  document.fonts.ready.then(() => {
    const w = sfb.offsetWidth, h = sfb.offsetHeight;
    const key = w + 'x' + h;
    if (w && h && key !== lastSize) { lastSize = key; RB.invoke('board_resize', { width: w, height: h }); }
  });
}

function startCountdown(float = floating()) {
  if (!ui.visible) return;
  if (float) { ui.remaining = 0; ui.total = 0; }
  else { const t = (S ? S.settings.hideSec : 10) * 1000; ui.remaining = t; ui.total = t; }
  render();
}

function hide() {
  ui.visible = false; ui.remaining = 0; render();
  setTimeout(() => { if (!ui.visible) RB.invoke('board_hidden'); }, 420);
}

RB.onState(snap => {
  const wasFloat = floating();
  S = snap;
  // Unpinned: the toast starts counting down from where it is now.
  if (wasFloat && !floating() && ui.visible && !ui.flipping) startCountdown(false);
  render();
});
RB.listen('rb://present', e => {
  ui.visible = true; ui.remaining = 0; render();
  if (e.payload) startCountdown();
  else setTimeout(() => { if (ui.visible && !ui.flipping && ui.remaining === 0) startCountdown(); }, 500);
});

sfb.addEventListener('flaplayout', measure);
sfb.addEventListener('flapstart', () => { ui.flipping = true; render(); });
sfb.addEventListener('flapdone', () => { ui.flipping = false; startCountdown(); });
stage.addEventListener('mouseenter', () => { ui.hover = true; render(); });
stage.addEventListener('mouseleave', () => { ui.hover = false; render(); });

setInterval(() => {
  if (!S) return;
  const s = S.settings;
  if (ui.visible && s.mode !== 'float' && !ui.flipping && ui.remaining > 0 && !(s.pauseOnHover && ui.hover) && !ui.dragging && !ui.resizing) {
    ui.remaining -= 100;
    if (ui.remaining <= 0) return hide();
    render();
  }
}, 100);

RB.delegate(document.body, 'click', {
  pin: () => {
    const float = !floating();
    RB.invoke('update_settings', { patch: { mode: float ? 'float' : 'toast' } });
    ui.visible = true; startCountdown(float);
  },
  replay: () => RB.invoke('replay'),
  hide
});

// Dragging: move the native window with the pointer; the backend snaps on release.
let drag = null, pending = null;
stage.addEventListener('pointerdown', e => {
  if (e.button !== 0 || e.target.closest('[data-nodrag]') || ui.resizing) return;
  drag = { sx: e.screenX, sy: e.screenY, moved: false };
  stage.setPointerCapture(e.pointerId);
});
stage.addEventListener('pointermove', e => {
  if (!drag) return;
  const dx = e.screenX - drag.sx, dy = e.screenY - drag.sy;
  if (!drag.moved) {
    if (Math.hypot(dx, dy) < 4) return;
    drag.moved = true; ui.dragging = true; render();
    RB.invoke('board_drag_start');
  }
  if (!pending) requestAnimationFrame(() => { RB.invoke('board_drag_move', pending); pending = null; });
  pending = { dx, dy };
});
const endDrag = () => {
  if (!drag) return;
  const moved = drag.moved; drag = null;
  if (!moved) return;
  setTimeout(() => RB.invoke('board_drag_end').then(() => { ui.dragging = false; render(); }), 30);
};
stage.addEventListener('pointerup', endDrag);
stage.addEventListener('pointercancel', endDrag);

// Resizing: drag the grip to scale the whole board; the window follows its size.
const grip = $('grip');
grip.addEventListener('pointerdown', e => {
  if (e.button !== 0 || !S) return;
  e.stopPropagation(); e.preventDefault();
  const g = gripCorner(S.settings), w = sfb.offsetWidth, h = sfb.offsetHeight, start = S.settings.scale || 1;
  // Never let the board outgrow the screen it's on.
  const fit = Math.min((screen.availWidth - 40) / w, (screen.availHeight - 40) / h) * start;
  ui.resizing = { sx: e.screenX, sy: e.screenY, w, h, start, scale: start, max: Math.min(MAX_SCALE, fit),
    dirX: g[1] === 'r' ? 1 : -1, dirY: g[0] === 'b' ? 1 : -1 };
  grip.setPointerCapture(e.pointerId);
  render();
});
grip.addEventListener('pointermove', e => {
  const r = ui.resizing; if (!r) return;
  const dx = (e.screenX - r.sx) * r.dirX, dy = (e.screenY - r.sy) * r.dirY;
  // Follow whichever direction the pointer moved further, keeping the board's proportions.
  const f = Math.abs(dx) / r.w >= Math.abs(dy) / r.h ? (r.w + dx) / r.w : (r.h + dy) / r.h;
  const scale = Math.round(Math.min(r.max, Math.max(MIN_SCALE, r.start * f)) * 100) / 100;
  if (scale !== r.scale) { r.scale = scale; render(); }
});
const endResize = () => {
  const r = ui.resizing; if (!r) return;
  RB.invoke('update_settings', { patch: { scale: r.scale } }).finally(() => { ui.resizing = null; render(); });
};
grip.addEventListener('pointerup', endResize);
grip.addEventListener('pointercancel', endResize);
