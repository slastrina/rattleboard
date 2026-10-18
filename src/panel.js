// Settings, Message History, Compose (compose.js) and the first-run setup, in one native window.
const root = document.getElementById('root');
document.documentElement.classList.add(navigator.userAgent.includes('Mac') ? 'mac' : 'win');

let S = null;
const L = {
  view: (location.hash || '#settings').slice(1).split('/')[0],
  tab: (location.hash.split('/')[1]) || 'display',
  showKey: false,
  secrets: { localKey: '', cloudToken: '', serverKey: '' },
  showServerKey: false, copied: '',
  step: 0, setupSource: 'virtual', scan: 'idle', found: [], setupBoard: -1, su: null, suBusy: false, suError: '', suRotation: null,
  test: 'idle', testError: '', testLayout: null,
  // Settings → Boards: the add form's fields (keys stay in memory only until they're saved)
  addMode: 'nearby', form: { host: '', key: '', name: '', link: '', token: '' }, found2: undefined, adding: false, boardMsg: null,
  renaming: null, removing: null, notifyError: '',
  // Settings → API server → Pair a phone
  pair: null, pairAsk: false
};
const WIDTH = { settings: 640, history: 680, setup: 640, compose: 780 };
const PAIR_MS = 2 * 60 * 1000;
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
  const caption = `${RB.posNames[s.position]}. You can also drag the board toward another corner; it snaps to the nearest one.`;
  return `<div style="display:flex; align-items:center; gap:14px">
    <div style="position:relative; width:150px; height:94px; border-radius:6px; background:${WALL[S.os] || WALL.mac}; border:1px solid rgba(0,0,0,.15); flex:none">${html}</div>
    <div style="color:#6b6d72; font-size:12px; line-height:1.45; text-wrap:pretty">${caption}</div></div>`;
}

const modeSeg = () => seg([['toast', 'Pop-up'], ['float', 'Pinned'], ['fullscreen', 'Full screen']], S.settings.mode, 'mode');
const modeHint = () => S.settings.mode === 'float'
  ? 'Stays on screen wherever you drop it. Rattleboard remembers its spot and size.'
  : S.settings.mode === 'fullscreen'
  ? `Covers the whole display and keeps it awake, like a board on the wall. ${S.os === 'mac' ? '⌃⌘F' : 'F11'} or Esc leaves full screen; move the pointer for controls.`
  : 'Flips into view in a corner when the message changes, then tucks away.';
const sizeRow = () => S.settings.mode === 'fullscreen' ? '' : `<div class="lbl">Tile size</div>
  <div style="display:flex; align-items:center; gap:10px">${range(0.4, 3, 0.05, S.settings.scale, 'scale')}<span id="scaleLabel" style="font-variant-numeric:tabular-nums; min-width:44px">${Math.round(S.settings.scale * 100)}%</span></div>
  <div></div><div class="hint" style="margin-top:-10px">Or hover over the board and drag its corner handle. A smaller message makes a smaller board at the same tile size.</div>`;
// Where/how the board shows up, shared by Settings → Display and setup.
function placementRows() {
  const s = S.settings;
  return `<div class="lbl">Show as</div>
    <div style="display:flex; flex-direction:column; gap:6px">${modeSeg()}<span class="hint">${modeHint()}</span></div>
    ${s.mode === 'float'
      ? `<div class="lbl">Layer</div>${seg([['desktop', 'On the desktop'], ['top', 'Above other windows']], s.floatLayer, 'floatLayer')}`
      : s.mode === 'fullscreen' ? screenRow()
      : `<div class="lbl" style="align-self:start; padding-top:4px">Pop up in</div>${corners()}`}`;
}
// Full screen's display, when there's more than one: remembered by name, and by position to
// tell two of the same model apart. The main display is the default and the fallback.
function screenRow() {
  const s = S.settings, list = L.screens || [];
  if (list.length < 2) return '';
  const cur = list.findIndex(d => d.name === s.screenName && d.x === s.screenPos.x && d.y === s.screenPos.y);
  const named = cur < 0 ? list.findIndex(d => d.name === s.screenName) : cur;
  const opt = (d, i) => `<option value="${i}" ${i === named ? 'selected' : ''}>${esc(d.name || 'Display ' + (i + 1))} · ${d.width} × ${d.height}${d.primary ? ' (main)' : ''}</option>`;
  return `<div class="lbl">Display</div>
    <select class="field" style="justify-self:start; width:280px" data-field="screen">
      <option value="" ${s.screenName && named >= 0 ? '' : 'selected'}>Main display</option>${list.map(opt).join('')}</select>`;
}
const loadScreens = () => RB.invoke('list_screens').then(l => { L.screens = l; render(true); });
// Board size: the presets, or a custom size (validated here and in the backend).
const presetSeg = (cur, act = 'model', rows = S.settings.rows, cols = S.settings.cols, fields = ['rows', 'cols']) => {
  const presets = seg([...RB.PRESETS.map(([v, l]) => [v, l]), ['custom', 'Custom']], cur, act);
  const size = (RB.PRESETS.find(p => p[0] === cur) || [0, 0, rows, cols]).slice(2);
  return `<div style="display:flex; flex-direction:column; gap:6px; align-items:flex-start">${presets}${cur === 'custom' ? `<div style="display:flex; align-items:center; gap:6px">
    <input class="field" type="number" min="2" max="12" style="width:60px" value="${rows}" data-field="${fields[0]}"><span class="hint">rows ×</span>
    <input class="field" type="number" min="8" max="40" style="width:60px" value="${cols}" data-field="${fields[1]}"><span class="hint">columns (2–12 × 8–40)</span></div>`
    : `<span class="hint">${size[0]} rows × ${size[1]} columns</span>`}</div>`;
};
const modelSeg = () => presetSeg(S.settings.model);
// Settings → Display → Size: each message at its own size (the default), or always one size.
const boardSizeRows = () => {
  const s = S.settings;
  return `<div class="lbl" style="align-self:start; padding-top:4px">Size</div>
    <div style="display:flex; flex-direction:column; gap:8px; align-items:flex-start">
      ${seg([['auto', 'Match each message'], ['fixed', 'Fixed']], s.adaptive ? 'auto' : 'fixed', 'adaptive')}
      <span class="hint">${s.adaptive ? 'Each message shows at the size it was sent at, and the board resizes to fit. Text that doesn’t say, and the demo, use:' : 'Every message is centred on a board of this size:'}</span>
      ${modelSeg()}
    </div>`;
};
const nameField = () => `<input class="field" style="justify-self:start; width:240px" value="${esc(S.settings.boardName)}" data-field="boardName" placeholder="Rattleboard">`;

// ---------------------------------------------------------------- settings

// Settings → Behavior → "Check for updates" and what it found. Before a check, a newer
// version an automatic check found shows the same way.
function updateRow() {
  const u = L.update || (S.update && S.update.available ? { state: 'new', latest: S.update.available } : null);
  const link = `color:${RB.ACC}; text-decoration:none`;
  const status = !u ? ''
    : u.state === 'checking' ? 'Checking…'
    : u.state === 'ok' ? `You're up to date (v${esc(u.current)})`
    : u.state === 'new' ? `<span style="color:${RB.ACC}; font-weight:600">v${esc(u.latest)} is available</span> — <a href="#" data-act="openUpdate" style="${link}">Download</a>`
    : `Couldn't check: ${esc(u.error)}`;
  const brew = u && u.state === 'new' && S.update && S.update.homebrew
    ? `<span class="hint">Update with: <code style="user-select:text">brew upgrade --cask rattleboard</code></span>` : '';
  return `<div style="display:flex; flex-direction:column; gap:4px">
      <div style="display:flex; align-items:center; gap:10px"><button class="btn" data-act="checkUpdates" ${u && u.state === 'checking' ? 'disabled' : ''}>Check for updates</button><span class="hint" data-update-status>${status}</span></div>
      ${brew}
    </div>`;
}

function checkUpdates() {
  L.update = { state: 'checking' };
  render(true);
  RB.invoke('check_for_updates')
    .then(r => { L.update = r.newer ? { state: 'new', latest: r.latest } : { state: 'ok', current: r.current }; })
    .catch(e => { L.update = { state: 'err', error: String(e) }; })
    .finally(() => render(true));
}

