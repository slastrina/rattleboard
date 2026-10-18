// The floating split-flap replica. Visibility, the hide countdown and dragging live here;
// the backend decides *when* to present it and owns window placement. In full screen the
// window covers a display, and the board is fitted into it here.
const stage = document.getElementById('stage');
const sfb = document.getElementById('sfb');
const $ = id => document.getElementById(id);

let S = null;
const ui = { visible: false, flipping: false, remaining: 0, total: 0, hover: false, dragging: false, resizing: null };
let lastSize = '';
// The rows × cols on screen; a message of another size waits for the window to fit it first.
let shownGeom = '', fitting = false;

const floating = () => !!S && S.settings.mode === 'float';
const full = () => !!S && S.settings.mode === 'fullscreen';
// Pinned and full screen stay up; only the pop-up hides again.
const stays = () => floating() || full();
const MIN_SCALE = 0.4, MAX_SCALE = 3;

// Full screen: the largest tile at which the whole board, frame included, fits the display
// with a comfortable margin. The sizes follow split-flap.js's _style.
function fitTile(rows, cols) {
  const W = innerWidth, H = innerHeight, m = Math.max(24, Math.round(Math.min(W, H) * 0.06));
  const fits = w => {
    const h = Math.round(w * 1.42), pad = Math.max(22, Math.round(w * 1.15));
    return cols * w + (cols - 1) * Math.max(2, Math.round(w * 0.17)) + 2 * pad <= W - 2 * m
      && rows * h + (rows - 1) * Math.max(3, Math.round(w * 0.3)) + 2 * pad <= H - 2 * m;
  };
  let lo = 5, hi = 1000;
  while (lo < hi) { const mid = Math.ceil((lo + hi) / 2); if (fits(mid)) lo = mid; else hi = mid - 1; }
  return lo;
}

function tileSize(s, rows, cols) {
  if (full()) return fitTile(rows, cols);
  const scale = ui.resizing ? ui.resizing.scale : s.scale || 1;
  // Scale is the tile size, so a smaller message makes a smaller board, not bigger tiles.
  return Math.max(5, Math.round(24 * scale));
}

// The resize grip sits on the corner facing away from the screen edge the board is parked against.
function gripCorner(s) {
  if (s.mode === 'float') return 'br';
  return { tl: 'br', tr: 'bl', bl: 'tr', br: 'tl' }[s.position] || 'tl';
}

function render() {
  if (!S) return;
  const s = S.settings, { rows, cols } = RB.dims(S), tile = tileSize(s, rows, cols), blk = s.finish !== 'white';
  const pad = Math.max(22, Math.round(tile * 1.15));
  const set = (k, v) => { if (sfb.getAttribute(k) !== String(v)) sfb.setAttribute(k, v); };
  set('tile', tile); set('speed', s.speed); set('finish', s.finish);
  set('sound', s.sound ? 'on' : 'off'); set('transition', JSON.stringify(S.transition || null));
  const geom = rows + 'x' + cols;
  if (geom !== shownGeom && shownGeom) {
    // Resize the (blank) board and its window, then flip the message in, so the flaps never
    // turn inside a window of the wrong size.
    shownGeom = geom; fitting = true;
    set('rows', rows); set('cols', cols);
    requestAnimationFrame(() => measure().then(() => { fitting = false; render(); }));
  } else if (!fitting) {
    shownGeom = geom;
    set('rows', rows); set('cols', cols);
    set('seq', S.seq); set('message', JSON.stringify(RB.fit(S.display, RB.dims(S))));
  }

  const st = stage.style;
  st.setProperty('--pad', pad + 'px');
  st.setProperty('--bar-bottom', Math.round(pad / 2 - 1) + 'px');
  st.setProperty('--band', blk ? 'rgba(240,238,232,.75)' : 'rgba(28,28,28,.7)');
  st.setProperty('--track', blk ? 'rgba(255,255,255,.12)' : 'rgba(0,0,0,.1)');
  st.setProperty('--bb', blk ? 'rgba(255,255,255,.1)' : 'rgba(0,0,0,.07)');
  st.setProperty('--bb-on', blk ? 'rgba(255,255,255,.24)' : 'rgba(0,0,0,.16)');
  // Full screen's backdrop (as backdrop() in lib.rs) and its controls.
  const rs = document.documentElement.style;
  document.documentElement.classList.toggle('full', full());
  rs.setProperty('--backdrop', blk ? '#050506' : '#e4e2dc');
  rs.setProperty('--ov', blk ? 'rgba(24,24,26,.86)' : 'rgba(250,249,246,.9)');
  rs.setProperty('--ov-ink', blk ? 'rgba(240,238,232,.9)' : 'rgba(28,28,28,.85)');
  rs.setProperty('--bb', blk ? 'rgba(255,255,255,.1)' : 'rgba(0,0,0,.07)');
  rs.setProperty('--bb-on', blk ? 'rgba(255,255,255,.2)' : 'rgba(0,0,0,.13)');
  // The credit sits in the margin fitTile leaves below the board, so it never meets the frame.
  const m = Math.max(24, Math.round(Math.min(innerWidth, innerHeight) * 0.06)), fs = Math.min(22, Math.max(9, Math.round(innerHeight / 1080 * 11.5)));
  rs.setProperty('--credit', blk ? 'rgba(240,238,232,.38)' : 'rgba(28,28,28,.4)');
  rs.setProperty('--credit-fs', fs + 'px');
  rs.setProperty('--credit-gap', Math.max(6, Math.round((m - fs) / 2)) + 'px');

  const float = s.mode === 'float';
  const topish = float ? s.custom.y < screen.availHeight * 0.375 * devicePixelRatio : s.position[0] === 't';
  const g = gripCorner(s), grip = $('grip');
  grip.className = g;
  grip.style.opacity = ui.hover || ui.resizing ? 1 : 0;
  stage.classList.toggle('top', topish);
  stage.classList.toggle('on', ui.visible);
  stage.classList.toggle('dragging', ui.dragging);

  const counting = ui.visible && !stays() && !ui.flipping && ui.remaining > 0;
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
  // Next shows while sources are running.
  const next = !!(S.sources && S.sources.active);
  $('nextBtn').style.display = next ? '' : 'none';
  $('ovNext').style.display = next ? '' : 'none';
  $('bar').style.opacity = counting ? 1 : 0;
  $('fill').style.width = (ui.total ? (ui.remaining / ui.total) * 100 : 0) + '%';
  renderOverlay();
}

