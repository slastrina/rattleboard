// The screen saver's board (macOS .saver and Windows .scr host this page). The host calls
// saverUpdate(view) about once a second with what the Rattleboard app on this computer is
// showing (its local screen saver feed), or saverUpdate(null) when the app isn't running.
// With nothing from the app, or a blank board, it shows the time and date instead.
//
// Against burn-in (OLED screens especially): the board and its credit drift a few pixels
// every minute, and every few minutes every flap turns over: a wave of colour sweeps the
// board, then the flaps carry on round their drums back to what was there. Nothing on screen
// stays in the same place for long.
(() => {
  const sfb = document.getElementById('sfb'), stage = document.getElementById('stage');
  const root = document.documentElement;
  // The little preview in System Settings: no waves, and flaps shown where they land.
  const preview = !!window.RB_SAVER_PREVIEW || location.hash === '#preview';

  const DRIFT_MS = 60 * 1000;
  const WIPE_EVERY_MS = 5 * 60 * 1000;
  const WIPE_HOLD_MS = 3500;
  // Without a word from the host for this long, the app's board is taken to be gone.
  const STALE_MS = 6000;

  // ---------------------------------------------------------------- text into flaps

  const SYM = { '!': 37, '@': 38, '#': 39, '$': 40, '(': 41, ')': 42, '-': 44, '+': 46, '&': 47, '=': 48, ';': 49, ':': 50,
    "'": 52, '"': 53, '%': 54, ',': 55, '.': 56, '/': 59, '?': 60, '°': 62 };
  const code = ch => {
    const c = ch.toUpperCase();
    if (c >= 'A' && c <= 'Z') return c.charCodeAt(0) - 64;
    if (c >= '1' && c <= '9') return c.charCodeAt(0) - 48 + 26;
    if (c === '0') return 36;
    return SYM[c] || 0;
  };
  const blank = (rows, cols) => Array.from({ length: rows }, () => Array(cols).fill(0));
  function line(g, r, text) {
    const cols = g[0].length, t = [...text.slice(0, cols)], at = Math.floor((cols - t.length) / 2);
    t.forEach((ch, i) => (g[r][at + i] = code(ch)));
  }

  // ---------------------------------------------------------------- the time and date

  const hour12 = (() => {
    try { return new Intl.DateTimeFormat(undefined, { hour: 'numeric' }).resolvedOptions().hour12 !== false; } catch (e) { return true; }
  })();
  function clockGrid(rows, cols) {
    const now = new Date(), g = blank(rows, cols);
    const day = now.toLocaleDateString('en', { weekday: 'long' });
    const date = `${now.getDate()} ${now.toLocaleDateString('en', { month: 'long' })} ${now.getFullYear()}`;
    let h = now.getHours();
    const m = String(now.getMinutes()).padStart(2, '0');
    const time = hour12 ? `${h % 12 || 12}:${m} ${h < 12 ? 'AM' : 'PM'}` : `${String(h).padStart(2, '0')}:${m}`;
    if (rows >= 6) { line(g, 1, day); line(g, 2, date); line(g, 4, time); }
    else { line(g, 0, day); line(g, 1, time); }
    return g;
  }

  // ---------------------------------------------------------------- colour waves

  const RAINBOW = [63, 64, 65, 66, 67, 68];
  const PATTERNS = [
    (r, c) => RAINBOW[(r + c) % 6],
    (r, c) => RAINBOW[Math.floor(c / 2) % 6],
    (r, c) => RAINBOW[(r + 6 - (c % 6)) % 6],
    (r, c) => ((r + c) % 2 ? 69 : RAINBOW[r % 6]),
    (r, c, rows, cols) => RAINBOW[Math.floor((Math.abs(c - (cols - 1) / 2) + Math.abs(r - (rows - 1) / 2)) / 1.5) % 6],
  ];
  const STRATEGIES = ['diagonal', 'column', 'edges-to-center', 'reverse-column', 'row'];
  let wave = Math.floor(Math.random() * PATTERNS.length);
  const pattern = (rows, cols) => {
    const p = PATTERNS[wave++ % PATTERNS.length];
    return Array.from({ length: rows }, (_, r) => Array.from({ length: cols }, (_, c) => p(r, c, rows, cols)));
  };

  // ---------------------------------------------------------------- what's shown

  let view = null, heard = 0, seq = 0, wiping = false, lastWipe = Date.now(), shown = '';

  // The app's board when it's running and showing something, otherwise the clock.
  function current() {
    const live = view && Date.now() - heard < STALE_MS && Array.isArray(view.display)
      && view.display.some(row => row.some(c => c));
    if (live) {
      const rows = view.rows || view.display.length, cols = view.cols || (view.display[0] || []).length;
      return { rows, cols, finish: view.finish === 'white' ? 'white' : 'black', grid: fit(view.display, rows, cols),
        transition: view.transition || null, key: 'app:' + (view.seq || 0) };
    }
    return { rows: 6, cols: 22, finish: 'black', grid: clockGrid(6, 22), transition: { strategy: 'column', stepIntervalMs: 40 }, key: 'clock' };
  }
  const fit = (g, rows, cols) => Array.from({ length: rows }, (_, r) => Array.from({ length: cols }, (_, c) => (g[r] && g[r][c]) || 0));

  // The largest tile at which the board fits with room left over to drift in.
  function fitTile(rows, cols) {
    const W = innerWidth, H = innerHeight, m = Math.max(24, Math.round(Math.min(W, H) * 0.1));
    const fits = w => {
      const h = Math.round(w * 1.42), pad = Math.max(22, Math.round(w * 1.15));
      return cols * w + (cols - 1) * Math.max(2, Math.round(w * 0.17)) + 2 * pad <= W - 2 * m
        && rows * h + (rows - 1) * Math.max(3, Math.round(w * 0.3)) + 2 * pad + 40 <= H - 2 * m;
    };
    let lo = 3, hi = 1000;
    while (lo < hi) { const mid = Math.ceil((lo + hi) / 2); if (fits(mid)) lo = mid; else hi = mid - 1; }
    return lo;
  }

  function put(b, grid, transition, replay) {
    const set = (k, v) => { if (sfb.getAttribute(k) !== String(v)) sfb.setAttribute(k, v); };
    set('rows', b.rows); set('cols', b.cols); set('tile', fitTile(b.rows, b.cols));
    set('finish', b.finish); set('speed', 1); set('sound', 'off');
    set('transition', JSON.stringify(transition || null));
    if (replay) set('seq', ++seq);
    set('message', JSON.stringify(grid));
    const fs = Math.min(18, Math.max(9, Math.round(innerHeight / 1080 * 11)));
    root.style.setProperty('--credit-fs', fs + 'px');
    stage.classList.add('on');
  }

  function render(force) {
    if (wiping) return;
    const b = current(), msg = JSON.stringify(b.grid), key = b.key + msg + b.rows + 'x' + b.cols + b.finish;
    if (!force && key === shown) return;
    shown = key;
    put(b, b.grid, b.transition, false);
  }

  // Every flap turns over: a wave of colour, held a moment, then on round the drums back to
  // what the board shows now (from colour, every flap passes every letter and number).
  function wipe() {
    if (wiping) return;
    wiping = true; lastWipe = Date.now();
    const b = current(), strategy = STRATEGIES[wave % STRATEGIES.length];
    put(b, pattern(b.rows, b.cols), { strategy, stepIntervalMs: 70 }, false);
    setTimeout(() => {
      wiping = false; shown = '';
      const now = current();
      put(now, now.grid, { strategy, stepIntervalMs: 50 }, false);
      shown = now.key + JSON.stringify(now.grid) + now.rows + 'x' + now.cols + now.finish;
    }, WIPE_HOLD_MS);
  }

  // A slow wander round the middle of the screen, never more than the room fitTile left.
  let step = Math.random() * 1000;
  function drift() {
    step += 1;
    const room = Math.max(8, Math.round(Math.min(innerWidth, innerHeight) * 0.05));
    const x = Math.round(Math.sin(step * 0.61) * room), y = Math.round(Math.cos(step * 0.37) * room * 0.7);
    stage.style.transform = `translate(${x}px, ${y}px)`;
  }

  // The host's call: the app's view, or null when it isn't running.
  window.saverUpdate = v => {
    const was = view && Date.now() - heard < STALE_MS;
    view = v && typeof v === 'object' ? v : null;
    heard = view ? Date.now() : 0;
    render(was !== !!view);
    // The host's call still comes when the page gets no animation frames (and its timers are
    // slowed): the flaps go straight to where they're going.
    if (preview || performance.now() - lastFrame > 900) sfb.settle && sfb.settle();
  };

  // Some hosts (macOS's preview pane, a hidden window) don't give the page animation frames,
  // so flaps would never turn: then they go straight to where they're going.
  let lastFrame = performance.now();
  const frame = () => { lastFrame = performance.now(); requestAnimationFrame(frame); };
  requestAnimationFrame(frame);
  setInterval(() => { if (preview || performance.now() - lastFrame > 900) sfb.settle && sfb.settle(); }, 500);

  addEventListener('resize', () => render(true));
  // The clock's next minute, and a host gone quiet.
  setInterval(() => render(false), 1000);
  setInterval(drift, DRIFT_MS);
  if (!preview) setInterval(() => { if (Date.now() - lastWipe >= WIPE_EVERY_MS) wipe(); }, 5000);
  drift();
  render(true);
  // A first wave soon after starting, so every flap turns over at least once.
  if (!preview) setTimeout(wipe, 20000);
})();