function viewSettings() {
  const s = S.settings;
  let body = '';
  if (L.tab === 'display') {
    body = `<div class="grid">
      <div class="lbl">Name</div>${nameField()}
      ${boardSizeRows()}
      <div class="lbl">Finish</div>${seg([['black', 'Black'], ['white', 'White']], s.finish, 'finish')}
      ${placementRows()}
      ${sizeRow()}
    </div>`;
  } else if (L.tab === 'behavior') {
    body = `<div class="grid">
      ${s.mode === 'float' ? `<div></div><div class="hint">The board is pinned as a widget, so it doesn't hide. Pop-up timing applies when you unpin it.</div>`
        : s.mode === 'fullscreen' ? `<div></div><div class="hint">The board is full screen, so it doesn't hide. Pop-up timing applies when you leave full screen.</div>` : ''}
      <div class="lbl">Hide after</div>
      <div style="display:flex; align-items:center; gap:10px">${range(3, 60, 1, s.hideSec, 'hideSec')}<span id="hideLabel" style="font-variant-numeric:tabular-nums; min-width:44px">${s.hideSec} s</span></div>
      <div class="lbl">Pause on hover</div>
      <div style="display:flex; align-items:center; gap:10px">${tog(s.pauseOnHover, 'pauseOnHover')}<span class="hint">Countdown waits while the pointer is over the board</span></div>
      <div class="lbl">Flip speed</div>
      <div style="display:flex; align-items:center; gap:10px">${range(0.5, 2, 0.1, s.speed, 'speed')}<span id="speedLabel" style="font-variant-numeric:tabular-nums">${speedLabel(s.speed)}</span></div>
      <div class="lbl">Flap sound</div>
      <div style="display:flex; align-items:center; gap:10px">${tog(s.sound, 'sound')}<span class="hint">Mechanical clatter while flaps turn</span></div>
      <div class="lbl">Start at login</div>
      <div style="display:flex; align-items:center; gap:10px">${tog(s.launchLogin, 'launchLogin')}<span class="hint">Start Rattleboard when I log in</span></div>
      <div class="lbl">Updates</div>
      <div style="display:flex; align-items:center; gap:10px">${tog(s.updateCheck, 'updateCheck')}<span class="hint">Check for updates automatically (once a day, on GitHub). Rattleboard never installs anything itself.</span></div>
      <div></div>${updateRow()}
      <div class="lbl">Full screen at launch</div>
      <div style="display:flex; align-items:center; gap:10px">${tog(s.fullscreenLaunch, 'fullscreenLaunch')}<span class="hint">Open in full screen at launch, for a spare computer used as a board. Otherwise full screen lasts until Rattleboard quits.</span></div>
      ${S.hasTray === false ? `<div class="lbl">No tray</div>
      <div style="display:flex; align-items:center; gap:10px"><button class="btn" data-act="quitApp">Quit Rattleboard</button><span class="hint">This desktop has no tray icon area, so Rattleboard keeps running after this window closes. Open Rattleboard again to come back here.</span></div>` : ''}
    </div>`;
  } else if (L.tab === 'api') {
    body = viewApi();
  } else if (L.tab === 'boards') {
    body = viewBoards();
  } else if (L.tab === 'sources') {
    body = viewSources();
  } else if (L.tab === 'notify') {
    body = viewNotify();
  } else {
    const local = s.source === 'local', demo = s.source === 'demo' || s.source === 'virtual', virt = s.source === 'virtual', key = local ? L.secrets.localKey : L.secrets.cloudToken;
    const keyType = L.showKey ? 'text' : 'password', keyBtn = L.showKey ? 'Hide' : 'Show';
    const connDot = !s.polling ? '#9a9ca1' : S.conn === 'testing' ? '#e0a526' : S.conn === 'error' ? '#e0553f' : S.conn === 'ok' ? '#34b05a' : '#9a9ca1';
    const connText = S.conn === 'testing' ? 'Testing…' : !s.polling ? 'Paused' : S.conn === 'error' ? (S.error || "Can't reach the board") : S.conn === 'ok' ? `Connected · checked ${checkedLabel()}` : 'Not checked yet';
    const url = local ? (/:\d+$/.test(s.host.trim()) ? `http://${s.host.trim()}/local-api/message` : `http://${s.host.trim() || '[BOARD_IP]'}:7000/local-api/message`) : '';
    const lines = virt
      ? [[`No mirroring: the board shows what apps send to ${S.server.urls[0] || 'the API server'}`, '#ecebe6'], ['→ see the API server tab for the key and examples', '#9fd3a8']]
      : demo
      ? [['No network requests: sample messages from the built-in demo', '#ecebe6'], [`→ a new message every 20 s   ${S.rows} × ${S.cols} flap codes`, '#9fd3a8']]
      : local
      ? [[`GET ${url}`, '#ecebe6'], [`X-Vestaboard-Local-Api-Key: ${RB.mask(key)}`, '#a9c4f5'], [`→ [[0,0,1,24,9,…], …]   ${S.rows} × ${S.cols} flap codes`, '#9fd3a8']]
      : [['GET https://cloud.vestaboard.com/', '#ecebe6'], [`X-Vestaboard-Token: ${RB.mask(key)}`, '#a9c4f5'], ['→ { "id": "…", "layout": [[0,0,0,1,2,3,…], …] }', '#9fd3a8']];
    const note = virt ? 'Rattleboard acts as the board itself. Apps write to it with the Vestaboard Local, Read/Write or Cloud API format.' : demo ? 'Switch to Local network or Cloud when you have a board.' : local ? 'A change is detected when the returned layout differs from the previous check.' : 'A change is detected when the message id differs from the previous check.';
    body = `<div class="grid" style="gap:14px 16px">
      <div class="lbl">Read from</div>${seg([['local', 'Local network'], ['cloud', 'Cloud'], ['demo', 'Demo'], ['virtual', 'Virtual board']], s.source, 'source')}
      ${demo ? '' : local ? `
        <div class="lbl">Address</div><input class="field" style="justify-self:start; width:240px" value="${esc(s.host)}" data-field="host">
        <div></div><div class="hint" style="margin-top:-8px">${HOST_HINT}</div>
        <div class="lbl">Key</div>
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
    </div>`;
  }
  return `<div class="hdr" style="display:flex; justify-content:center; padding:16px 24px 0">${seg(tabs().map(t => [t, TAB_NAMES[t]]), L.tab, 'tab', 'tabs')}</div>
    <div class="scroll"><div style="padding:18px 24px 22px">${body}</div></div>
    <div class="hdr hint" style="display:flex; flex-wrap:wrap; gap:6px; padding:10px 24px 12px; border-top:1px solid rgba(0,0,0,.06); font-size:11.5px">
      <span>Rattleboard v${esc(S.version)}</span>·<a href="#" data-act="site" style="color:${RB.ACC}; text-decoration:none">iokig.com</a>·<a href="#" data-act="kofi" style="color:${RB.ACC}; text-decoration:none">Support on Ko-fi</a>·<span>Not affiliated with or endorsed by Vestaboard, Inc.</span></div>`;
}
const TAB_NAMES = { display: 'Display', behavior: 'Behavior', connection: 'Connection', api: 'API server', boards: 'Boards', sources: 'Sources', notify: 'Notifications' };
// Notification forwarding exists only where the OS lets apps read notifications (Windows).
const tabs = () => ['display', 'behavior', 'connection', 'api', 'boards', 'sources', ...(S && S.notify.supported ? ['notify'] : [])];
function viewApi() {
  const s = S.settings, sv = S.server, key = L.secrets.serverKey;
  const base = sv.urls[0] || `http://127.0.0.1:${s.serverPort}`;
  const shownKey = L.showServerKey ? key : RB.mask(key);
  const dot = !s.serverEnabled ? '#9a9ca1' : sv.running ? '#34b05a' : '#e0553f';
  const status = !s.serverEnabled ? 'Off' : sv.running ? `Listening on ${sv.urls.join('  ·  ')}` : (sv.error || 'Starting…');
  const ex = [
    ['# Write text (word-wrapped and centred, like the Vestaboard cloud)', '#8e9096'],
    [`curl -X POST ${base}/local-api/message \\`, '#ecebe6'],
    [`  -H "X-Vestaboard-Local-Api-Key: ${shownKey}" \\`, '#a9c4f5'],
    [`  -d '{"text": "Hello from my app"}'`, '#ecebe6'],
    ['# Write exact flaps: rows of character codes, any size up to 12 × 40', '#8e9096'],
    [`curl -X POST ${base}/local-api/message -H "X-Vestaboard-Local-Api-Key: …" -d '[[0,8,5,12,12,15,…],…]'`, '#ecebe6'],
    ['# Read what the board shows', '#8e9096'],
    [`curl ${base}/local-api/message -H "X-Vestaboard-Local-Api-Key: …"`, '#ecebe6'],
    ['# Read/Write and Cloud API clients: point them at the base URL;', '#8e9096'],
    ['# X-Vestaboard-Read-Write-Key and X-Vestaboard-Token take the same key', '#8e9096']
  ];
  const curl = `curl -X POST ${base}/local-api/message -H "X-Vestaboard-Local-Api-Key: ${key}" -d '{"text": "Hello from my app"}'`;
  return `<div class="grid" style="gap:14px 16px">
    <div class="lbl">API server</div>
    <div style="display:flex; align-items:center; gap:10px">${tog(s.serverEnabled, 'serverEnabled')}<span class="hint">Let other apps write to this board, using the same API format as a real board</span></div>
    <div class="lbl">Port</div><input class="field" style="justify-self:start; width:90px" value="${s.serverPort}" data-field="serverPort" inputmode="numeric">
    <div class="lbl">Other devices</div>
    <div style="display:flex; align-items:center; gap:10px">${tog(s.serverLan, 'serverLan')}<span class="hint">Accept requests from phones and computers on your network, not only this one</span></div>
    <div class="lbl">API key</div>
    <div style="display:flex; gap:6px; min-width:0">
      <input class="field mono" style="width:240px; font-size:12px" readonly value="${esc(L.showServerKey ? key : (key ? RB.mask(key) : 'Created when you turn the server on'))}">
      <button class="btn" data-act="showServerKey" ${key ? '' : 'disabled'}>${L.showServerKey ? 'Hide' : 'Show'}</button>
      <button class="btn" data-act="copyKey" ${key ? '' : 'disabled'}>${L.copied === 'key' ? 'Copied' : 'Copy'}</button>
      <button class="btn" data-act="newKey" title="Apps using the old key stop working">New key</button>
    </div>
    <div class="lbl">Status</div>
    <div style="display:flex; align-items:center; gap:10px; min-width:0">
      <div class="dot" style="background:${dot}"></div><span style="white-space:nowrap; overflow:hidden; text-overflow:ellipsis; user-select:text">${esc(status)}</span>
      <button class="btn" style="margin-left:auto; flex:none" data-act="copyCurl" data-curl="${esc(curl)}" ${sv.running && key ? '' : 'disabled'}>${L.copied === 'curl' ? 'Copied' : 'Copy example'}</button>
    </div>
    ${pairRows()}
    <div class="mono" style="grid-column:1 / -1; background:#1d1e21; border-radius:8px; padding:12px 14px; font-size:11px; line-height:1.65; display:flex; flex-direction:column; overflow:auto; user-select:text">
      ${ex.map(([t, c]) => `<div style="color:${c}; white-space:pre">${esc(t)}</div>`).join('')}
    </div>
    <div style="grid-column:1 / -1" class="hint">${s.source === 'virtual'
      ? 'Read from is set to Virtual board, so the board only shows what apps send here.'
      : `Messages sent here show up right away. While mirroring ${s.source === 'demo' ? 'the demo' : 'your board'}, its next change replaces them. To use Rattleboard purely as a virtual board, set Connection → Read from to Virtual board.`}</div>
  </div>`;
}
const speedLabel = v => (+v).toFixed(1) + '×' + (+v === 1 ? ' · true to hardware' : '');

