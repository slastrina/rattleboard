// Settings, Message History and the first-run setup, in one native window.
const root = document.getElementById('root');
document.documentElement.classList.add(navigator.userAgent.includes('Mac') ? 'mac' : 'win');

let S = null;
const L = {
  view: (location.hash || '#settings').slice(1),
  tab: 'display',
  showKey: false,
  secrets: { localKey: '', cloudToken: '' },
  step: 0, setupSource: 'local', scan: 'idle', found: [], setupBoard: -1,
  test: 'idle', testError: '', testLayout: null
};
const WIDTH = { settings: 600, history: 580, setup: 640 };
const WALL = {
  mac: 'linear-gradient(165deg, #8c8478 0%, #5d5853 50%, #36332f 100%)',
  win: 'linear-gradient(160deg, #7f8fa2 0%, #4b5a6f 55%, #2f3a4a 100%)'
};

const save = patch => RB.invoke('update_settings', { patch });
const esc = RB.esc;

function seg(opts, cur, act, cls = '') {
  return `<div class="seg ${cls}">${opts.map(([v, l]) => `<button class="${String(v) === String(cur) ? 'on' : ''}" data-act="${act}" data-v="${v}">${l}</button>`).join('')}</div>`;
}
const tog = (on, act) => `<div class="tog ${on ? 'on' : ''}" data-act="${act}"><div></div></div>`;
const range = (min, max, step, val, field) => `<input type="range" min="${min}" max="${max}" step="${step}" value="${val}" data-field="${field}">`;

function checkedLabel() {
  if (!S.lastChecked) return 'not yet';
  const s = Math.floor((Date.now() - S.lastChecked) / 1000);
  return s <= 0 ? 'just now' : s < 60 ? s + 's ago' : RB.ago(Date.now() - S.lastChecked);
}

function corners() {
  const s = S.settings;
  const pos = { tl: [6, 8], tr: [104, 8], bl: [6, 68], br: [104, 68] };
  let html = Object.entries(pos).map(([c, [l, t]]) =>
    `<div class="corner ${s.position === c ? 'on' : ''}" title="${RB.posNames[c]}" data-act="corner" data-v="${c}" style="left:${l}px; top:${t}px"></div>`).join('');
  if (s.position === 'custom') {
    const W = screen.width * devicePixelRatio, H = screen.height * devicePixelRatio;
    html += `<div class="corner on" style="left:${Math.round(Math.min(110, Math.max(0, s.custom.x / W * 150)))}px; top:${Math.round(Math.min(76, Math.max(0, s.custom.y / H * 94)))}px; cursor:default"></div>`;
  }
  const caption = s.position === 'custom' ? 'Custom spot. Drag the board on screen; release near a corner to snap back.' : `${RB.posNames[s.position]}. You can also drag the board anywhere on screen.`;
  return `<div style="display:flex; align-items:center; gap:14px">
    <div style="position:relative; width:150px; height:94px; border-radius:6px; background:${WALL[S.os] || WALL.mac}; border:1px solid rgba(0,0,0,.15); flex:none">${html}</div>
    <div style="color:#6b6d72; font-size:12px; line-height:1.45; text-wrap:pretty">${caption}</div></div>`;
}

const modelSeg = () => seg([['flag', 'Flagship · 6 × 22'], ['note', 'Note · 3 × 15']], S.settings.model, 'model');
const nameField = () => `<input class="field" style="justify-self:start; width:240px" value="${esc(S.settings.boardName)}" data-field="boardName" placeholder="Vestaboard">`;

// ---------------------------------------------------------------- settings

