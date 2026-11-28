(function () {
  if (customElements.get('split-flap-board')) return;
  const SYM = { 37: '!', 38: '@', 39: '#', 40: '$', 41: '(', 42: ')', 44: '-', 46: '+', 47: '&', 48: '=', 49: ';', 50: ':', 52: "'", 53: '"', 54: '%', 55: ',', 56: '.', 59: '/', 60: '?', 62: '°' };
  const COLORS = { 63: '#d93a2f', 64: '#ec7a26', 65: '#f2c230', 66: '#2f9a52', 67: '#2b6bd1', 68: '#7d4cc2', 69: '#f3f1ec', 70: '#0d0d0d' };
  // Drum order: blank, A–Z, 1–9, 0, punctuation, colour flaps
  const DRUM = [];
  for (let i = 0; i <= 36; i++) DRUM.push(i);
  Object.keys(SYM).map(Number).sort((a, b) => a - b).forEach(k => DRUM.push(k));
  for (let i = 63; i <= 71; i++) DRUM.push(i);
  const IDX = {}; DRUM.forEach((c, i) => (IDX[c] = i));
  const glyph = c => (c >= 1 && c <= 26 ? String.fromCharCode(64 + c) : c >= 27 && c <= 35 ? String(c - 26) : c === 36 ? '0' : SYM[c] || '');
  const FIN = {
    black: { frame: '#0a0a0a', tile: '#1c1c1c', ink: '#f0eee8', seam: 'rgba(0,0,0,.9)', fill: '#f0eee8', hi: 'rgba(255,255,255,.06)', shade: 'rgba(0,0,0,.22)' },
    white: { frame: '#ecebe6', tile: '#fbfaf7', ink: '#1c1c1c', seam: 'rgba(0,0,0,.22)', fill: '#1c1c1c', hi: 'rgba(255,255,255,.7)', shade: 'rgba(0,0,0,.06)' }
  };
  const CSS = `
:host{display:block;width:max-content;line-height:1}
.board{display:grid;gap:var(--gy) var(--gx);padding:var(--pad);background:var(--frame);border-radius:calc(var(--w)*.4);
 box-shadow:inset 0 1px 0 var(--hi),0 0 0 1px rgba(0,0,0,.35),0 22px 48px rgba(0,0,0,.38),0 4px 12px rgba(0,0,0,.25)}
.t{position:relative;width:var(--w);height:var(--h);perspective:calc(var(--h)*5)}
.t::after{content:'';position:absolute;left:0;right:0;top:50%;height:1px;margin-top:-.5px;background:var(--seam);z-index:3;pointer-events:none}
.h{position:absolute;left:0;right:0;height:50%;overflow:hidden;background:var(--tile);backface-visibility:hidden}
.top{top:0;border-radius:var(--r) var(--r) 0 0;transform-origin:50% 100%}
.bot{bottom:0;border-radius:0 0 var(--r) var(--r);transform-origin:50% 0}
.bot::before{content:'';position:absolute;inset:0;background:linear-gradient(var(--shade),transparent 55%);z-index:1;pointer-events:none}
.top::before{content:'';position:absolute;inset:0;box-shadow:inset 0 1px 0 var(--hi);z-index:1;pointer-events:none}
.f{position:absolute;left:0;right:0;height:200%;display:flex;align-items:center;justify-content:center;
 font-family:'Barlow Semi Condensed','Arial Narrow',sans-serif;font-weight:600;font-size:var(--fs);color:var(--ink);background:var(--tile)}
.top .f{top:0}.bot .f{bottom:0}
.fl{visibility:hidden;will-change:transform;z-index:2}`;
  const PROPS = ['message', 'rows', 'cols', 'tile', 'speed', 'finish', 'sound', 'seq', 'transition'];
  // Which step a tile starts on, for the staggered transitions (Vestaboard's "strategy").
  const GROUP = {
    column: (r, c) => c,
    'reverse-column': (r, c, cols) => cols - 1 - c,
    'edges-to-center': (r, c, cols) => Math.min(c, cols - 1 - c),
    row: r => r,
    diagonal: (r, c) => r + c
  };

  // Bandpass resonances fitted to the click transients of the Frankfurt Airport
  // board (iainmccurdy, freesound.org/s/546828/) after subtracting the noise spectrum.
  const DEPARTURE_BANDS = [
    [2000, 2.3, 0.52], [3050, 3.8, 0.13], [3610, 4, 1],
    [6310, 4, 0.73], [7490, 5, 0.53], [8690, 3.6, 0.18]
  ];
  const DEPARTURE_INTERVAL = 0.0745;
  const DEPARTURE_REFLECTIONS = [
    // Delay and level for each stereo reflection. Later reflections lose more high end.
    [0.0087, 0.34], [0.0139, -0.30], [0.0210, 0.27], [0.0292, -0.23],
    [0.0381, 0.28], [0.0475, -0.27], [0.0590, 0.24], [0.0715, -0.19],
    [0.0850, 0.16], [0.1000, -0.11]
  ];

  function synthesizeDepartureRoom(samples, sampleRate, channel) {
    for (const [index, [delay, level]] of DEPARTURE_REFLECTIONS.entries()) {
      const side = (index % 2 ? -1 : 1) * (channel === 0 ? 1 : -1);
      const start = Math.round((delay + side * (0.0006 + index * 0.00016)) * sampleRate);
      const cutoff = 6600 / (1 + delay * 19);
      const decay = Math.exp(-2 * Math.PI * cutoff / sampleRate);
      let value = level * (1 + 0.19 * side) * (1 - decay);
      for (let i = start; i < samples.length && Math.abs(value) > 0.00001; i++) {
        samples[i] += value;
        value *= decay;
      }
    }
  }

  function synthesizeDepartureClack(samples, sampleRate) {
    for (const [frequency, quality, level] of DEPARTURE_BANDS) {
      if (frequency >= sampleRate / 2) continue;
      const angle = 2 * Math.PI * frequency / sampleRate;
      const alpha = Math.sin(angle) / (2 * quality);
      const inputGain = alpha / (1 + alpha);
      const feedback1 = 2 * Math.cos(angle) / (1 + alpha);
      const feedback2 = (alpha - 1) / (1 + alpha);
      let input1 = 0, input2 = 0, output1 = 0, output2 = 0;
      for (let i = 0; i < samples.length; i++) {
        const input = Math.random() * 2 - 1;
        const output = inputGain * (input - input2) + feedback1 * output1 + feedback2 * output2;
        samples[i] += level * output;
        input2 = input1; input1 = input;
        output2 = output1; output1 = output;
      }
    }
    let peak = 0;
    for (let i = 0; i < samples.length; i++) {
      const time = i / sampleRate;
      const attack = Math.min(1, time / 0.002);
      const decay = 0.42 * Math.exp(-time / 0.003) + 0.58 * Math.exp(-time / 0.011);
      const tail = Math.min(1, (samples.length - 1 - i) / (sampleRate * 0.006));
      samples[i] *= attack * decay * tail;
      peak = Math.max(peak, Math.abs(samples[i]));
    }
    if (!peak) return;
    const gain = 0.65 / peak;
    for (let i = 0; i < samples.length; i++) samples[i] *= gain;
  }

  class SplitFlapBoard extends HTMLElement {
    static get observedAttributes() { return PROPS; }
    constructor() {
      super();
      this._p = { rows: '6', cols: '22', tile: '24', speed: '1', finish: 'black', sound: 'off', seq: '0', message: '', transition: '' };
      this.root = this.attachShadow({ mode: 'open' });
      this.tiles = []; this._q = false; this._running = false; this._lastClack = 0;
      this.fin = FIN.black;
    }
    attributeChangedCallback(n, o, v) { this._p[n] = v; this._schedule(); }
    connectedCallback() {
      this._schedule();
      // Moved in the DOM mid-flip: carry on from where the flaps were.
      if (!this._running && this.tiles.some(t => t.cur !== t.target)) { this._running = true; this._raf = requestAnimationFrame(this._tick); }
    }
    disconnectedCallback() { cancelAnimationFrame(this._raf); this._running = false; }
    _schedule() { if (this._q) return; this._q = true; queueMicrotask(() => { this._q = false; this._update(); }); }
    _update() {
      const p = this._p, rows = +p.rows || 6, cols = +p.cols || 22, w = +p.tile || 24;
      let msg; try { msg = JSON.parse(p.message || '[]'); } catch (e) { msg = []; }
      if (rows + 'x' + cols !== this._geom) { this._geom = rows + 'x' + cols; this._build(rows, cols); this._w = null; }
      if (w !== this._w || p.finish !== this._fin) {
        this._w = w; this._fin = p.finish; this.fin = FIN[p.finish] || FIN.black;
        this._style(w); this.tiles.forEach(t => this._settle(t));
      }
      if (p.message !== this._msgKey || p.seq !== this._seq) {
        const replay = p.seq !== this._seq && p.message === this._msgKey;
        this._msgKey = p.message; this._seq = p.seq;
        if (replay) this.tiles.forEach(t => { t.cur = 0; this._settle(t); });
        this._go(msg);
      }
      this._emit('flaplayout');
    }
    _style(w) {
      const f = this.fin, s = this.board.style, h = Math.round(w * 1.42);
      const v = { '--w': w + 'px', '--h': h + 'px', '--gx': Math.max(2, Math.round(w * 0.17)) + 'px', '--gy': Math.max(3, Math.round(w * 0.3)) + 'px',
        '--pad': Math.max(22, Math.round(w * 1.15)) + 'px', '--r': Math.max(1.5, w * 0.08) + 'px', '--fs': Math.round(h * 0.6) + 'px',
        '--frame': f.frame, '--tile': f.tile, '--ink': f.ink, '--seam': f.seam, '--hi': f.hi, '--shade': f.shade };
      for (const k in v) s.setProperty(k, v[k]);
    }
    _build(rows, cols) {
      cancelAnimationFrame(this._raf); this._running = false;
      this.root.innerHTML = `<style>${CSS}</style><div class="board"></div>`;
      this.board = this.root.querySelector('.board');
      this.board.style.gridTemplateColumns = `repeat(${cols}, var(--w))`;
      const tpl = document.createElement('div'); tpl.className = 't';
      tpl.innerHTML = '<div class="h top"><div class="f"></div></div><div class="h bot"><div class="f"></div></div><div class="h top fl"><div class="f"></div></div><div class="h bot fl"><div class="f"></div></div>';
      const frag = document.createDocumentFragment();
      this.tiles = [];
      for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) {
        const el = tpl.cloneNode(true), hs = el.children;
        this.tiles.push({ el, r, c, sTop: hs[0], sBot: hs[1], fTop: hs[2], fBot: hs[3], cur: 0, target: 0, nextAt: 0, v: 0.9 + Math.random() * 0.2 });
        frag.appendChild(el);
      }
      this.board.appendChild(frag);
    }
    _paint(h, code) {
      const f = h.firstChild;
      if (code >= 63) { f.style.background = code === 71 ? this.fin.fill : COLORS[code]; f.textContent = ''; }
      else { f.style.background = ''; f.textContent = glyph(code); }
    }
    _settle(t) {
      if (t.a1) t.a1.cancel(); if (t.a2) t.a2.cancel();
      const c = DRUM[t.cur]; this._paint(t.sTop, c); this._paint(t.sBot, c);
      t.fTop.style.visibility = 'hidden'; t.fBot.style.visibility = 'hidden';
    }
    // {strategy, stepIntervalMs, stepSize}; null for classic, where every flap starts at once.
    _transition() {
      let t; try { t = JSON.parse(this._p.transition || 'null'); } catch (e) { t = null; }
      return t && t.strategy && t.strategy !== 'classic' ? t : null;
    }
    _go(msg) {
      const now = performance.now(), tr = this._transition(); let moving = false;
      let start = t => t.c * 12 + Math.random() * 160;
      if (tr) {
        // Group k starts at k × interval. A little jitter inside each group keeps the flaps of
        // one column from clacking in perfect unison, as they never quite do on the hardware.
        const gap = Math.max(1, +tr.stepIntervalMs || 120), size = Math.max(1, +tr.stepSize || 1);
        const jitter = Math.min(30, gap / 2), cols = +this._p.cols || 22, group = GROUP[tr.strategy];
        const order = new Map();
        if (!group) this.tiles.map(t => [t, Math.random()]).sort((a, b) => a[1] - b[1]).forEach(([t], i) => order.set(t, i));
        start = t => Math.floor((group ? group(t.r, t.c, cols) : order.get(t)) / size) * gap + Math.random() * jitter;
      }
      for (const t of this.tiles) {
        const code = (msg[t.r] && msg[t.r][t.c]) || 0;
        t.target = IDX[code] != null ? IDX[code] : 0;
        t.nextAt = now + start(t);
        if (t.cur !== t.target) moving = true;
      }
      if (!moving) { setTimeout(() => this._emit('flapdone'), 0); return; }
      this._emit('flapstart');
      if (!this._running) { this._running = true; this._raf = requestAnimationFrame(this._tick); }
    }
    // Every flap straight to where it's going, without turning: for a page that isn't given
    // animation frames (a screen saver's preview, for one), where the flaps would never move.
    settle() {
      cancelAnimationFrame(this._raf); this._running = false;
      let moved = false;
      for (const t of this.tiles) { if (t.cur !== t.target) { t.cur = t.target; moved = true; } this._settle(t); }
      if (moved) this._emit('flapdone');
    }
    // Browsers require a user gesture before a web board can play audio.
    async unlockSound() {
      const C = SplitFlapBoard;
      const ctx = C.ctx || (C.ctx = new (window.AudioContext || window.webkitAudioContext)());
      if (ctx.state === 'suspended') await ctx.resume();
    }
    _emit(n) { this.dispatchEvent(new CustomEvent(n, { bubbles: true, composed: true })); }
    _tick = now => {
      const d = 62 / Math.max(0.25, +this._p.speed || 1);
      let moving = 0, flips = 0, flipDuration = 0;
      for (const t of this.tiles) {
        if (t.cur === t.target) continue;
        moving++;
        if (now >= t.nextAt) {
          const dd = d * t.v; this._step(t, dd); flips++; flipDuration = dd;
          t.nextAt += dd; if (t.nextAt < now) t.nextAt = now + dd * 0.5;
        }
      }
      if (flips && this._p.sound === 'departure') this._departureClack(flips, moving === 1 ? flipDuration : 0);
      if (flips && this._p.sound === 'on') this._clack(flips);
      if (moving) this._raf = requestAnimationFrame(this._tick);
      else { this._running = false; setTimeout(() => this._emit('flapdone'), d); }
    };
    _step(t, d) {
      if (t.a1) t.a1.cancel(); if (t.a2) t.a2.cancel();
      const cur = DRUM[t.cur], ni = (t.cur + 1) % DRUM.length, nx = DRUM[ni];
      this._paint(t.sTop, nx); this._paint(t.sBot, cur); this._paint(t.fTop, cur); this._paint(t.fBot, nx);
      t.fTop.style.visibility = 'visible'; t.fBot.style.visibility = 'visible';
      const h = d * 0.5;
      t.a1 = t.fTop.animate([{ transform: 'rotateX(0deg)' }, { transform: 'rotateX(-90deg)' }], { duration: h, easing: 'cubic-bezier(.55,0,1,.7)', fill: 'forwards' });
      t.a2 = t.fBot.animate([{ transform: 'rotateX(90deg)' }, { transform: 'rotateX(-4deg)', offset: 0.8 }, { transform: 'rotateX(0deg)' }], { duration: h, delay: h, easing: 'ease-out', fill: 'both' });
      t.cur = ni;
      t.a2.onfinish = () => { this._paint(t.sBot, nx); t.fTop.style.visibility = 'hidden'; t.fBot.style.visibility = 'hidden'; };
    }
    _clack(n) {
      const now = performance.now(); if (now - this._lastClack < 15) return; this._lastClack = now;
      try {
        const C = SplitFlapBoard;
        const ctx = C.ctx || (C.ctx = new (window.AudioContext || window.webkitAudioContext)());
        if (ctx.state === 'suspended') ctx.resume();
        if (!C.buf) {
          const len = Math.floor(ctx.sampleRate * 0.02), b = ctx.createBuffer(1, len, ctx.sampleRate), ch = b.getChannelData(0);
          for (let i = 0; i < len; i++) ch[i] = (Math.random() * 2 - 1) * Math.pow(1 - i / len, 6);
          C.buf = b;
        }
        const src = ctx.createBufferSource(); src.buffer = C.buf; src.playbackRate.value = 0.8 + Math.random() * 0.5;
        const f = ctx.createBiquadFilter(); f.type = 'bandpass'; f.frequency.value = 1700 + Math.random() * 1800; f.Q.value = 0.9;
        const g = ctx.createGain(); g.gain.value = Math.min(0.5, 0.06 * Math.sqrt(n));
        src.connect(f).connect(g).connect(ctx.destination); src.start();
      } catch (e) {}
    }
    _departureClack(n, singleStepMs) {
      try {
        const C = SplitFlapBoard;
        const ctx = C.ctx || (C.ctx = new (window.AudioContext || window.webkitAudioContext)());
        if (ctx.state !== 'running') {
          if (ctx.state === 'suspended') ctx.resume().catch(() => {});
          return;
        }
        const now = ctx.currentTime;
        const start = singleStepMs ? now : Math.max(now, this._nextDepartureClack || 0);
        // A short lookahead keeps the rhythm even despite display-frame and audio-clock jitter.
        if (start - now > 0.020) return;
        if (!C.departureBuffers) {
          C.departureBuffers = Array.from({ length: 6 }, () => {
            const buffer = ctx.createBuffer(1, Math.round(ctx.sampleRate * 0.036), ctx.sampleRate);
            synthesizeDepartureClack(buffer.getChannelData(0), ctx.sampleRate);
            return buffer;
          });
          const room = ctx.createConvolver();
          const impulse = ctx.createBuffer(2, Math.round(ctx.sampleRate * 0.115), ctx.sampleRate);
          for (let channel = 0; channel < impulse.numberOfChannels; channel++) {
            synthesizeDepartureRoom(impulse.getChannelData(channel), ctx.sampleRate, channel);
          }
          room.normalize = false;
          room.buffer = impulse;
          room.connect(ctx.destination);
          C.departureRoom = room;
        }
        const src = ctx.createBufferSource();
        src.buffer = C.departureBuffers[Math.floor(Math.random() * C.departureBuffers.length)];
        const gain = ctx.createGain();
        const level = singleStepMs ? 0.14 : Math.min(0.52, 0.09 * Math.sqrt(n));
        gain.gain.value = level;
        src.connect(gain);
        gain.connect(ctx.destination);
        if (singleStepMs) {
          // A lone digit sounds every step and leaves space for the next impact at high speed.
          if (this._departureSingleSource) this._departureSingleSource.stop(start);
          const duration = Math.min(src.buffer.duration, singleStepMs * 0.0008);
          if (duration < src.buffer.duration) {
            gain.gain.setValueAtTime(level, start + duration - 0.002);
            gain.gain.linearRampToValueAtTime(0, start + duration);
          }
          src.start(start, 0, duration);
          this._departureSingleSource = src;
          src.onended = () => {
            if (this._departureSingleSource === src) this._departureSingleSource = null;
          };
        } else {
          gain.connect(C.departureRoom);
          src.start(start);
        }
        this._nextDepartureClack = start + DEPARTURE_INTERVAL + (Math.random() - 0.5) * 0.012;
      } catch (e) {}
    }
  }
  PROPS.forEach(p => Object.defineProperty(SplitFlapBoard.prototype, p, {
    get() { return this._p[p]; }, set(v) { this._p[p] = String(v); this._schedule(); }, configurable: true
  }));
  customElements.define('split-flap-board', SplitFlapBoard);
})();