// The pairing QR code: shown only when asked for, and gone again after two minutes.
function pairRows() {
  const s = S.settings;
  if (L.pair) {
    const left = Math.max(0, Math.ceil((L.pair.until - Date.now()) / 1000));
    return `<div class="lbl" style="align-self:start; padding-top:4px">Pair a phone</div>
      <div style="display:flex; gap:16px; align-items:center; background:#fff; border-radius:10px; padding:14px; box-shadow:0 0 0 1px rgba(0,0,0,.1)">
        <div style="width:150px; height:150px; flex:none; display:flex" class="qr">${L.pair.svg}</div>
        <div style="display:flex; flex-direction:column; gap:8px; min-width:0">
          <div style="font-weight:600">Scan with the Rattleboard app</div>
          <div class="hint" style="line-height:1.45; text-wrap:pretty">Or with the phone's camera. The phone adds this computer’s board with its address and key. The code includes the key, so only show it to devices you trust.</div>
          <div style="display:flex; align-items:center; gap:8px">
            <button class="btn" data-act="copyPair">${L.copied === 'pair' ? 'Copied' : 'Copy link'}</button>
            <button class="btn" data-act="hidePair">Hide</button>
            <span class="hint" id="pairLeft" style="font-variant-numeric:tabular-nums">Hides in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}</span>
          </div>
        </div>
      </div>`;
  }
  const ask = L.pairAsk && !s.serverLan;
  return `<div class="lbl">Pair a phone</div>
    <div style="display:flex; align-items:center; gap:10px; min-width:0">
      ${ask ? `<span class="hint" style="flex:1; text-wrap:pretty">Pairing needs <b style="font-weight:600; color:#3b3c40">Other devices</b> on, so the phone can reach this computer.</span>
        <button class="btn" style="flex:none" data-act="pairLan">Turn on</button><button class="btn" style="flex:none" data-act="pairCancel">Cancel</button>`
      : `<button class="btn" style="flex:none" data-act="pair" ${s.serverEnabled ? '' : 'disabled'}>Show pairing code</button><span class="hint">${L.pairError ? `<span style="color:#c0392b">${esc(L.pairError)}</span>` : 'A QR code the Rattleboard phone app scans to add this board'}</span>`}
    </div>`;
}
function showPair() {
  L.pairError = '';
  RB.invoke('pair_qr')
    .then(q => { L.pair = { ...q, until: Date.now() + PAIR_MS }; L.pairAsk = false; })
    .catch(e => { L.pairError = String(e); })
    .finally(() => render(true));
}
setInterval(() => {
  if (!L.pair) return;
  const left = Math.max(0, Math.ceil((L.pair.until - Date.now()) / 1000));
  if (!left) { L.pair = null; return render(true); }
  const el = document.getElementById('pairLeft');
  if (el) el.textContent = `Hides in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
}, 1000);

// ---------------------------------------------------------------- boards

const KIND = {
  rattleboard: ['Rattleboard', '#34b05a'],
  local: ['Local API', RB.ACC],
  cloud: ['Vestaboard · Cloud', '#8a63c9']
};
const sizeName = RB.sizeName;
const HOST_HINT = 'Rattleboard: its address, e.g. 192.168.1.20:7070. Vestaboard: its IP address (port 7000) and Local API key.';
const formField = (f, ph, type = 'text', w = '100%') => `<input class="field" style="width:${w}; max-width:340px" type="${type}" value="${esc(L.form[f])}" data-form="${f}" placeholder="${esc(ph)}" autocomplete="off" spellcheck="false">`;

function viewBoards() {
  const list = S.boards.map(b => {
    const [kind, color] = KIND[b.kind] || KIND.local;
    const where = b.kind === 'cloud' ? 'cloud.vestaboard.com' : b.host;
    const name = L.renaming === b.id
      ? `<input class="field" style="width:200px" value="${esc(b.name)}" data-rename="${esc(b.id)}">`
      : `<span style="font-weight:600; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(b.name)}</span>`;
    return `<div style="display:flex; align-items:center; gap:12px; padding:10px 14px; border-top:1px solid rgba(0,0,0,.06)">
      <div class="dot" style="background:${color}"></div>
      <div style="display:flex; flex-direction:column; gap:2px; min-width:0; flex:1">${name}
        <span style="font-size:11.5px; color:#6b6d72; line-height:1.35">${kind} · ${esc(where)} · ${sizeName(b.rows, b.cols)}</span></div>
      ${L.renaming === b.id ? `<button class="btn" data-act="renameDone" data-v="${esc(b.id)}">Done</button>`
        : L.removing === b.id ? `<span class="hint">Remove it and its key?</span><button class="btn" style="color:#c0392b" data-act="removeYes" data-v="${esc(b.id)}">Remove</button><button class="btn" data-act="removeNo">Cancel</button>`
        : `<button class="btn" data-act="rename" data-v="${esc(b.id)}">Rename</button><button class="btn" data-act="removeBoard" data-v="${esc(b.id)}">Remove</button>`}
    </div>`;
  }).join('');
  const msg = L.boardMsg ? `<div style="grid-column:2; margin-top:-6px; font-size:12px; color:${L.boardMsg.ok ? '#2f8a4c' : '#c0392b'}">${esc(L.boardMsg.text)}</div>` : '';
  const addBtn = (act, label = 'Add board') => `<button class="btn" style="justify-self:start" data-act="${act}" ${L.adding ? 'disabled' : ''}>${L.adding ? 'Checking…' : label}</button>`;
  let form = '';
  if (L.addMode === 'nearby') {
    const f = L.found2;
    const rows = (f || []).map((d, i) => {
      const saved = S.boards.find(b => b.host === d.host);
      return `<div style="display:flex; align-items:center; gap:12px; padding:9px 14px; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}">
        <div style="display:flex; flex-direction:column; gap:2px; flex:1; min-width:0"><span style="font-weight:600">${esc(d.name)}</span><span style="font-size:11.5px; color:#6b6d72">${esc(d.host)}</span></div>
        ${saved ? `<span class="hint">Saved as ${esc(saved.name)}</span>` : `<button class="btn" data-act="addFound" data-v="${i}">Add…</button>`}</div>`;
    }).join('');
    const note = f == null ? 'Searching your network…' : !f.length ? 'No Rattleboards answered. Make sure the other one has its API server on with Other devices allowed.' : '';
    form = `<div style="grid-column:2; display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1)">
        ${note ? `<div style="padding:12px 14px; color:#6b6d72">${note}</div>` : ''}${rows}</div>
      <div></div><div style="display:flex; align-items:center; gap:10px"><button class="btn" style="flex:none" data-act="scanAgain" ${f == null ? 'disabled' : ''}>Search again</button><span class="hint">Rattleboards on TVs and other computers announce themselves on your network</span></div>`;
  } else if (L.addMode === 'manual') {
    form = `<div class="lbl">Address</div>${formField('host', '192.168.1.20 or tv.local:7070', 'text', '240px')}
      <div class="lbl">Key</div>${formField('key', "Its API key, or a Vestaboard's Local API key", 'password', '240px')}
      <div class="lbl">Name</div>${formField('name', 'Optional', 'text', '240px')}
      <div></div><div style="display:flex; align-items:center; gap:10px">${addBtn('addManual')}<span class="hint">${HOST_HINT}</span></div>`;
  } else if (L.addMode === 'link') {
    const c = L.pairConfirm;
    form = `<div class="lbl">Pair link</div>${formField('link', 'rattleboard://pair?v=2&url=…')}
      <div></div>${c
        // Like the phone apps, ask before saving a board from a link.
        ? `<div style="display:flex; align-items:center; gap:10px; flex-wrap:wrap"><span>Add <b style="font-weight:600">${esc(c.name)}</b> at ${esc(c.host)} (${sizeName(c.rows, c.cols)})?</span>${addBtn('addLinkYes', 'Add board')}<button class="btn" data-act="addLinkNo">Cancel</button></div>`
        : `<div style="display:flex; align-items:center; gap:10px">${addBtn('addLink', 'Check link')}<span class="hint">On the other Rattleboard: Settings → API server → Show pairing code → Copy link.</span></div>`}`;
  } else {
    form = `<div class="lbl">Cloud API token</div>${formField('token', 'Paste your token', 'password', '240px')}
      <div class="lbl">Name</div>${formField('name', 'Optional', 'text', '240px')}
      <div></div><div style="display:flex; align-items:center; gap:10px">${addBtn('addCloud')}<span class="hint">Cloud boards take a new message at most every 15 s.</span></div>`;
  }
  return `<div class="grid" style="gap:14px 16px">
    <div class="lbl" style="align-self:start; padding-top:10px">Boards</div>
    <div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); overflow:hidden">
      <div style="display:flex; align-items:center; gap:12px; padding:10px 14px">
        <div class="dot" style="background:#9a9ca1"></div>
        <div style="display:flex; flex-direction:column; gap:2px; flex:1"><span style="font-weight:600">This board</span><span style="font-size:11.5px; color:#6b6d72">${esc(S.settings.boardName || 'Rattleboard')} on this computer · ${S.settings.adaptive ? 'matches each message' : sizeName(S.rows, S.cols)}</span></div>
        <button class="btn" data-act="compose">Compose…</button>
      </div>
      ${list}
    </div>
    ${S.boards.length ? '' : `<div></div><div class="hint" style="margin-top:-6px">Add a TV or computer running Rattleboard, or a real Vestaboard, to send messages and sequences to it from Compose.</div>`}
    ${groupRows()}
    <div class="lbl">Add a board</div>${seg([['nearby', 'Nearby'], ['manual', 'By address'], ['link', 'Pair link'], ['cloud', 'Cloud']], L.addMode, 'addMode')}
    ${form}
    ${msg}
    <div style="grid-column:1 / -1" class="hint">Keys are kept in ${RB.keychain(S)}, one entry per board, and never written to a file.</div>
  </div>`;
}

