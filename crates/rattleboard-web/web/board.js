// Rattleboard Web's board: read-only, full screen, live. It shows what the server pushes
// (Server-Sent Events) with the desktop's split-flap renderer, fitted to the window.
// Tap for full screen; the screen is kept awake where the browser allows it; the pointer
// hides when idle. The viewer token, when the board needs one, is in the URL fragment
// (#v=...), which is never sent to the server; it's passed on as ?v= for the stream.
(() => {
  const sfb = document.getElementById('sfb'), note = document.getElementById('note'), dot = document.getElementById('dot');
  const root = document.documentElement;
  const token = new URLSearchParams(location.hash.slice(1)).get('v') || '';
  const q = token ? '?v=' + encodeURIComponent(token) : '';
  let V = null;

  // The largest tile at which the whole board, frame included, fits the window with a
  // comfortable margin (as the desktop's full screen; sizes follow split-flap.js).
  function fitTile(rows, cols) {
    const W = innerWidth, H = innerHeight, m = Math.max(16, Math.round(Math.min(W, H) * 0.06));
    const fits = w => {
      const h = Math.round(w * 1.42), pad = Math.max(22, Math.round(w * 1.15));
      return cols * w + (cols - 1) * Math.max(2, Math.round(w * 0.17)) + 2 * pad <= W - 2 * m
        && rows * h + (rows - 1) * Math.max(3, Math.round(w * 0.3)) + 2 * pad <= H - 2 * m;
    };
    let lo = 3, hi = 1000;
    while (lo < hi) { const mid = Math.ceil((lo + hi) / 2); if (fits(mid)) lo = mid; else hi = mid - 1; }
    return lo;
  }

  const fit = (g, rows, cols) => Array.from({ length: rows }, (_, r) => Array.from({ length: cols }, (_, c) => (g[r] && g[r][c]) || 0));

  function render() {
    if (!V) return;
    const set = (k, v) => { if (sfb.getAttribute(k) !== String(v)) sfb.setAttribute(k, v); };
    root.dataset.finish = V.finish === 'white' ? 'white' : 'black';
    document.title = V.name || 'Rattleboard';
    set('rows', V.rows); set('cols', V.cols); set('tile', fitTile(V.rows, V.cols));
    set('finish', V.finish); set('speed', 1); set('sound', 'off');
    set('transition', JSON.stringify(V.transition || null));
    set('seq', V.seq); set('message', JSON.stringify(fit(V.display || [], V.rows, V.cols)));
    sfb.classList.add('on');
    // The credit sits in the margin fitTile leaves, so it never meets the frame.
    const m = Math.max(16, Math.round(Math.min(innerWidth, innerHeight) * 0.06)), fs = Math.min(22, Math.max(9, Math.round(innerHeight / 1080 * 11.5)));
    root.style.setProperty('--credit-fs', fs + 'px');
    root.style.setProperty('--credit-gap', Math.max(6, Math.round((m - fs) / 2)) + 'px');
  }
  addEventListener('resize', render);

  let noteTimer = 0;
  function say(text, ms) {
    clearTimeout(noteTimer);
    note.textContent = text; note.classList.toggle('on', !!text);
    if (text && ms) noteTimer = setTimeout(() => note.classList.remove('on'), ms);
  }

  // ------------------------------------------------------------ live updates

  let es = null, retry = 1000;
  function connect() {
    fetch('rattleboard/view' + q, { cache: 'no-store' }).then(r => {
      if (r.status === 401) { say(token ? 'This viewer link is out of date. Ask for a new one.' : 'This board needs its viewer link (ending in #v=…).'); throw 0; }
      if (!r.ok) throw 0;
      return r.json();
    }).then(v => {
      V = v; render(); retry = 1000;
      es = new EventSource('rattleboard/stream' + q);
      es.addEventListener('board', e => { V = JSON.parse(e.data); render(); dot.classList.remove('on'); });
      es.onopen = () => dot.classList.remove('on');
      es.onerror = () => {
        // EventSource retries by itself; after a restart it may need a fresh start.
        dot.classList.add('on');
        if (es.readyState === EventSource.CLOSED) { es = null; setTimeout(connect, retry); retry = Math.min(30000, retry * 2); }
      };
    }).catch(e => {
      dot.classList.add('on');
      if (e !== 0 || !note.classList.contains('on')) setTimeout(connect, retry);
      retry = Math.min(30000, retry * 2);
    });
  }
  connect();
  addEventListener('hashchange', () => location.reload());

  // ------------------------------------------------------------ kiosk

  // Tap or click for full screen (browsers only allow it after a gesture).
  const canFull = !!(root.requestFullscreen || root.webkitRequestFullscreen);
  const isFull = () => !!(document.fullscreenElement || document.webkitFullscreenElement);
  document.addEventListener('click', e => {
    if (e.target.closest('a') || !canFull || isFull()) return;
    (root.requestFullscreen || root.webkitRequestFullscreen).call(root).catch(() => {});
  });
  if (canFull && !isFull()) setTimeout(() => { if (!isFull()) say('Tap for full screen', 4000); }, 600);

  // Keep the screen on while the board is visible. Browsers allow this only on https:// or
  // localhost, and drop it whenever the page is hidden, so it's asked for again.
  let lock = null;
  async function keepAwake() {
    if (!('wakeLock' in navigator) || document.visibilityState !== 'visible' || lock) return;
    try { lock = await navigator.wakeLock.request('screen'); lock.addEventListener('release', () => { lock = null; }); } catch (e) { lock = null; }
  }
  document.addEventListener('visibilitychange', keepAwake);
  document.addEventListener('click', keepAwake);
  keepAwake();

  // The pointer hides after a few seconds of stillness.
  let idleTimer = 0;
  function wake() {
    root.classList.remove('idle');
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => root.classList.add('idle'), 3000);
  }
  document.addEventListener('pointermove', wake);
  document.addEventListener('pointerdown', wake);
  wake();
})();