function viewSettings() {
  const s = S.settings;
  let body = '';
  if (L.tab === 'display') {
    body = `<div class="grid">
      <div class="lbl">Name</div>${nameField()}
      <div class="lbl">Board</div>${modelSeg()}
      <div class="lbl">Finish</div>${seg([['black', 'Black'], ['white', 'White']], s.finish, 'finish')}
      <div class="lbl">Size on screen</div>${seg([['s', 'Small'], ['m', 'Medium'], ['l', 'Large']], s.size, 'size')}
      <div class="lbl" style="align-self:start; padding-top:4px">Pop up in</div>${corners()}
    </div>`;
  } else if (L.tab === 'behavior') {
    body = `<div class="grid">
      <div class="lbl">Hide after</div>
      <div style="display:flex; align-items:center; gap:10px">${range(3, 60, 1, s.hideSec, 'hideSec')}<span id="hideLabel" style="font-variant-numeric:tabular-nums; min-width:44px">${s.hideSec} s</span></div>
      <div class="lbl">Pause on hover</div>
      <div style="display:flex; align-items:center; gap:10px">${tog(s.pauseOnHover, 'pauseOnHover')}<span class="hint">Countdown waits while the pointer is over the board</span></div>
      <div class="lbl">Pin as widget</div>
      <div style="display:flex; align-items:center; gap:10px">${tog(s.pinned, 'pinned')}<span class="hint">Board stays on the desktop, below other windows</span></div>
      <div class="lbl">Flip speed</div>
      <div style="display:flex; align-items:center; gap:10px">${range(0.5, 2, 0.1, s.speed, 'speed')}<span id="speedLabel" style="font-variant-numeric:tabular-nums">${speedLabel(s.speed)}</span></div>
      <div class="lbl">Flap sound</div>
      <div style="display:flex; align-items:center; gap:10px">${tog(s.sound, 'sound')}<span class="hint">Mechanical clatter while flaps turn</span></div>
    </div>`;
  } else {
    const local = s.source === 'local', demo = s.source === 'demo', key = local ? L.secrets.localKey : L.secrets.cloudToken;
    const keyType = L.showKey ? 'text' : 'password', keyBtn = L.showKey ? 'Hide' : 'Show';
    const connDot = !s.polling ? '#9a9ca1' : S.conn === 'testing' ? '#e0a526' : S.conn === 'error' ? '#e0553f' : S.conn === 'ok' ? '#34b05a' : '#9a9ca1';
    const connText = S.conn === 'testing' ? 'Testing…' : !s.polling ? 'Paused' : S.conn === 'error' ? (S.error || "Can't reach the board") : S.conn === 'ok' ? `Connected · checked ${checkedLabel()}` : 'Not checked yet';
    const url = local ? (/:\d+$/.test(s.host.trim()) ? `http://${s.host.trim()}/local-api/message` : `http://${s.host.trim() || '[BOARD_IP]'}:7000/local-api/message`) : '';
    const lines = demo
      ? [['No network requests: sample messages from the built-in demo', '#ecebe6'], [`→ a new message every 20 s   ${RB.dims(s.model).rows} × ${RB.dims(s.model).cols} flap codes`, '#9fd3a8']]
      : local
      ? [[`GET ${url}`, '#ecebe6'], [`X-Vestaboard-Local-Api-Key: ${RB.mask(key)}`, '#a9c4f5'], [`→ [[0,0,1,24,9,…], …]   ${RB.dims(s.model).rows} × ${RB.dims(s.model).cols} flap codes`, '#9fd3a8']]
      : [['GET https://cloud.vestaboard.com/', '#ecebe6'], [`X-Vestaboard-Token: ${RB.mask(key)}`, '#a9c4f5'], ['→ { "id": "…", "layout": [[0,0,0,1,2,3,…], …] }', '#9fd3a8']];
    const note = demo ? 'Switch to Local network or Cloud when you have a board.' : local ? 'A change is detected when the returned layout differs from the previous check.' : 'A change is detected when the message id differs from the previous check.';
    body = `<div class="grid" style="gap:14px 16px">
      <div class="lbl">Read from</div>${seg([['local', 'Local network'], ['cloud', 'Cloud'], ['demo', 'Demo']], s.source, 'source')}
      ${demo ? '' : local ? `
        <div class="lbl">Board IP</div><input class="field" style="justify-self:start; width:240px" value="${esc(s.host)}" data-field="host">
        <div class="lbl">Local API key</div>
        <div style="display:flex; gap:6px"><input class="field" style="width:240px" type="${keyType}" value="${esc(key)}" data-field="localKey"><button class="btn" style="padding:0 10px" data-act="showKey">${keyBtn}</button></div>`
      : `
        <div class="lbl">Cloud API token</div>
        <div style="display:flex; gap:6px"><input class="field" style="width:240px" type="${keyType}" value="${esc(key)}" data-field="cloudToken" placeholder="Paste your token"><button class="btn" style="padding:0 10px" data-act="showKey">${keyBtn}</button></div>`}
      ${demo ? '' : `<div class="lbl">Check every</div>${seg([[1, '1 s'], [2, '2 s'], [5, '5 s'], [15, '15 s'], [30, '30 s']], s.pollEvery, 'poll', 'poll')}`}
      <div class="lbl">Status</div>
      <div style="display:flex; align-items:center; gap:10px; min-width:0">
        <div class="dot" style="background:${connDot}"></div><span id="connText" style="white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(connText)}</span>
        <button class="btn" style="margin-left:auto; flex:none" data-act="test">Test connection</button>
      </div>
      <div class="mono" style="grid-column:1 / -1; background:#1d1e21; border-radius:8px; padding:12px 14px; font-size:11.5px; line-height:1.7; display:flex; flex-direction:column; overflow:hidden">
        ${lines.map(([t, c]) => `<div style="color:${c}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(t)}</div>`).join('')}
        <div style="color:#8e9096; font-family:-apple-system, 'Segoe UI', system-ui, sans-serif; font-size:11.5px; margin-top:6px">${note}</div>
      </div>
      <div class="lbl">Open at login</div>${tog(s.launchLogin, 'launchLogin')}
    </div>`;
  }
  return `<div style="padding:16px 24px 22px; display:flex; flex-direction:column; gap:18px">
    <div style="display:flex; justify-content:center">${seg([['display', 'Display'], ['behavior', 'Behavior'], ['connection', 'Connection']], L.tab, 'tab', 'tabs')}</div>
    ${body}</div>`;
}
const speedLabel = v => (+v).toFixed(1) + '×' + (+v === 1 ? ' · true to hardware' : '');

// ---------------------------------------------------------------- history

function viewHistory() {
  const s = S.settings, flag = s.model !== 'note', fin = RB.fin(s.finish);
  const cw = flag ? 5 : 7, chh = flag ? 7 : 10, now = new Date();
  const shown = JSON.stringify(RB.fit(S.display, s.model));
  const y = new Date(now); y.setDate(now.getDate() - 1);
  const day = t => { const d = new Date(t).toDateString(); return d === now.toDateString() ? 'Today' : d === y.toDateString() ? 'Yesterday' : new Date(t).toLocaleDateString('en-US', { month: 'short', day: 'numeric' }); };
  const rows = S.history.map((h, i) => {
    const g = RB.fit(h.layout, s.model), onNow = i === 0 && JSON.stringify(g) === shown;
    const thumb = g.map(r => `<div style="display:flex; gap:1px">${r.map(c => `<div style="width:${cw}px; height:${chh}px; border-radius:.5px; background:${c >= 63 ? RB.HEX[c] || fin.ink : c ? fin.dim : fin.tile}"></div>`).join('')}</div>`).join('');
    return `<div style="display:grid; grid-template-columns:auto minmax(0,1fr) auto; gap:14px; align-items:center; padding:12px 18px; border-bottom:1px solid rgba(0,0,0,.06); background:${onNow ? 'rgba(0,0,0,.03)' : 'transparent'}">
      <div style="background:${fin.frame}; padding:5px; border-radius:4px; display:flex; flex-direction:column; gap:1px">${thumb}</div>
      <div style="display:flex; flex-direction:column; gap:4px; min-width:0">
        <div class="flap-font" style="font-size:14px; letter-spacing:.04em; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(RB.decode(g) || '(blank)')}</div>
        <div style="font-size:11.5px; color:#6b6d72">${day(h.at)} ${RB.fmtTime(h.at)} · ${esc(h.src)}${onNow ? ' · on board now' : ''}</div>
      </div>
      <button class="btn" data-act="flip" data-v="${h.id}">Flip again</button>
    </div>`;
  }).join('');
  const n = S.history.length;
  return `<div style="display:flex; flex-direction:column">
    <div style="display:flex; align-items:center; justify-content:space-between; padding:12px 18px; border-bottom:1px solid rgba(0,0,0,.08)">
      <span style="color:#55575c">${n ? `${n} change${n > 1 ? 's' : ''} detected` : 'No changes yet'}</span>
      <button class="btn" style="padding:3px 10px" data-act="clear">Clear</button>
    </div>
    <div style="max-height:470px; overflow:auto; display:flex; flex-direction:column">
      ${rows || `<div style="padding:40px 18px; text-align:center; color:#6b6d72">No changes yet. New board messages appear here as they're detected.</div>`}
    </div></div>`;
}

// ---------------------------------------------------------------- setup

function viewSetup() {
  const s = S.settings, st = L.step, ACC = RB.ACC;
  const steps = ['Connection', 'Find board', 'Popup', 'Done'].map((l, i) =>
    `<div style="flex:1; display:flex; flex-direction:column; gap:6px"><div style="height:3px; border-radius:2px; background:${i <= st ? ACC : 'rgba(0,0,0,.12)'}"></div><div style="font-size:11.5px; color:${i === st ? '#1b1b1d' : '#8a8c91'}">${l}</div></div>`).join('');
  let body = '';
  if (st === 0) {
    const card = (v, title, tag, text, needs) => `<div data-act="setupSource" data-v="${v}" style="border-radius:10px; padding:14px; background:#fff; box-shadow:${L.setupSource === v ? `0 0 0 2px ${ACC}` : '0 0 0 1px rgba(0,0,0,.12)'}; cursor:pointer; display:flex; flex-direction:column; gap:6px">
      <div style="display:flex; justify-content:space-between; align-items:center"><span style="font-weight:600">${title}</span><span style="font-size:11px; color:${v === 'local' ? '#2f8a4c' : '#6b6d72'}">${tag}</span></div>
      <div style="color:#55575c; font-size:12.5px; line-height:1.45">${text}</div>
      <div class="mono" style="font-size:11px; color:#6b6d72">${needs}</div></div>`;
    body = `<div style="font-size:20px; font-weight:600; letter-spacing:-.01em">Mirror your board on this computer</div>
      <div style="color:#55575c; line-height:1.5; max-width:520px; text-wrap:pretty">Rattleboard checks your board in the background. When the message changes, a live replica flips into view and tucks away again. How should it read the board?</div>
      <div style="display:grid; grid-template-columns:1fr 1fr; gap:12px">
        ${card('local', 'Local network', 'Recommended', 'Reads the board directly over your Wi‑Fi. Fastest, and works without internet.', 'Needs: Local API key')}
        ${card('cloud', 'Cloud', '', 'Works from anywhere, including when this computer is away from home.', 'Needs: Cloud API token')}
        <div style="grid-column:1 / -1">${card('demo', 'Demo', 'No board needed', 'Flips through sample messages every 20 seconds so you can try Rattleboard first.', 'Needs: nothing')}</div>
      </div>`;
  } else if (st === 1) {
    if (L.setupSource === 'demo') {
      body = `<div style="font-size:17px; font-weight:600">Try it with sample messages</div>
        <div style="color:#55575c; line-height:1.5; max-width:520px">Rattleboard will flip to a new sample message every 20 seconds. Switch to your real board any time in Settings → Connection.</div>
        <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:12px; align-items:center">
          <div class="lbl">Board</div>${modelSeg()}
          <div class="lbl">Name</div>${nameField()}
        </div>`;
    } else if (L.setupSource === 'local') {
      const found = L.found.map((f, i) => {
        const on = L.setupBoard === i;
        return `<div data-act="pickBoard" data-v="${i}" style="display:flex; align-items:center; gap:12px; padding:10px 14px; cursor:pointer; background:${on ? 'rgba(0,0,0,.035)' : 'transparent'}; border-radius:8px">
          <div style="width:16px; height:16px; border-radius:50%; box-shadow:${on ? `inset 0 0 0 5px ${ACC}` : 'inset 0 0 0 1.5px rgba(0,0,0,.3)'}; flex:none"></div>
          <div style="display:flex; flex-direction:column; gap:2px"><span style="font-weight:600">Vestaboard</span><span style="font-size:11.5px; color:#6b6d72">${esc(f.name)} · ${esc(f.ip)}</span></div></div>`;
      }).join('');
      const scanMsg = L.scan === 'scanning' ? 'Searching your network…' : L.scan === 'done' && !L.found.length ? 'No board answered at vestaboard.local. Enter its IP address below.' : '';
      body = `<div style="font-size:17px; font-weight:600">Find your board</div>
        <div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1)">
          ${scanMsg ? `<div style="padding:14px; color:#6b6d72">${scanMsg}</div>` : ''}${found}
        </div>
        <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:12px; align-items:center">
          <div class="lbl">Or enter IP</div><input class="field" style="justify-self:start; width:220px" value="${esc(s.host)}" data-field="host">
          <div class="lbl">Local API key</div><input class="field" style="width:100%; max-width:340px" value="${esc(L.secrets.localKey)}" data-field="localKey" placeholder="From the board's Local API settings">
          <div class="lbl">Name</div>${nameField()}
        </div>`;
    } else {
      body = `<div style="font-size:17px; font-weight:600">Connect through the cloud</div>
        <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:12px; align-items:center">
          <div class="lbl">Cloud API token</div><input class="field" style="width:100%; max-width:340px" value="${esc(L.secrets.cloudToken)}" data-field="cloudToken" placeholder="Paste your token">
          <div class="lbl">Name</div>${nameField()}
        </div>
        <div style="color:#6b6d72; font-size:12px">Rattleboard reads <span class="mono">cloud.vestaboard.com</span> and only reacts when the message id changes.</div>`;
    }
    const dot = L.test === 'ok' ? '#34b05a' : L.test === 'testing' ? '#e0a526' : L.test === 'error' ? '#e0553f' : '#b9bbc0';
    const text = L.test === 'ok' ? (L.setupSource === 'demo' ? 'Demo ready' : 'Connected') : L.test === 'testing' ? 'Reaching your board…' : L.test === 'error' ? L.testError : 'Not tested yet';
    body += `<div style="display:flex; align-items:center; gap:12px">
        <button class="btn" style="font-size:12.5px; padding:5px 12px" data-act="setupTest">Test connection</button>
        <div style="display:flex; align-items:center; gap:6px; color:#55575c"><div class="dot" style="background:${dot}"></div>${esc(text)}</div>
      </div>`;
    if (L.test === 'ok' && L.testLayout) {
      const flag = s.model !== 'note';
      body += `<div style="display:flex; align-items:center; gap:14px">
        <div style="background:${RB.fin(s.finish).frame}; padding:8px; border-radius:6px; display:flex; flex-direction:column; gap:3px; flex:none">${RB.mini(L.testLayout, S)}</div>
        <div style="color:#55575c; font-size:12.5px; line-height:1.45">Detected a ${flag ? 'Flagship board (6 × 22)' : 'Note board (3 × 15)'}. This is what it shows right now.</div></div>`;
    }
  } else if (st === 2) {
    body = `<div style="font-size:17px; font-weight:600">Where should it appear?</div>
      <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:16px; align-items:center">
        <div class="lbl" style="align-self:start; padding-top:4px">Pop up in</div>${corners()}
        <div class="lbl">Hide after</div>
        <div style="display:flex; align-items:center; gap:10px">${range(3, 60, 1, s.hideSec, 'hideSec')}<span id="hideLabel" style="font-variant-numeric:tabular-nums">${s.hideSec} s</span></div>
        <div class="lbl">Board</div>${modelSeg()}
      </div>`;
  } else {
    const summary = [
      ['Reading from', s.source === 'local' ? `Local network · ${s.host}` : s.source === 'demo' ? 'Demo · sample messages' : 'Cloud API'],
      ...(s.source === 'demo' ? [] : [['Checks every', s.pollEvery + ' s']]),
      ['Pops up', `${s.position === 'custom' ? 'Custom spot' : RB.posNames[s.position]} · hides after ${s.hideSec} s`]
    ];
    body = `<div style="font-size:20px; font-weight:600">You're set</div>
      <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:10px 16px">${summary.map(([k, v]) => `<div style="color:#6b6d72">${k}</div><div>${esc(v)}</div>`).join('')}</div>
      <div style="color:#55575c; line-height:1.5">Rattleboard now lives in your ${S.os === 'mac' ? 'menu bar' : 'system tray'}. Click its icon any time for the latest message, history and settings.</div>`;
  }
  return `<div style="display:flex; flex-direction:column">
    <div style="display:flex; gap:6px; padding:16px 24px 0">${steps}</div>
    <div style="padding:20px 24px 8px; min-height:300px; display:flex; flex-direction:column; gap:16px">${body}</div>
    <div style="display:flex; justify-content:space-between; padding:14px 24px 18px">
      <button class="btn" style="font-size:13px; padding:5px 14px; opacity:${st === 0 ? 0.4 : 1}" data-act="back">Back</button>
      <button class="primary" data-act="next">${st === 3 ? 'Finish' : 'Continue'}</button>
    </div></div>`;
}

// ---------------------------------------------------------------- render loop

let lastFit = '', pointerDown = false;
function render(force) {
  if (!S) return;
  const a = document.activeElement;
  if (!force && (pointerDown || (a && a.tagName === 'INPUT' && a.type !== 'range'))) return;
  const v = L.view;
  root.innerHTML = v === 'history' ? viewHistory() : v === 'setup' ? viewSetup() : viewSettings();
  const fit = [v, L.step, L.tab, L.setupSource, S.settings.source].join('/');
  if (fit !== lastFit) { lastFit = fit; RB.invoke('panel_resize', { width: WIDTH[v] || 600, height: Math.ceil(root.offsetHeight) }); }
}
document.addEventListener('pointerdown', () => (pointerDown = true), true);
document.addEventListener('pointerup', () => { pointerDown = false; setTimeout(render, 0); }, true);
document.addEventListener('focusout', () => setTimeout(render, 0));

function setView(v) {
  L.view = v;
  if (v === 'setup') Object.assign(L, { step: 0, test: 'idle', testLayout: null, setupSource: S ? S.settings.source : 'local' });
  render(true);
}
RB.listen('rb://view', e => setView(e.payload));

const testFlow = () => {
  L.test = 'testing'; render(true);
  RB.invoke('test_connection')
    .then(r => { L.test = 'ok'; L.testLayout = r.layout; })
    .catch(e => { L.test = 'error'; L.testError = String(e); })
    .finally(() => render(true));
};

RB.delegate(root, 'click', {
  tab: el => { L.tab = el.dataset.v; L.showKey = false; render(true); },
  model: el => save({ model: el.dataset.v }),
  finish: el => save({ finish: el.dataset.v }),
  size: el => save({ size: el.dataset.v }),
  corner: el => save({ position: el.dataset.v }),
  source: el => save({ source: el.dataset.v, pollEvery: el.dataset.v === 'cloud' ? 15 : 2 }),
  poll: el => save({ pollEvery: +el.dataset.v }),
  pauseOnHover: () => save({ pauseOnHover: !S.settings.pauseOnHover }),
  pinned: () => { save({ pinned: !S.settings.pinned }); RB.invoke('show_board'); },
  sound: () => save({ sound: !S.settings.sound }),
  launchLogin: () => save({ launchLogin: !S.settings.launchLogin }),
  showKey: () => { L.showKey = !L.showKey; render(true); },
  test: () => RB.invoke('test_connection').catch(() => {}),
  clear: () => RB.invoke('clear_history'),
  flip: el => RB.invoke('show_history_entry', { id: +el.dataset.v }),
  setupSource: el => { L.setupSource = el.dataset.v; render(true); },
  pickBoard: el => { const i = +el.dataset.v; L.setupBoard = i; save({ host: L.found[i].ip }); },
  setupTest: testFlow,
  back: () => { L.step = Math.max(0, L.step - 1); render(true); },
  next: () => {
    if (L.step === 3) return RB.invoke('finish_setup');
    L.step++;
    if (L.step === 1) {
      save({ source: L.setupSource, pollEvery: L.setupSource === 'cloud' ? 15 : 2 });
      L.test = 'idle'; L.testLayout = null;
      if (L.setupSource === 'local' && L.scan === 'idle') {
        L.scan = 'scanning';
        RB.invoke('scan_boards').then(f => {
          L.found = f; L.scan = 'done';
          if (f.length && (!S.settings.host || S.settings.host === 'vestaboard.local')) { L.setupBoard = 0; save({ host: f[0].ip }); }
          render(true);
        });
      }
    }
    render(true);
  }
});

// Text fields commit when they lose focus (or on Enter); sliders update live and save on release.
root.addEventListener('change', e => {
  const f = e.target.dataset.field; if (!f) return;
  const v = e.target.value;
  if (f === 'localKey' || f === 'cloudToken') {
    L.secrets[f] = v.trim();
    RB.invoke('set_secret', { kind: f === 'cloudToken' ? 'cloud' : 'local', value: v });
  } else if (f === 'host' || f === 'boardName') save({ [f]: v.trim() });
  else if (f === 'hideSec') save({ hideSec: +v });
  else if (f === 'speed') save({ speed: +v });
});
root.addEventListener('input', e => {
  const f = e.target.dataset.field;
  if (f === 'hideSec') document.getElementById('hideLabel').textContent = e.target.value + ' s';
  if (f === 'speed') document.getElementById('speedLabel').textContent = speedLabel(e.target.value);
});
root.addEventListener('keydown', e => { if (e.key === 'Enter' && e.target.tagName === 'INPUT') e.target.blur(); });
document.addEventListener('keydown', e => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'w') RB.invoke('close_window', { label: 'panel' });
});

RB.invoke('get_secrets').then(sec => { L.secrets = sec; render(true); });
RB.onState(snap => {
  const first = !S; S = snap;
  if (first && L.view === 'setup') L.setupSource = S.settings.source;
  render();
});
setInterval(() => { if (L.view === 'settings' && L.tab === 'connection') render(); }, 1000);