// Named sets of boards to send to at once. A board can be in any number of them.
function groupRows() {
  const groups = S.groups || [], members = [['this', 'This board'], ...S.boards.map(b => [b.id, b.name])];
  const rows = groups.map((g, i) => {
    const name = L.renamingGroup === g.id
      ? `<input class="field" style="width:180px" value="${esc(g.name)}" data-group-name="${esc(g.id)}">`
      : `<span style="font-weight:600">${esc(g.name)}</span><span class="hint">${g.members.length} board${g.members.length === 1 ? '' : 's'}</span>`;
    const chips = members.map(([id, label]) => `<button class="chip ${g.members.includes(id) ? 'on' : ''}" data-act="groupMember" data-g="${esc(g.id)}" data-v="${esc(id)}">${esc(label)}</button>`).join('');
    return `<div style="display:flex; flex-direction:column; gap:8px; padding:10px 14px; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}">
      <div style="display:flex; align-items:center; gap:8px">${name}<div style="flex:1"></div>
        ${L.renamingGroup === g.id ? `<button class="btn" data-act="groupRenameDone">Done</button>` : `<button class="btn" data-act="groupRename" data-v="${esc(g.id)}">Rename</button><button class="btn" data-act="groupDelete" data-v="${esc(g.id)}">Delete</button>`}</div>
      <div style="display:flex; flex-wrap:wrap; gap:6px">${chips}</div>
    </div>`;
  }).join('');
  return `<div class="lbl" style="align-self:start; padding-top:${groups.length ? 10 : 4}px">Groups</div>
    <div style="display:flex; flex-direction:column; gap:8px; align-items:flex-start">
      ${groups.length ? `<div style="align-self:stretch; display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1)">${rows}</div>` : ''}
      <div style="display:flex; align-items:center; gap:10px"><button class="btn" data-act="groupNew" ${S.boards.length ? '' : 'disabled'}>New group</button><span class="hint">${groups.length ? 'Tick the boards in each group. Compose, Message History and notifications can send to a group or to all boards at once.' : 'Send to several boards at once, like “Downstairs” or “Office”.'}</span></div>
    </div>`;
}
const saveGroups = groups => RB.invoke('set_groups', { groups });

function scanNearby() {
  L.found2 = null; render(true);
  RB.invoke('discover_boards').then(f => { L.found2 = f; render(true); });
}
// Runs one of the add_* commands, then clears the form (and the key with it).
function addBoard(cmd, args) {
  L.adding = true; L.boardMsg = null; render(true);
  RB.invoke(cmd, args)
    .then(b => {
      L.boardMsg = { ok: true, text: `Added ${b.name}: ${(KIND[b.kind] || KIND.local)[0]}, ${sizeName(b.rows, b.cols)}` };
      L.form = { host: '', key: '', name: '', link: '', token: '' };
    })
    .catch(e => { L.boardMsg = { ok: false, text: String(e) }; })
    .finally(() => { L.adding = false; render(true); });
}

// ---------------------------------------------------------------- notifications (Windows)

