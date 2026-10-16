// The floating split-flap replica. Visibility, the hide countdown and dragging live here;
// the backend decides *when* to present it and owns window placement.
const stage = document.getElementById('stage');
const sfb = document.getElementById('sfb');
const $ = id => document.getElementById(id);

let S = null;
const ui = { visible: false, flipping: false, remaining: 0, total: 0, hover: false, dragging: false };
let lastSize = '';

function tileSize(s) {
  const flag = s.model !== 'note';
  return (flag ? { s: 18, m: 24, l: 30 } : { s: 26, m: 34, l: 42 })[s.size] || 24;
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

  const topish = s.position === 'custom' ? s.custom.y < screen.availHeight * 0.375 * devicePixelRatio : s.position[0] === 't';
  stage.classList.toggle('top', topish);
  stage.classList.toggle('on', ui.visible);
  stage.classList.toggle('dragging', ui.dragging);

  const counting = ui.visible && !s.pinned && !ui.flipping && ui.remaining > 0;
  const paused = counting && s.pauseOnHover && ui.hover;
  const name = s.boardName || 'Vestaboard';
  $('statusText').textContent = ui.dragging ? 'Release near a corner to snap'
    : ui.flipping ? `${name} · updating`
    : s.pinned ? `${name} · pinned`
    : counting ? `${name} · hides in ${Math.ceil(ui.remaining / 1000)}s${paused ? ' · paused' : ''}`
    : S.conn === 'error' ? `${name} · offline` : name;
  $('dot').style.background = ui.flipping ? '#f2c230' : S.conn === 'error' ? '#e0553f' : '#3fbf6a';
  $('chrome').style.opacity = ui.hover || ui.dragging ? 1 : s.pinned ? 0.8 : 0;
  $('pinBtn').textContent = s.pinned ? 'Unpin' : 'Pin';
  $('pinBtn').classList.toggle('on', s.pinned);
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

function startCountdown(pinned = S && S.settings.pinned) {
  if (!ui.visible) return;
  if (pinned) { ui.remaining = 0; ui.total = 0; }
  else { const t = (S ? S.settings.hideSec : 10) * 1000; ui.remaining = t; ui.total = t; }
  render();
}

function hide() {
  ui.visible = false; ui.remaining = 0; render();
  setTimeout(() => { if (!ui.visible) RB.invoke('board_hidden'); }, 420);
}

RB.onState(snap => { S = snap; render(); });
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
  if (ui.visible && !s.pinned && !ui.flipping && ui.remaining > 0 && !(s.pauseOnHover && ui.hover) && !ui.dragging) {
    ui.remaining -= 100;
    if (ui.remaining <= 0) return hide();
    render();
  }
}, 100);

RB.delegate(document.body, 'click', {
  pin: () => {
    const pinned = !S.settings.pinned;
    RB.invoke('update_settings', { patch: { pinned } });
    ui.visible = true; startCountdown(pinned);
  },
  replay: () => RB.invoke('replay'),
  hide
});

// Dragging: move the native window with the pointer; the backend snaps on release.
let drag = null, pending = null;
stage.addEventListener('pointerdown', e => {
  if (e.button !== 0 || e.target.closest('[data-nodrag]')) return;
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