// Keep the native window exactly as large as the board (plus shadow room).
function measure() {
  // Full screen: the window is the display's size already. Measure afresh when it ends.
  if (full()) { lastSize = ''; return Promise.resolve(); }
  return document.fonts.ready.then(() => {
    const w = sfb.offsetWidth, h = sfb.offsetHeight;
    const key = w + 'x' + h;
    if (w && h && key !== lastSize) { lastSize = key; return RB.invoke('board_resize', { width: w, height: h }); }
  });
}

function startCountdown(float = stays()) {
  if (!ui.visible) return;
  if (float) { ui.remaining = 0; ui.total = 0; }
  else { const t = (S ? S.settings.hideSec : 10) * 1000; ui.remaining = t; ui.total = t; }
  render();
}

function hide() {
  if (full()) return;
  ui.visible = false; ui.remaining = 0; render();
  setTimeout(() => { if (!ui.visible) RB.invoke('board_hidden'); }, 420);
}

RB.onState(snap => {
  const first = !S, wasStaying = stays(), wasFull = full();
  S = snap;
  // Pinned or full screen, the board is always up, even if it was presented before this
  // page was listening.
  if (full() || (first && floating())) ui.visible = true;
  // Unpinned, or out of full screen: the toast starts counting down from where it is now.
  if (wasStaying && !stays() && ui.visible && !ui.flipping) startCountdown(false);
  if (full() !== wasFull) { pair = null; renderPair(); if (full()) wake(); }
  if (pair && pair.waiting && S.server.running && S.settings.serverLan && S.server.urls.length > 1) showPair();
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
  if (full()) return idle();
  if (ui.visible && !stays() && !ui.flipping && ui.remaining > 0 && !(s.pauseOnHover && ui.hover) && !ui.dragging && !ui.resizing) {
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
  next: () => RB.invoke('sources_next'),
  hide,
  exitFull: () => RB.invoke('toggle_fullscreen'),
  site: () => { if (ov.shown) RB.invoke('open_site'); },
  settings: () => RB.invoke('open_window', { view: 'settings' }),
  pair: showPair,
  pairHide: () => { pair = null; renderPair(); },
  pairOn: () => { pair = { waiting: true }; renderPair(); RB.invoke('update_settings', { patch: { serverEnabled: true, serverLan: true } }); }
});

// ---------------------------------------------------------------- full screen

// Moving the pointer or tapping shows a small bar of controls; it and the pointer hide again
// after a few seconds of stillness (never while the pointer rests on the bar).
const IDLE_MS = 3000;
const ov = { shown: false, at: 0, x: null, y: null, over: false, inside: true };

function wake(e) {
  if (!full()) return;
  // WebKit sends a move without movement after layout changes; that's not the user.
  if (e && e.type === 'pointermove') {
    if (e.screenX === ov.x && e.screenY === ov.y) return;
    ov.x = e.screenX; ov.y = e.screenY;
  }
  ov.at = Date.now();
  if (!ov.shown) { ov.shown = true; renderOverlay(); }
}

function idle() {
  if (!ov.shown || ov.over || Date.now() - ov.at < IDLE_MS) return;
  ov.shown = false; renderOverlay();
  // Only when it's over the board: on another display, the pointer is someone's.
  if (ov.inside) RB.invoke('hide_pointer');
}

function renderOverlay() {
  const on = full();
  document.documentElement.classList.toggle('nopointer', on && !ov.shown && ov.inside);
  $('overlay').classList.toggle('on', on && ov.shown);
  // Not a link on a screen nobody touches; with the controls up, a click opens the page.
  $('credit').classList.toggle('live', on && ov.shown);
  if (!on) return;
  const s = S.settings, p = (S.playing || []).find(p => p.target === 'this');
  const status = !s.polling ? 'Checking paused'
    : S.conn === 'error' ? (S.error || "Can't reach the board")
    : p ? `Playing ${p.name ? `‘${p.name}’` : 'sequence'} ${p.frame}/${p.frames}`
    : S.sources && S.sources.active && S.sources.showing ? `Showing ${S.sources.showing}`
    : S.lastChangeAt ? `Changed ${RB.ago(Date.now() - S.lastChangeAt)}` : 'Waiting for a message';
  $('ovText').textContent = `${s.boardName || 'Vestaboard'} · ${RB.sourceLabel(S)} · ${status}`;
  $('ovDot').style.background = !s.polling ? '#9a9ca1' : S.conn === 'error' ? '#e0553f' : ui.flipping ? '#f2c230' : '#3fbf6a';
}

// Pair a phone: the QR code from Settings → API server, on the board's own screen.
const PAIR_MS = 2 * 60 * 1000;
let pair = null;
const pairLeft = () => { const left = Math.max(0, Math.ceil((pair.until - Date.now()) / 1000)); return `Hides in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`; };

function showPair() {
  RB.invoke('pair_qr')
    .then(q => { pair = { ...q, until: Date.now() + PAIR_MS }; })
    .catch(e => { pair = { error: String(e) }; })
    .finally(renderPair);
}

function renderPair() {
  const el = $('pair'), open = full() && !!pair;
  el.classList.toggle('open', open);
  if (!open) { el.classList.remove('on'); return; }
  const s = S.settings, off = !s.serverEnabled || !s.serverLan;
  el.innerHTML = pair.svg
    ? `<div class="qr">${pair.svg}</div>
      <div style="display:flex; flex-direction:column; gap:8px; min-width:0">
        <div style="font-weight:600">Scan with the Rattleboard app</div>
        <div class="hint">Or with the phone's camera. The phone adds this computer’s board with its address and key. The code includes the key, so only show it to devices you trust.</div>
        <div style="display:flex; align-items:center; gap:8px"><button data-act="pairHide">Hide</button><span class="hint" id="pairLeft" style="font-variant-numeric:tabular-nums">${pairLeft()}</span></div>
      </div>`
    : `<div style="display:flex; flex-direction:column; gap:8px">
        <div style="font-weight:600">Pair a phone</div>
        <div class="hint">${pair.waiting ? 'Starting the API server for other devices…'
          : off ? 'Pairing needs the API server on, with Other devices allowed, so the phone can reach this computer.' : RB.esc(pair.error)}</div>
        <div style="display:flex; gap:8px">${off && !pair.waiting ? '<button data-act="pairOn">Turn on</button>' : ''}<button data-act="pairHide">${pair.waiting ? 'Cancel' : 'Close'}</button></div>
      </div>`;
  requestAnimationFrame(() => el.classList.add('on'));
}

setInterval(() => {
  if (full()) renderOverlay();
  if (!pair || !pair.until) return;
  if (Date.now() >= pair.until) { pair = null; return renderPair(); }
  const el = $('pairLeft'); if (el) el.textContent = pairLeft();
}, 1000);

document.addEventListener('pointermove', wake);
document.addEventListener('pointerdown', wake);
document.documentElement.addEventListener('mouseleave', () => { ov.inside = false; renderOverlay(); });
document.documentElement.addEventListener('mouseenter', () => { ov.inside = true; });
$('overlay').addEventListener('mouseenter', () => { ov.over = true; });
$('overlay').addEventListener('mouseleave', () => { ov.over = false; ov.at = Date.now(); });
// A new resolution: fit the board to it.
addEventListener('resize', () => { if (full()) render(); });
document.addEventListener('keydown', e => {
  if (e.key === 'F11') { e.preventDefault(); RB.invoke('toggle_fullscreen'); }
  // macOS catches Esc before the page does (see fullscreen.rs); this is for Windows.
  else if (e.key === 'Escape' && full()) RB.invoke('toggle_fullscreen');
  // Arrow keys move the rotation on, as a TV remote's do.
  else if (e.key === 'ArrowRight' && S && S.sources && S.sources.active) RB.invoke('sources_next');
});

// Dragging: move the native window with the pointer; the backend snaps on release.
let drag = null, pending = null;
stage.addEventListener('pointerdown', e => {
  if (e.button !== 0 || e.target.closest('[data-nodrag]') || ui.resizing || full()) return;
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