function viewNotify() {
  const s = S.settings, n = S.notify;
  const apps = [...s.notifyApps, ...n.apps.filter(a => !s.notifyApps.some(x => x.id === a.id))];
  const on = id => s.notifyApps.some(a => a.id === id);
  const access = n.access === 'allowed' ? ['#34b05a', 'Allowed']
    : n.access === 'denied' ? ['#e0553f', 'Turned off in Windows Settings → Privacy & security → Notifications']
    : n.access === 'unavailable' ? ['#9a9ca1', "Windows didn't let Rattleboard read notifications on this PC"]
    : ['#e0a526', 'Not allowed yet'];
  const back = [[0, 'Never'], [10, '10 s'], [30, '30 s'], [60, '1 min'], [300, '5 min']];
  return `<div class="grid" style="gap:14px 16px">
    <div class="lbl">Forward</div>
    <div style="display:flex; align-items:center; gap:10px">${tog(s.notifyEnabled, 'notifyEnabled')}<span class="hint">Put notifications from the apps below on a board</span></div>
    <div class="lbl">Access</div>
    <div style="display:flex; align-items:center; gap:10px; min-width:0"><div class="dot" style="background:${access[0]}"></div><span style="text-wrap:pretty">${esc(access[1])}</span>
      ${n.access === 'allowed' ? '' : `<button class="btn" style="margin-left:auto; flex:none" data-act="notifyAccess">Allow access…</button>`}</div>
    ${L.notifyError ? `<div></div><div style="margin-top:-6px; font-size:12px; color:#c0392b">${esc(L.notifyError)}</div>` : ''}
    <div class="lbl" style="align-self:start; padding-top:10px">Apps</div>
    <div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); max-height:180px; overflow:auto">
      ${apps.length ? apps.map((a, i) => `<label style="display:flex; align-items:center; gap:10px; padding:8px 14px; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}; cursor:pointer">
        <input type="checkbox" ${on(a.id) ? 'checked' : ''} data-notify-app="${esc(a.id)}" data-name="${esc(a.name)}" style="accent-color:${RB.ACC}; margin:0">
        <span>${esc(a.name || a.id)}</span></label>`).join('')
        : `<div style="padding:12px 14px; color:#6b6d72">Apps appear here once they've sent a notification.</div>`}
    </div>
    <div class="lbl">Show on</div>
    <select class="field" style="justify-self:start; width:240px" data-field="notifyTarget">${RB.targetOptions(S, s.notifyTarget)}</select>
    <div class="lbl">Hide content</div>
    <div style="display:flex; align-items:center; gap:10px">${tog(s.notifyHideContent, 'notifyHideContent')}<span class="hint">Show only which app it's from, not what it says</span></div>
    <div class="lbl">Quiet hours</div>
    <div style="display:flex; align-items:center; gap:8px">${tog(s.notifyQuiet, 'notifyQuiet')}
      <input class="field" type="time" style="width:110px" value="${esc(s.notifyQuietFrom)}" data-field="notifyQuietFrom" ${s.notifyQuiet ? '' : 'disabled'}>
      <span class="hint">to</span>
      <input class="field" type="time" style="width:110px" value="${esc(s.notifyQuietTo)}" data-field="notifyQuietTo" ${s.notifyQuiet ? '' : 'disabled'}></div>
    <div class="lbl">Go back after</div>
    <div style="display:flex; flex-direction:column; gap:6px; align-items:flex-start">${seg(back, s.notifyReturnSec, 'notifyReturn', 'poll')}<span class="hint">Puts the previous message back, if the board still shows the notification</span></div>
  </div>`;
}

// ---------------------------------------------------------------- history

function viewHistory() {
  const s = S.settings, d = RB.dims(S), fin = RB.fin(s.finish);
  const cw = Math.max(3, Math.min(7, Math.floor(112 / d.cols))), chh = Math.round(cw * 1.4), now = new Date();
  const shown = JSON.stringify(RB.fit(S.display, d));
  const y = new Date(now); y.setDate(now.getDate() - 1);
  const day = t => { const d = new Date(t).toDateString(); return d === now.toDateString() ? 'Today' : d === y.toDateString() ? 'Yesterday' : new Date(t).toLocaleDateString('en-US', { month: 'short', day: 'numeric' }); };
  const rows = S.history.map((h, i) => {
    const g = h.layout, onNow = i === 0 && JSON.stringify(RB.fit(g, d)) === shown;
    return `<div style="display:grid; grid-template-columns:auto minmax(0,1fr) auto; gap:14px; align-items:center; padding:12px 18px; border-bottom:1px solid rgba(0,0,0,.06); background:${onNow ? 'rgba(0,0,0,.03)' : 'transparent'}">
      ${thumb(g, cw)}
      <div style="display:flex; flex-direction:column; gap:4px; min-width:0">
        <div class="flap-font" style="font-size:14px; letter-spacing:.04em; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(RB.decode(g) || '(blank)')}</div>
        <div style="font-size:11.5px; color:#6b6d72; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${day(h.at)} ${RB.fmtTime(h.at)} · ${esc(h.src)}${onNow ? ' · on board now' : ''}${centredNote(g)}</div>
      </div>
      <div style="display:flex; gap:4px; align-items:center"><button class="btn" data-act="flip" data-v="${h.id}">Flip again</button>${sendActions('h' + h.id, g)}</div>
    </div>`;
  }).join('');
  const n = S.history.length;
  return `<div class="hdr" style="display:flex; align-items:center; justify-content:space-between; padding:12px 18px; border-bottom:1px solid rgba(0,0,0,.08)">
      <span style="color:#55575c">${n ? `${n} change${n > 1 ? 's' : ''} detected` : 'No changes yet'}</span>
      <button class="btn" style="padding:3px 10px" data-act="clear">Clear</button>
    </div>
    <div class="scroll"><div style="display:flex; flex-direction:column">
      ${rows || `<div style="padding:40px 18px; text-align:center; color:#6b6d72">No changes yet. New board messages appear here as they're detected.</div>`}
    </div></div>`;
}

// ---------------------------------------------------------------- resending

// A small picture of a grid, at its own size.
function thumb(g, cw) {
  const fin = RB.fin(S.settings.finish), w = cw || Math.max(3, Math.min(7, Math.floor(112 / ((g[0] || []).length || 22)))), h = Math.round(w * 1.4);
  return `<div style="background:${fin.frame}; padding:5px; border-radius:4px; display:flex; flex-direction:column; gap:1px; flex:none">${g.map(r => `<div style="display:flex; gap:1px">${r.map(c => `<div style="width:${w}px; height:${h}px; border-radius:.5px; background:${c >= 63 ? (c === 71 ? fin.ink : RB.HEX[c]) : c ? fin.dim : fin.tile}"></div>`).join('')}</div>`).join('')}</div>`;
}
const sendTargets = () => RB.targets(S);
const sendTarget = () => { const t = sendTargets().find(t => t[0] === S.settings.sendTarget) || sendTargets()[0]; return [t[0], RB.targetName(S, t[0])]; };
// Fixed-size boards centre a grid made for another size; say so before it's sent.
function centredNote(g, target = sendTarget()[0]) {
  const r = g.length, c = (g[0] || []).length;
  const off = RB.destinations(S, target).filter(d => !d.adaptive && (d.rows !== r || d.cols !== c)).length;
  return !off ? '' : RB.destinations(S, target).length > 1 ? ` · made for ${r} × ${c}, centred on ${off} board${off > 1 ? 's' : ''}` : ` · made for ${r} × ${c}, will be centred`;
}
const resendGrids = {};
// "Send to <last board>", a menu of the others, and Edit in Compose. `key` names the grid.
function sendActions(key, g) {
  resendGrids[key] = g;
  const [id, name] = sendTarget(), sent = L.resent && L.resent.key === key;
  const pick = S.boards.length ? `<select class="field" style="width:28px; padding:0 2px" data-send-pick="${esc(key)}" title="Send to another board or a group"><option value=""></option>${RB.targetOptions(S, null)}</select>` : '';
  const send = S.boards.length ? `<button class="btn" data-act="resend" data-v="${esc(key)}" data-t="${esc(id)}" title="${esc(sent && L.resent.error ? L.resent.error : centredNote(g, id).replace(' · ', '') || 'Send this exact layout')}">${sent ? esc(L.resent.text) : `Send to ${esc(name)}`}</button>` : '';
  return `${send}${pick}<button class="btn" data-act="editGrid" data-v="${esc(key)}" title="Open this layout in Compose">Edit</button>`;
}
function resend(key, target) {
  const name = RB.targetName(S, target);
  RB.invoke('send_message', { target, message: { characters: resendGrids[key] } })
    .then(res => {
      const bad = res.filter(d => d.status !== 'sent');
      L.resent = !bad.length ? { key, text: 'Sent' }
        : bad.length < res.length ? { key, text: `Sent to ${res.length - bad.length} of ${res.length}`, error: bad.map(d => `${d.name}: ${d.error}`).join('\n') }
        : { key, text: 'Failed', error: bad.map(d => `${d.name}: ${d.error}`).join('\n') };
    })
    .catch(e => { L.resent = { key, text: 'Failed', error: `${name}: ${e}` }; })
    .finally(() => { render(true); setTimeout(() => { if (L.resent && L.resent.key === key) { L.resent = null; render(true); } }, 1600); });
}

// ---------------------------------------------------------------- setup

// The steps, by flow: a standalone board (Read from: Virtual board) asks what it should do and
// then helps get that going; mirroring another board, the cloud or the demo keep their own.
const FEEDS = [['bbc', 'BBC World', 'https://feeds.bbci.co.uk/news/world/rss.xml'], ['hn', 'Hacker News', 'https://hnrss.org/frontpage'], ['', 'Other feed', '']];
const CD_PRESETS = [['nye', "New Year's Eve"], ['christmas', 'Christmas'], ['halloween', 'Halloween'], ['', 'My own date']];
const newSetupChoices = () => ({
  sources: true, messages: true,
  clock: true, quote: true, weather: false, city: '', countdown: false, cdPreset: 'nye', cdTitle: '', cdDate: '',
  news: false, feed: 'bbc', feedUrl: ''
});
function setupSteps() {
  if (L.setupSource !== 'virtual') return [['use', 'Connection'], ['find', 'Find board'], ['popup', 'Popup'], ['done', 'Done']];
  const u = L.su;
  return [['use', 'Use'], ['what', 'Set up'], ...(u.sources || u.messages ? [['start', 'Get started']] : []), ['popup', 'Popup'], ['done', 'Done']];
}
const setupKey = () => (setupSteps()[L.step] || ['use'])[0];
const checkRow = (on, act, title, text, v = '') => `<div data-act="${act}" data-v="${v}" style="display:flex; gap:10px; align-items:flex-start; cursor:pointer">
    <div style="width:16px; height:16px; margin-top:1px; border-radius:4px; flex:none; display:flex; align-items:center; justify-content:center; color:#fff; font-size:11px; font-weight:700; background:${on ? RB.ACC : '#fff'}; box-shadow:${on ? 'none' : 'inset 0 0 0 1.5px rgba(0,0,0,.3)'}">${on ? '✓' : ''}</div>
    <div style="display:flex; flex-direction:column; gap:2px"><span style="font-weight:600">${title}</span>${text ? `<span class="hint" style="line-height:1.45; text-wrap:pretty">${text}</span>` : ''}</div></div>`;

// Step 3 of a standalone board: starter sources, and how to send it a message.
function setupStart() {
  const u = L.su, s = S.settings, sv = S.server;
  let out = '';
  if (u.sources) {
    const chips = (list, cur, act) => `<div style="display:flex; flex-wrap:wrap; gap:6px">${list.map(([v, l]) => `<button class="chip ${cur === v ? 'on' : ''}" data-act="${act}" data-v="${v}">${l}</button>`).join('')}</div>`;
    out += `<div style="font-size:17px; font-weight:600">Pick some sources</div>
      <div style="display:flex; flex-direction:column; gap:12px; padding:14px; border-radius:10px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1)">
        ${checkRow(u.clock, 'suPick', 'Clock', 'The time, in big digits', 'clock')}
        ${checkRow(u.quote, 'suPick', 'Quote of the day', 'A public-domain quote each day', 'quote')}
        ${checkRow(u.weather, 'suPick', 'Weather', 'Now and today, from Open-Meteo', 'weather')}
        ${u.weather ? `<div style="margin-left:26px; display:flex; align-items:center; gap:8px"><input class="field" style="width:220px" value="${esc(u.city)}" data-su="city" placeholder="City, e.g. Melbourne"></div>` : ''}
        ${checkRow(u.countdown, 'suPick', 'Countdown', 'Days to a holiday or a date of your own, with a celebration on the day', 'countdown')}
        ${u.countdown ? `<div style="margin-left:26px; display:flex; flex-direction:column; gap:8px">${chips(CD_PRESETS, u.cdPreset, 'suCd')}
          ${u.cdPreset ? '' : `<div style="display:flex; gap:8px; flex-wrap:wrap"><input class="field" style="width:200px" value="${esc(u.cdTitle)}" data-su="cdTitle" placeholder="Name, e.g. Launch day"><input class="field" type="date" style="width:160px" value="${esc(u.cdDate)}" data-su="cdDate"></div>`}</div>` : ''}
        ${checkRow(u.news, 'suPick', 'News feed', 'Headlines from an RSS or Atom feed', 'news')}
        ${u.news ? `<div style="margin-left:26px; display:flex; flex-direction:column; gap:8px">${chips(FEEDS.map(f => [f[0], f[1]]), u.feed, 'suFeed')}
          ${u.feed ? '' : `<input class="field" style="width:100%; max-width:380px" value="${esc(u.feedUrl)}" data-su="feedUrl" placeholder="https://example.com/feed.xml">`}</div>` : ''}
      </div>
      <div class="hint" style="margin-top:-8px">They take turns on the board. You can change these in Settings → Sources.${L.suError ? ` <span style="color:#c0392b">${esc(L.suError)}</span>` : ''}</div>`;
  }
  if (u.messages) {
    const key = L.secrets.serverKey, base = sv.urls[0] || `http://127.0.0.1:${s.serverPort}`;
    const curl = k => `curl -X POST ${base}/local-api/message -H "X-Vestaboard-Local-Api-Key: ${k}" -d '{"text": "Hello"}'`;
    const status = sv.running ? sv.urls.join('  ·  ') : (sv.error || 'Starting…');
    out += `<div style="font-size:17px; font-weight:600">Send it a message</div>
      <div class="grid" style="gap:12px 16px">
        <div class="lbl">Address</div><div class="mono" style="font-size:12px; user-select:text; min-width:0; overflow-wrap:anywhere">${esc(status)}</div>
        <div class="lbl">API key</div>
        <div style="display:flex; gap:6px; min-width:0">
          <input class="field mono" style="width:220px; font-size:12px" readonly value="${esc(L.showServerKey ? key : (key ? RB.mask(key) : 'Created when the server starts'))}">
          <button class="btn" data-act="showServerKey" ${key ? '' : 'disabled'}>${L.showServerKey ? 'Hide' : 'Show'}</button>
          <button class="btn" data-act="copyKey" ${key ? '' : 'disabled'}>${L.copied === 'key' ? 'Copied' : 'Copy'}</button>
        </div>
        <div class="lbl">Try it</div>
        <div style="display:flex; gap:8px; align-items:center; min-width:0">
          <div class="mono" style="flex:1; min-width:0; background:#1d1e21; color:#ecebe6; border-radius:8px; padding:8px 10px; font-size:11px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; user-select:text">${esc(curl(L.showServerKey ? key : RB.mask(key)))}</div>
          <button class="btn" style="flex:none" data-act="copyCurl" data-curl="${esc(curl(key))}" ${sv.running && key ? '' : 'disabled'}>${L.copied === 'curl' ? 'Copied' : 'Copy'}</button>
        </div>
        ${pairRows()}
      </div>
      <div class="hint" style="margin-top:-8px; text-wrap:pretty">Scripts, the phone apps and <a href="#" data-act="suHa" style="color:${RB.ACC}; text-decoration:none">Home Assistant</a> send to it the same way. More examples are in Settings → API server.</div>`;
  }
  return out;
}

// Turns the starter picks into rotation entries, skipping any the rotation already has (so
// running setup again doesn't add a second clock). Uses the same add_source as Settings → Sources.
function setupStarters() {
  const u = L.su, feed = FEEDS.find(f => f[0] === u.feed);
  const want = [
    u.clock && ['rattleboard.clock', {}],
    u.quote && ['rattleboard.quote', {}],
    u.weather && u.city.trim() && ['rattleboard.weather', { city: u.city.trim() }],
    u.countdown && (u.cdPreset ? ['rattleboard.countdown', { preset: u.cdPreset }]
      : u.cdTitle.trim() && u.cdDate && ['rattleboard.countdown', { preset: '', title: u.cdTitle.trim(), date: u.cdDate, yearly: false }]),
    u.news && (feed && feed[2] || u.feedUrl.trim()) && ['rattleboard.rss', { url: feed && feed[2] ? feed[2] : u.feedUrl.trim() }]
  ].filter(Boolean);
  return RB.invoke('get_sources').then(d => want.reduce((p, [plugin, settings]) => p.then(() => {
    const has = d.rotation.some(x => x.plugin === plugin && Object.entries(settings).every(([k, v]) => String((x.settings || {})[k] ?? '') === String(v)));
    return has ? null : RB.invoke('add_source', { plugin, settings: Object.keys(settings).length ? settings : null });
  }), Promise.resolve()));
}

function viewSetup() {
  const s = S.settings, st = L.step, ACC = RB.ACC, key = setupKey(), virt = L.setupSource === 'virtual', list = setupSteps();
  const steps = list.map(([, l], i) =>
    `<div style="flex:1; display:flex; flex-direction:column; gap:6px"><div style="height:3px; border-radius:2px; background:${i <= st ? ACC : 'rgba(0,0,0,.12)'}"></div><div style="font-size:11.5px; color:${i === st ? '#1b1b1d' : '#8a8c91'}">${l}</div></div>`).join('');
  let body = '';
  if (key === 'use') {
    const card = (v, title, tag, text, needs) => `<div data-act="setupSource" data-v="${v}" style="border-radius:10px; padding:14px; background:#fff; box-shadow:${L.setupSource === v ? `0 0 0 2px ${ACC}` : '0 0 0 1px rgba(0,0,0,.12)'}; cursor:pointer; display:flex; flex-direction:column; gap:6px">
      <div style="display:flex; justify-content:space-between; align-items:center"><span style="font-weight:600">${title}</span><span style="font-size:11px; color:${v === 'virtual' ? '#2f8a4c' : '#6b6d72'}">${tag}</span></div>
      <div style="color:#55575c; font-size:12.5px; line-height:1.45">${text}</div>
      <div class="mono" style="font-size:11px; color:#6b6d72">${needs}</div></div>`;
    body = `<div style="font-size:20px; font-weight:600; letter-spacing:-.01em">How will you use Rattleboard?</div>
      <div style="display:grid; grid-template-columns:1fr 1fr; gap:12px">
        ${card('virtual', 'Standalone board', 'Recommended', 'This computer is the board. Show a rotation of clocks, countdowns, weather, news and quotes, and let your apps and phone send it messages.', 'Needs: nothing')}
        ${card('local', 'Mirror another board', '', 'Show what another Rattleboard on your Wi‑Fi shows: a TV, desktop or phone. Also works with a Vestaboard’s Local API.', 'Needs: its address and key')}
        ${card('cloud', 'Cloud', '', 'Mirror a board through the Vestaboard Cloud API. Works from anywhere, including away from home.', 'Needs: Cloud API token')}
        ${card('demo', 'Demo', 'No setup', 'Flips through sample messages every 20 seconds so you can try Rattleboard first.', 'Needs: nothing')}
      </div>`;
  } else if (key === 'what') {
    const u = L.su;
    body = `<div style="font-size:17px; font-weight:600">What should it do?</div>
      <div style="display:flex; flex-direction:column; gap:14px">
        ${checkRow(u.sources, 'suSources', 'Show sources', 'Rotate clocks, countdowns, weather, news and quotes.')}
        ${checkRow(u.messages, 'suMessages', 'Accept messages', 'Turn on the API server so scripts, Home Assistant and the phone apps can send to it.')}
        ${u.messages ? `<div style="margin-left:26px">${checkRow(s.serverLan, 'serverLan', 'From other devices too', 'Lets phones, tablets, TVs and other computers on your network reach it, not only this computer. Pairing a phone needs this.')}</div>` : ''}
        ${!u.sources && !u.messages ? '<div class="hint">With neither, it shows only what you send from Compose. You can change all of this later in Settings.</div>' : ''}
      </div>
      <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:12px; align-items:center">
        <div class="lbl">Board</div>${modelSeg()}
        <div class="lbl">Name</div>${nameField()}
      </div>`;
  } else if (key === 'start') {
    body = setupStart();
  } else if (key === 'find') {
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
          <div style="display:flex; flex-direction:column; gap:2px"><span style="font-weight:600">${esc(f.name)}</span><span style="font-size:11.5px; color:#6b6d72">${esc(f.ip)}</span></div></div>`;
      }).join('');
      const scanMsg = L.scan === 'scanning' ? 'Searching your network…' : L.scan === 'done' && !L.found.length ? 'Nothing answered. Enter the address of a Rattleboard or a Vestaboard below.' : '';
      body = `<div style="font-size:17px; font-weight:600">Find the board to mirror</div>
        <div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1)">
          ${scanMsg ? `<div style="padding:14px; color:#6b6d72">${scanMsg}</div>` : ''}${found}
        </div>
        <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:12px; align-items:center">
          <div class="lbl">Or enter address</div><input class="field" style="justify-self:start; width:220px" value="${esc(s.host)}" data-field="host">
          <div></div><div class="hint" style="margin-top:-6px">${HOST_HINT}</div>
          <div class="lbl">Key</div><input class="field" style="width:100%; max-width:340px" value="${esc(L.secrets.localKey)}" data-field="localKey" placeholder="Its API key, or the Vestaboard's Local API key">
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
      body += `<div style="display:flex; align-items:center; gap:14px">
        <div style="background:${RB.fin(s.finish).frame}; padding:8px; border-radius:6px; display:flex; flex-direction:column; gap:3px; flex:none">${RB.mini(L.testLayout, S)}</div>
        <div style="color:#55575c; font-size:12.5px; line-height:1.45">Detected a ${sizeName(S.rows, S.cols).replace(' · ', ' board, ')}. This is what it shows right now.</div></div>`;
    }
  } else if (key === 'popup') {
    body = `<div style="font-size:17px; font-weight:600">Where should it appear?</div>
      <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:16px; align-items:center">
        ${placementRows()}
        ${s.mode !== 'toast' ? '' : `<div class="lbl">Hide after</div>
        <div style="display:flex; align-items:center; gap:10px">${range(3, 60, 1, s.hideSec, 'hideSec')}<span id="hideLabel" style="font-variant-numeric:tabular-nums">${s.hideSec} s</span></div>`}
        <div class="lbl">Board</div>${modelSeg()}
      </div>`;
  } else {
    const rot = (L.suRotation || []).filter(x => x.enabled).map(x => x.label || x.name);
    const summary = [
      ['Reading from', s.source === 'local' ? `Local network · ${s.host}` : s.source === 'demo' ? 'Demo · sample messages' : s.source === 'virtual' ? 'Nothing: this is a standalone board' : 'Cloud API'],
      ...(virt ? [
        ['Sources', rot.length ? rot.join(', ') : 'None yet'],
        ['Messages', s.serverEnabled ? (S.server.urls[0] || 'API server on') : 'Off']
      ] : []),
      ...(s.source === 'demo' || s.source === 'virtual' ? [] : [['Checks every', s.pollEvery + ' s']]),
      s.mode === 'float' ? ['Shows as', 'Pinned widget · drag it anywhere, resize from its corner']
        : s.mode === 'fullscreen' ? ['Shows as', `Full screen · ${S.os === 'mac' ? '⌃⌘F' : 'F11'} or Esc to leave`]
        : ['Pops up', `${RB.posNames[s.position]} · hides after ${s.hideSec} s`]
    ];
    body = `<div style="font-size:20px; font-weight:600">You're set</div>
      <div style="display:grid; grid-template-columns:130px minmax(0,1fr); gap:10px 16px">${summary.map(([k, v]) => `<div style="color:#6b6d72">${k}</div><div>${esc(v)}</div>`).join('')}</div>
      <div style="color:#55575c; line-height:1.5">Rattleboard now lives in your ${S.os === 'mac' ? 'menu bar' : 'system tray'}. Click its icon any time for the latest message, history and settings.</div>`;
  }
  const last = st === list.length - 1;
  return `<div class="hdr" style="display:flex; gap:6px; padding:16px 24px 0">${steps}</div>
    <div class="scroll"><div style="padding:20px 24px 8px; display:flex; flex-direction:column; gap:16px">${body}</div></div>
    <div class="hdr" style="display:flex; justify-content:space-between; padding:14px 24px 18px; border-top:1px solid rgba(0,0,0,.06)">
      <button class="btn" style="font-size:13px; padding:5px 14px; opacity:${st === 0 ? 0.4 : 1}" data-act="back">Back</button>
      <button class="primary" data-act="next" ${L.suBusy ? 'disabled' : ''}>${last ? 'Finish' : L.suBusy ? 'Adding…' : 'Continue'}</button>
    </div>`;
}

// Continue: what leaving each step does.
function setupNext() {
  const key = setupKey(), list = setupSteps();
  if (L.step === list.length - 1) return RB.invoke('finish_setup');
  const go = () => { L.step = Math.min(L.step + 1, setupSteps().length - 1); render(true); };
  if (key === 'use') {
    L.test = 'idle'; L.testLayout = null;
    if (L.setupSource === 'virtual') {
      save({ source: 'virtual', pollEvery: 2, serverEnabled: L.su.messages });
    } else {
      save(sourcePatch(L.setupSource));
      if (L.setupSource === 'local' && L.scan === 'idle') {
        L.scan = 'scanning';
        RB.invoke('scan_boards').then(f => {
          L.found = f; L.scan = 'done';
          if (f.length && (!S.settings.host || S.settings.host === 'vestaboard.local')) { L.setupBoard = 0; save({ host: f[0].ip }); }
          render(true);
        });
      }
    }
    return go();
  }
  if (key === 'what') { save({ serverEnabled: L.su.messages }); return go(); }
  if (key === 'start' && L.su.sources) {
    L.suBusy = true; L.suError = ''; render(true);
    return setupStarters()
      .then(() => RB.invoke('get_sources')).then(d => { L.suRotation = d.rotation; L.suBusy = false; go(); })
      .catch(e => { L.suBusy = false; L.suError = String(e); render(true); });
  }
  if (setupSteps()[L.step + 1] && setupSteps()[L.step + 1][0] === 'done' && L.setupSource === 'virtual' && !L.suRotation) {
    return RB.invoke('get_sources').then(d => { L.suRotation = d.rotation; go(); });
  }
  go();
}

// ---------------------------------------------------------------- render loop

let lastView = '', lastPage = '', pointerDown = false;
const draw = v => (v === 'history' ? viewHistory() : v === 'setup' ? viewSetup() : v === 'compose' ? viewCompose() : viewSettings());
// Natural height of what's in #root: fixed header/footer plus the full scrollable content.
const contentHeight = () => [...root.children].reduce((h, el) => h + (el.classList.contains('scroll') ? el.firstElementChild.offsetHeight : el.offsetHeight), 0);

// The window is sized once per view, to fit its tallest tab or step, so switching tabs never
// moves or resizes it. After that the user can resize it freely and the middle scrolls.
function fitWindow(v) {
  // Sources can run long; its tab scrolls rather than making every tab that tall.
  // Setup: every step of both flows (a standalone board's "Get started" step scrolls).
  const keys = v === 'settings' ? tabs().filter(t => t !== 'sources')
    : v === 'setup' ? [['local', 0], ['local', 1], ['local', 2], ['local', 3], ['virtual', 0], ['virtual', 1], ['virtual', 3]] : [null];
  const saved = v === 'settings' ? L.tab : [L.step, L.setupSource];
  let h = 0;
  for (const k of keys) {
    if (v === 'settings') L.tab = k; else if (v === 'setup') [L.setupSource, L.step] = k;
    root.innerHTML = draw(v);
    h = Math.max(h, contentHeight());
  }
  if (v === 'settings') L.tab = saved; else if (v === 'setup') [L.step, L.setupSource] = saved;
  RB.invoke('panel_resize', { width: WIDTH[v] || 600, height: Math.min(Math.ceil(h), v === 'history' ? 560 : 9999) });
}

function render(force) {
  if (!S) return;
  const a = document.activeElement;
  if (!force && (pointerDown || SR.dragging || (a && ((a.tagName === 'INPUT' && a.type !== 'range' && a.type !== 'checkbox') || a.tagName === 'TEXTAREA')))) return;
  const v = L.view;
  if (v !== lastView) { lastView = v; fitWindow(v); }
  // Keep the scroll position across live re-renders, but start each tab or step at the top.
  const page = [v, L.tab, L.step].join('/'), sc = root.querySelector('.scroll');
  const top = sc && page === lastPage ? sc.scrollTop : 0;
  lastPage = page;
  root.innerHTML = draw(v);
  const sc2 = root.querySelector('.scroll');
  if (sc2) sc2.scrollTop = top;
  if (v === 'compose') composeAfter();
  if (v === 'settings' && L.tab === 'sources') sourcesAfter();
}
document.addEventListener('pointerdown', () => (pointerDown = true), true);
document.addEventListener('pointerup', () => { pointerDown = false; setTimeout(render, 0); }, true);
document.addEventListener('focusout', () => setTimeout(render, 0));

function setView(v) {
  L.view = v;
  if (v === 'setup') Object.assign(L, { step: 0, test: 'idle', testLayout: null, setupSource: setupDefault(), su: newSetupChoices(), suBusy: false, suError: '', suRotation: null });
  if (v === 'settings' && L.tab === 'boards' && L.addMode === 'nearby') scanNearby();
  if (v === 'settings' && L.tab === 'display') loadScreens();
  if (v === 'settings' && L.tab === 'sources') srcOpen();
  render(true);
}
RB.listen('rb://view', e => setView(e.payload));

// Setup starts on a standalone board, or, run again, on what this board does now.
const setupDefault = () => (S && S.settings.setupDone ? S.settings.source : 'virtual');
L.su = newSetupChoices();

// Switching source also picks a sensible check interval; a virtual board needs its server.
const sourcePatch = v => Object.assign({ source: v, pollEvery: v === 'cloud' ? 15 : 2 }, v === 'virtual' ? { serverEnabled: true } : {});
const loadSecrets = () => RB.invoke('get_secrets').then(sec => { L.secrets = sec; render(true); });
function flashCopied(what) {
  L.copied = what; render(true);
  setTimeout(() => { if (L.copied === what) { L.copied = ''; render(true); } }, 1400);
}

const testFlow = () => {
  L.test = 'testing'; render(true);
  RB.invoke('test_connection')
    .then(r => { L.test = 'ok'; L.testLayout = r.layout; })
    .catch(e => { L.test = 'error'; L.testError = String(e); })
    .finally(() => render(true));
};

RB.delegate(root, 'click', {
  tab: el => {
    L.tab = el.dataset.v; L.showKey = false; render(true);
    if (L.tab === 'display') loadScreens();
    if (L.tab === 'boards' && L.addMode === 'nearby' && L.found2 !== null) scanNearby();
    if (L.tab === 'sources') srcOpen();
  },
  model: el => save({ model: el.dataset.v }),
  adaptive: el => save({ adaptive: el.dataset.v === 'auto' }),
  finish: el => save({ finish: el.dataset.v }),
  mode: el => { save({ mode: el.dataset.v }); if (L.view === 'settings') RB.invoke('show_board'); },
  floatLayer: el => save({ floatLayer: el.dataset.v }),
  corner: el => save({ position: el.dataset.v }),
  source: el => save(sourcePatch(el.dataset.v)),
  poll: el => save({ pollEvery: +el.dataset.v }),
  pauseOnHover: () => save({ pauseOnHover: !S.settings.pauseOnHover }),
  serverEnabled: () => save({ serverEnabled: !S.settings.serverEnabled }),
  serverLan: () => save({ serverLan: !S.settings.serverLan }),
  showServerKey: () => { L.showServerKey = !L.showServerKey; render(true); },
  copyKey: () => RB.copy(L.secrets.serverKey).then(() => flashCopied('key')),
  copyCurl: el => RB.copy(el.dataset.curl).then(() => flashCopied('curl')),
  newKey: () => { L.pair = null; RB.invoke('regenerate_server_key').then(loadSecrets); },
  pair: () => { if (S.settings.serverLan) showPair(); else { L.pairAsk = true; render(true); } },
  pairLan: () => { L.pairAsk = false; L.pairWait = true; save({ serverLan: true }); },
  pairCancel: () => { L.pairAsk = false; render(true); },
  hidePair: () => { L.pair = null; render(true); },
  copyPair: () => RB.copy(L.pair.url).then(() => flashCopied('pair')),
  compose: () => RB.invoke('open_window', { view: 'compose' }),
  addMode: el => { L.addMode = el.dataset.v; L.boardMsg = null; render(true); if (L.addMode === 'nearby') scanNearby(); },
  scanAgain: scanNearby,
  addFound: el => { const f = L.found2[+el.dataset.v]; Object.assign(L.form, { host: f.host, name: f.name.replace(/^Rattleboard\s*\((.*)\)$/, '$1') || f.name, key: '' }); L.addMode = 'manual'; L.boardMsg = null; render(true); },
  addManual: () => addBoard('add_board', { host: L.form.host, key: L.form.key, name: L.form.name }),
  addLink: () => RB.invoke('read_pair_link', { link: L.form.link })
    .then(c => { L.pairConfirm = c; L.boardMsg = null; })
    .catch(e => { L.boardMsg = { ok: false, text: String(e) }; })
    .finally(() => render(true)),
  addLinkYes: () => { L.pairConfirm = null; addBoard('add_pair_link', { link: L.form.link }); },
  addLinkNo: () => { L.pairConfirm = null; render(true); },
  addCloud: () => addBoard('add_cloud_board', { token: L.form.token, name: L.form.name }),
  rename: el => { L.renaming = el.dataset.v; render(true); const i = root.querySelector('[data-rename]'); if (i) { i.focus(); i.select(); } },
  renameDone: () => { const i = root.querySelector('[data-rename]'); if (i) i.blur(); L.renaming = null; render(true); },
  removeBoard: el => { L.removing = el.dataset.v; render(true); },
  groupNew: () => {
    const id = 'g-' + Date.now().toString(36), n = (S.groups || []).length + 1;
    L.renamingGroup = id;
    saveGroups([...(S.groups || []), { id, name: `Group ${n}`, members: [] }]).then(() => {
      const i = root.querySelector('[data-group-name]'); if (i) { i.focus(); i.select(); }
    });
  },
  groupRename: el => { L.renamingGroup = el.dataset.v; render(true); const i = root.querySelector('[data-group-name]'); if (i) { i.focus(); i.select(); } },
  groupRenameDone: () => { const i = root.querySelector('[data-group-name]'); if (i) i.blur(); L.renamingGroup = null; render(true); },
  groupDelete: el => saveGroups(S.groups.filter(g => g.id !== el.dataset.v)),
  groupMember: el => saveGroups(S.groups.map(g => g.id !== el.dataset.g ? g
    : { ...g, members: g.members.includes(el.dataset.v) ? g.members.filter(m => m !== el.dataset.v) : [...g.members, el.dataset.v] })),
  removeYes: el => { L.removing = null; RB.invoke('remove_board', { id: el.dataset.v }); },
  removeNo: () => { L.removing = null; render(true); },
  notifyEnabled: () => save({ notifyEnabled: !S.settings.notifyEnabled }),
  notifyAccess: () => { L.notifyError = ''; RB.invoke('notify_request_access').catch(e => { L.notifyError = String(e); render(true); }); },
  notifyHideContent: () => save({ notifyHideContent: !S.settings.notifyHideContent }),
  notifyQuiet: () => save({ notifyQuiet: !S.settings.notifyQuiet }),
  notifyReturn: el => save({ notifyReturnSec: +el.dataset.v }),
  sound: () => save({ sound: !S.settings.sound }),
  launchLogin: () => save({ launchLogin: !S.settings.launchLogin }),
  updateCheck: () => save({ updateCheck: !S.settings.updateCheck }),
  checkUpdates,
  openUpdate: (el, e) => { e.preventDefault(); RB.invoke('open_update'); },
  site: (el, e) => { e.preventDefault(); RB.invoke('open_site'); },
  kofi: (el, e) => { e.preventDefault(); RB.invoke('open_kofi'); },
  fullscreenLaunch: () => save({ fullscreenLaunch: !S.settings.fullscreenLaunch }),
  showKey: () => { L.showKey = !L.showKey; render(true); },
  quitApp: () => RB.invoke('quit'),
  test: () => RB.invoke('test_connection').catch(() => {}),
  clear: () => RB.invoke('clear_history'),
  flip: el => RB.invoke('show_history_entry', { id: +el.dataset.v }),
  resend: el => resend(el.dataset.v, el.dataset.t),
  editGrid: el => { composeEdit(resendGrids[el.dataset.v]); RB.invoke('open_window', { view: 'compose' }); },
  setupSource: el => { L.setupSource = el.dataset.v; render(true); },
  pickBoard: el => { const i = +el.dataset.v; L.setupBoard = i; save({ host: L.found[i].ip }); },
  setupTest: testFlow,
  back: () => { L.step = Math.max(0, L.step - 1); render(true); },
  next: setupNext,
  suSources: () => { L.su.sources = !L.su.sources; render(true); },
  suMessages: () => { L.su.messages = !L.su.messages; render(true); },
  suPick: el => { L.su[el.dataset.v] = !L.su[el.dataset.v]; render(true); },
  suCd: el => { L.su.cdPreset = el.dataset.v; render(true); },
  suFeed: el => { L.su.feed = el.dataset.v; render(true); },
  suHa: (el, e) => { e.preventDefault(); RB.invoke('open_docs', { section: 'home-assistant' }); }
});

// Text fields commit when they lose focus (or on Enter); sliders update live and save on release.
root.addEventListener('change', e => {
  const t = e.target;
  if (t.dataset.rename) {
    const id = t.dataset.rename, name = t.value.trim(); L.renaming = null;
    return name ? RB.invoke('rename_board', { id, name }) : render(true);
  }
  if (t.dataset.groupName) {
    const id = t.dataset.groupName, name = t.value.trim(); L.renamingGroup = null;
    return name ? saveGroups(S.groups.map(g => (g.id === id ? { ...g, name } : g))) : render(true);
  }
  if (t.dataset.sendPick != null) {
    const target = t.value; t.value = '';
    return target && resend(t.dataset.sendPick, target);
  }
  if (t.dataset.notifyApp != null) {
    const id = t.dataset.notifyApp, rest = S.settings.notifyApps.filter(a => a.id !== id);
    return save({ notifyApps: t.checked ? [...rest, { id, name: t.dataset.name }] : rest });
  }
  if (t.dataset.su) { L.su[t.dataset.su] = t.value; return; }
  const f = t.dataset.field; if (!f) return;
  const v = t.value;
  if (f === 'localKey' || f === 'cloudToken') {
    L.secrets[f] = v.trim();
    RB.invoke('set_secret', { kind: f === 'cloudToken' ? 'cloud' : 'local', value: v });
  } else if (f === 'host' || f === 'boardName' || f === 'notifyTarget' || f === 'notifyQuietFrom' || f === 'notifyQuietTo') save({ [f]: v.trim() });
  else if (f === 'hideSec') save({ hideSec: +v });
  else if (f === 'screen') {
    const d = v === '' ? null : L.screens[+v];
    save(d ? { screenName: d.name, screenPos: { x: d.x, y: d.y } } : { screenName: '', screenPos: { x: 0, y: 0 } });
  }
  else if (f === 'rows' || f === 'cols') {
    const [lo, hi] = f === 'rows' ? [2, 12] : [8, 40], n = Math.round(+v);
    if (n >= lo && n <= hi) save({ [f]: n }); else e.target.value = S.settings[f];
  }
  else if (f === 'speed') save({ speed: +v });
  else if (f === 'scale') save({ scale: +v });
  else if (f === 'serverPort') {
    const port = Math.round(+v);
    if (port >= 1024 && port <= 65535) save({ serverPort: port });
    else e.target.value = S.settings.serverPort;
  }
});
root.addEventListener('input', e => {
  if (e.target.dataset.su) L.su[e.target.dataset.su] = e.target.value;
  if (e.target.dataset.form) { L.form[e.target.dataset.form] = e.target.value; if (e.target.dataset.form === 'link') L.pairConfirm = null; }
  const f = e.target.dataset.field;
  if (f === 'hideSec') document.getElementById('hideLabel').textContent = e.target.value + ' s';
  if (f === 'speed') document.getElementById('speedLabel').textContent = speedLabel(e.target.value);
  if (f === 'scale') document.getElementById('scaleLabel').textContent = Math.round(e.target.value * 100) + '%';
});
root.addEventListener('keydown', e => {
  if (e.key !== 'Enter' || e.target.tagName !== 'INPUT') return;
  const form = e.target.dataset.form;
  // Enter in the add-board form adds the board.
  if (form) { e.preventDefault(); const b = root.querySelector('[data-act^="add"]:not([data-act="addMode"]):not([data-act="addFound"])'); if (b && !b.disabled) b.click(); return; }
  e.target.blur();
});
document.addEventListener('keydown', e => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'w') RB.invoke('close_window', { label: 'panel' });
  else if (e.key === 'F11') { e.preventDefault(); RB.invoke('toggle_fullscreen'); }
});

loadSecrets();
loadScreens();
RB.onState(snap => {
  const first = !S, wasRunning = S && S.server.running, wasLan = S && S.settings.serverLan; S = snap;
  // The server creates its key the first time it starts.
  if (!first && S.server.running && !wasRunning) loadSecrets();
  // "Turn on" in the pairing prompt: show the code once the server listens on the network.
  if (L.pairWait && S.settings.serverLan && S.server.running && S.server.urls.length > 1) { L.pairWait = false; showPair(); }
  // The pairing code shows the old address and key once either changes.
  if (L.pair && ((wasLan && !S.settings.serverLan) || !S.server.running)) L.pair = null;
  if (first && L.view === 'setup') L.setupSource = setupDefault();
  render();
});
setInterval(() => { if (L.view === 'settings' && L.tab === 'connection') render(); }, 1000);
