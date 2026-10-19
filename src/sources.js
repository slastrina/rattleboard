// Settings → Sources: the rotation of plugin cards, a live preview, Up next, Add source,
// import and export, approving command plugins, and Community plugins (community.js). panel.js draws it as one of its tabs
// (viewSources); the engine itself is in src-tauri/src/sources.rs.
const SR = {
  data: null, sel: null, page: 0, preview: null, pseq: 0,
  picker: false, open: null, msg: null,
  revealed: {}, replacing: {}, drafts: {},
  approve: null, countdown: null, link: '', linkConfirm: null, dragging: false, drop: false
};
const CHIP_NAMES = [['', 'Plugin colour'], ['red', 'Red'], ['orange', 'Orange'], ['yellow', 'Yellow'], ['green', 'Green'], ['blue', 'Blue'], ['violet', 'Violet'], ['white', 'White']];
const CHIP_CODE = { red: 63, orange: 64, yellow: 65, green: 66, blue: 67, violet: 68, white: 69, black: 70 };
const DWELLS = [[0, 'Plugin default'], [10, '10 s'], [20, '20 s'], [30, '30 s'], [60, '1 min'], [120, '2 min'], [300, '5 min'], [600, '10 min'], [900, '15 min'], [1800, '30 min']];
const DAYS = [['mon', 'M'], ['tue', 'T'], ['wed', 'W'], ['thu', 'T'], ['fri', 'F'], ['sat', 'S'], ['sun', 'S']];
const KINDS = { native: 'Built-in', rss: 'RSS', json: 'JSON', ical: 'Calendar', text: 'Text', command: 'Command' };
const PRESETS = [['nye', "New Year's Eve"], ['christmas', 'Christmas'], ['halloween', 'Halloween'], ['birthday', 'Birthday'], ['', 'Other date']];

// Outside #root, so a re-render while the file dialog is open doesn't lose its answer.
const srcFile = Object.assign(document.createElement('input'), { type: 'file', accept: '.json,application/json', multiple: true });
srcFile.style.display = 'none';
document.body.appendChild(srcFile);
srcFile.addEventListener('change', () => { importFiles(srcFile.files); srcFile.value = ''; });

const srcBoard = document.createElement('split-flap-board');
srcBoard.id = 'srcBoard';

const plugin = id => (SR.data ? SR.data.plugins.find(p => p.id === id) : null);
const inst = id => (SR.data ? SR.data.rotation.find(x => x.id === id) : null);
const chipHex = c => (c && CHIP_CODE[c] ? RB.HEX[CHIP_CODE[c]] : null);
const pluginChip = p => (p && p.card && p.card.chip) || '';
const fmtDur = s => (!s ? '' : s % 3600 === 0 ? s / 3600 + ' h' : s % 60 === 0 ? s / 60 + ' min' : s + ' s');
const dur = v => { // "30s", "15m", "1h", 90 -> seconds
  if (typeof v === 'number') return v;
  const m = String(v || '').match(/^(\d+)\s*([smh]?)$/i);
  return m ? +m[1] * ({ s: 1, m: 60, h: 3600 }[(m[2] || 's').toLowerCase()]) : 0;
};
const clock = t => new Date(t).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
const togv = (on, act, v) => `<div class="tog ${on ? 'on' : ''}" data-act="${act}" data-v="${esc(v)}"><div></div></div>`;

function srcRefresh() {
  return RB.invoke('get_sources').then(d => {
    SR.data = d;
    if (!SR.sel || !inst(SR.sel)) SR.sel = d.rotation.length ? d.rotation[0].id : null;
    render();
  });
}
let lastRefresh = 0, refreshTimer = null;
// The engine says something changed: a card went up, a feed came in.
RB.listen('rb://sources', () => {
  if (L.view !== 'settings' || L.tab !== 'sources') return;
  const wait = Math.max(0, 1000 - (Date.now() - lastRefresh));
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => { lastRefresh = Date.now(); srcRefresh().then(() => srcPreview(false)); }, wait);
});
function srcOpen() { CM.open = false; srcRefresh().then(() => srcPreview(true)); }
setInterval(() => { if (L.view === 'settings' && L.tab === 'sources') { srcStatusTick(); } }, 1000);
setInterval(() => { if (L.view === 'settings' && L.tab === 'sources' && !SR.dragging) srcRefresh(); }, 15000);

function flash(text, ok = false) {
  SR.msg = { text: String(text), ok }; render(true);
  setTimeout(() => { if (SR.msg && SR.msg.text === String(text)) { SR.msg = null; render(true); } }, ok ? 3500 : 6000);
}

// ---------------------------------------------------------------- saving

const plainInst = x => ({ id: x.id, plugin: x.plugin, name: x.name, enabled: x.enabled, weight: x.weight, dwell: x.dwell, schedule: x.schedule, settings: x.settings, chip: x.chip, include: x.include, exclude: x.exclude, alertWords: x.alertWords });
function saveRot(mutate) {
  const rot = SR.data.rotation.map(x => ({ ...x, settings: { ...x.settings } }));
  mutate(rot);
  SR.data.rotation = rot;
  render(true);
  return RB.invoke('save_sources', { sources: { rotation: rot.map(plainInst) } })
    .catch(e => flash(e))
    .then(srcRefresh)
    .then(() => srcPreview(false));
}
const saveInst = (id, patch) => saveRot(rot => { const x = rot.find(r => r.id === id); if (x) Object.assign(x, patch); });
const saveSetting = (id, key, value) => saveRot(rot => { const x = rot.find(r => r.id === id); if (x) x.settings[key] = value; });
function saveCfg(patch) {
  Object.assign(SR.data, patch); render(true);
  return RB.invoke('save_sources', { sources: patch }).catch(e => flash(e)).then(srcRefresh);
}

// ---------------------------------------------------------------- preview

const srcTile = cols => Math.max(8, Math.min(24, Math.round(540 / (1.17 * cols))));
function srcPreviewHeight(rows, cols) {
  const t = srcTile(cols), h = Math.round(t * 1.42);
  return rows * h + (rows - 1) * Math.max(3, Math.round(t * 0.3)) + 2 * Math.max(22, Math.round(t * 1.15));
}
function srcPreview(reset) {
  if (!SR.sel) { SR.preview = null; return render(true); }
  const id = SR.sel;
  if (reset) SR.page = 0;
  return RB.invoke('source_preview', { instance: id })
    .then(p => { if (SR.sel !== id) return; SR.preview = p; if (SR.page >= p.pages.length) SR.page = 0; srcShow(); render(); })
    .catch(e => { SR.preview = { pages: [], reason: String(e), rows: 6, cols: 22 }; render(); });
}
// Refresh preview: fetches the source's feed or API again now (the engine skips it if it was
// fetched in the last few seconds), then says what happened and flips the card again, so
// the click shows even when nothing changed.
function srcRefreshNow(id) {
  const before = SR.preview && SR.sel === id ? SR.preview.fetched : null;
  SR.sel = id; SR.refreshing = id; SR.note = null; render(true);
  RB.invoke('source_preview', { instance: id, refresh: true })
    .then(p => {
      if (SR.sel !== id) return;
      SR.preview = p; SR.page = 0;
      const fresh = p.fetched && (p.fetched !== before || Date.now() - p.fetched < 10000);
      SR.note = !p.fetches ? { id, ok: true, text: `Updated ${clock(Date.now())} · ${p.reason}` }
        : fresh ? { id, ok: true, text: `Updated ${clock(p.fetched)} · ${p.reason}` }
        : { id, ok: false, text: `Couldn't refresh: ${p.reason}` };
      srcShow();
      srcBoard.setAttribute('seq', ++SR.pseq);
    })
    .catch(e => { SR.note = { id, ok: false, text: `Couldn't refresh: ${e}` }; })
    .finally(() => {
      SR.refreshing = null; render(true);
      const note = SR.note;
      setTimeout(() => { if (SR.note === note) { SR.note = null; render(true); } }, 8000);
    });
}
function srcShow() {
  const p = SR.preview; if (!p || !p.pages.length) return;
  const s = S.settings, set = (k, v) => { if (srcBoard.getAttribute(k) !== String(v)) srcBoard.setAttribute(k, v); };
  set('rows', p.rows); set('cols', p.cols); set('tile', srcTile(p.cols)); set('finish', s.finish); set('speed', s.speed); set('sound', 'off');
  set('message', JSON.stringify(p.pages[SR.page] || p.pages[0]));
}
// After each render: put the persistent preview board back (it keeps flipping across renders).
function sourcesAfter() {
  if (CM.open) return communityAfter();
  const slot = document.getElementById('srcPreview');
  if (slot && SR.preview && SR.preview.pages.length) { slot.appendChild(srcBoard); srcShow(); }
}

// ---------------------------------------------------------------- view

function srcStatusText() {
  const st = S.sources || {}, s = SR.data;
  if (!s) return ['#9a9ca1', 'Loading…'];
  const paused = srcPaused();
  if (paused) return ['#e0a526', paused];
  if (!s.rotation.some(x => x.enabled)) return ['#9a9ca1', s.rotation.length ? 'Every source is off' : 'Add a source to start a rotation on this board'];
  if (st.quiet) return ['#8a63c9', 'Quiet hours: nothing new goes up until ' + s.quietHours.to];
  if (st.holdUntil && st.holdUntil > Date.now()) return ['#e0a526', `Holding a message until ${clock(st.holdUntil)}; then the rotation carries on`];
  if (st.showing) {
    const left = st.until ? Math.max(0, Math.ceil((st.until - Date.now()) / 1000)) : 0;
    return ['#34b05a', `${st.how === 'alert' ? 'Alert' : 'Showing'}: ${st.showing}${left ? ` · next in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}` : ''}`];
  }
  return ['#34b05a', 'Running'];
}
function srcStatusTick() {
  const el = document.getElementById('srcStatus');
  if (el) { const [c, t] = srcStatusText(); el.textContent = t; document.getElementById('srcDot').style.background = c; }
}

// Paused sources say how to get them going: finish setup, or (while mirroring) show them
// anyway or stop mirroring and make this a standalone board.
// Why the rotation is paused, from this window's settings (as sources_ctx in lib.rs decides),
// so it changes the moment they do; the engine's own status follows within a second.
function srcPaused() {
  const s = S.settings;
  if (!s.setupDone) return 'Finish setting up Rattleboard first';
  if (['local', 'cloud', 'demo'].includes(s.source) && SR.data && !SR.data.showWhileMirroring) return `Paused while mirroring ${s.source === 'demo' ? 'the demo' : 'another board'}`;
  return null;
}
function pausedRow() {
  const s = S.settings;
  if (!srcPaused()) return '';
  if (!s.setupDone) return `<div></div><div style="display:flex; gap:8px; flex-wrap:wrap; margin-top:-6px"><button class="btn" data-act="srcRunSetup">Run setup</button></div>`;
  if (s.source === 'virtual') return '';
  const what = s.source === 'demo' ? 'the demo' : 'the board it mirrors';
  if (SR.confirmStandalone) return `<div></div><div style="display:flex; align-items:center; gap:8px; flex-wrap:wrap; margin-top:-6px">
      <span class="hint" style="text-wrap:pretty">This board stops showing ${what} and shows its own sources and the messages sent to it. Change it back in Settings → Connection.</span>
      <button class="primary" data-act="srcStandaloneYes">Make it standalone</button><button class="btn" data-act="srcStandaloneNo">Cancel</button></div>`;
  return `<div></div><div style="display:flex; gap:8px; flex-wrap:wrap; margin-top:-6px">
      <button class="btn" data-act="srcShowAnyway" title="Turns on Show sources while mirroring, below">Show sources anyway</button>
      <button class="btn" data-act="srcStandalone" title="Sets Connection → Read from to Virtual board">Make this a standalone board</button></div>`;
}

function viewSources() {
  if (!SR.data) return `<div class="hint" style="padding:30px 0; text-align:center">Loading sources…</div>`;
  if (CM.open) return viewCommunity();
  const d = SR.data, [dot, status] = srcStatusText(), p = SR.preview;
  const sel = inst(SR.sel);
  const preview = sel ? `<div style="grid-column:1 / -1; display:flex; flex-direction:column; gap:8px">
      <div style="display:flex; justify-content:center; padding:12px 0; border-radius:10px; background:rgba(0,0,0,.05); overflow:hidden">
        <div id="srcPreview" style="min-height:${srcPreviewHeight((p && p.rows) || 6, (p && p.cols) || 22)}px; display:flex; align-items:center; justify-content:center">${p && !p.pages.length ? `<span class="hint">${esc(p.reason)}</span>` : ''}</div>
      </div>
      <div style="display:flex; align-items:center; gap:8px; min-width:0">
        <span style="font-weight:600; white-space:nowrap">${esc(sel.label || sel.name)}</span>
        <span class="hint" id="srcReason" style="flex:1; min-width:0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis${SR.note && SR.note.id === sel.id && !SR.note.ok ? '; color:#c0392b' : ''}">${esc(SR.note && SR.note.id === sel.id ? SR.note.text : p ? p.reason : '')}</span>
        ${p && p.pages.length > 1 ? `<span class="hint">Page ${SR.page + 1} of ${p.pages.length}</span><button class="btn" data-act="srcPage">Next page</button>` : ''}
        <button class="btn" data-act="srcReplay">Replay</button>
      </div>
    </div>` : '';
  return `<div class="grid" style="gap:14px 16px" id="srcRoot">
    <div class="lbl">Now</div>
    <div style="display:flex; align-items:center; gap:10px; min-width:0">
      <div class="dot" id="srcDot" style="background:${dot}"></div><span id="srcStatus" style="flex:1; min-width:0; text-wrap:pretty">${esc(status)}</span>
      <button class="btn" style="flex:none" data-act="srcNext" ${S.sources && S.sources.active ? '' : 'disabled'} title="Show the next source now">Next</button>
    </div>
    ${pausedRow()}
    ${preview}
    <div style="grid-column:1 / -1; display:flex; flex-direction:column; gap:8px; min-width:0">
      <div style="display:flex; align-items:baseline; gap:10px"><span style="font-weight:600">Rotation</span><span class="hint">Takes turns in this order; drag ⋮⋮ to reorder</span></div>
      ${rotationList()}
      <div style="display:flex; align-items:center; gap:8px; flex-wrap:wrap">
        <button class="btn" data-act="srcPicker">${SR.picker ? 'Close' : 'Add source…'}</button>
        <button class="btn" data-act="srcImport">Import…</button>
        <button class="btn" data-act="cmOpen" title="Browse plugins other people made">Community plugins</button>
        <span class="hint">Or drop a .rattleboard-plugin.json file here</span>
      </div>
      ${SR.msg ? `<div style="font-size:12px; color:${SR.msg.ok ? '#2f8a4c' : '#c0392b'}; text-wrap:pretty">${esc(SR.msg.text)}</div>` : ''}
      ${SR.picker ? picker() : ''}
    </div>
    <div style="grid-column:1 / -1; display:flex; flex-direction:column; gap:8px; min-width:0">
      <div style="display:flex; align-items:baseline; gap:10px"><span style="font-weight:600">Up next</span><span class="hint">The coming hour, and why</span></div>
      ${upNext()}
    </div>
    <div style="grid-column:1 / -1; font-weight:600; margin-top:4px">How sources run</div>
    ${engineRows()}
    <div style="grid-column:1 / -1" class="hint">Sources run on this computer only. Secrets (tokens, private calendar addresses) are kept in ${RB.keychain(S)}, one entry per source, and never written to a file or sent anywhere but the source's own address.</div>
  </div>
  ${SR.approve ? approveDialog() : ''}${SR.countdown ? countdownDialog() : ''}`;
}

function summary(x, p) {
  const bits = [p ? p.name : x.plugin];
  bits.push(x.dwell ? 'shows ' + fmtDur(x.dwell) : p && p.dwell ? 'shows ' + fmtDur(dur(p.dwell)) : 'shows 5 min');
  if (x.weight > 1) bits.push('weight ' + x.weight);
  if (x.schedule && (x.schedule.days.length || x.schedule.from)) bits.push(scheduleText(x.schedule));
  return bits.join(' · ');
}
function scheduleText(s) {
  const days = s.days.length === 5 && !s.days.includes('sat') && !s.days.includes('sun') ? 'weekdays' : s.days.length === 2 && s.days.includes('sat') && s.days.includes('sun') ? 'weekends' : s.days.join(' ');
  return [days, s.from && s.to && s.from !== s.to ? `${s.from}–${s.to}` : ''].filter(Boolean).join(' ');
}

function rotationList() {
  const d = SR.data;
  if (!d.rotation.length) return `<div style="border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); padding:14px; color:#6b6d72; line-height:1.45">No sources yet. Add a clock, the weather, a countdown or a news feed, and the board takes turns showing them.</div>`;
  const rows = d.rotation.map((x, i) => {
    const p = plugin(x.plugin), color = chipHex(x.chip || pluginChip(p)) || '#b9bbc0', on = x.id === SR.sel, open = x.id === SR.open;
    const needsApproval = p && p.kind === 'command' && !p.approved;
    const err = x.error ? `<span style="color:#c0392b"> · ${esc(x.error)}</span>` : needsApproval ? `<span style="color:#c0392b"> · needs approval</span>` : '';
    return `<div class="srcRow" data-i="${i}" style="border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}; background:${on ? 'rgba(0,0,0,.03)' : 'transparent'}">
      <div style="display:flex; align-items:center; gap:10px; padding:8px 12px 8px 6px" data-act="srcSelect" data-v="${esc(x.id)}">
        <span class="srcGrip" draggable="true" data-i="${i}" title="Drag to reorder" style="cursor:grab; color:#a0a2a7; font-size:14px; letter-spacing:-2px; padding:0 4px; user-select:none">⋮⋮</span>
        ${togv(x.enabled, 'srcEnabled', x.id)}
        <div class="dot" style="background:${color}; width:10px; height:10px; border-radius:2px"></div>
        <div style="display:flex; flex-direction:column; gap:2px; flex:1; min-width:0">
          <span style="font-weight:600; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; opacity:${x.enabled ? 1 : 0.55}">${esc(x.label || x.name)}</span>
          <span style="font-size:11.5px; color:#6b6d72; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(summary(x, p))}${err}</span>
        </div>
        <button class="btn" data-act="srcEdit" data-v="${esc(x.id)}">${open ? 'Done' : 'Edit'}</button>
      </div>
      ${open ? editor(x, p) : ''}
    </div>`;
  }).join('');
  return `<div id="srcList" style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); overflow:hidden">${rows}</div>`;
}

// ---------------------------------------------------------------- one source's settings

function field(x, spec) {
  const v = x.settings[spec.key] ?? spec.default, data = `data-src-set="${esc(spec.key)}" data-inst="${esc(x.id)}"`;
  switch (spec.type) {
    case 'toggle': return `<div style="display:flex; align-items:center; gap:10px">${togv(!!v, 'srcSetToggle', x.id + '|' + spec.key)}</div>`;
    case 'select': return `<select class="field" style="justify-self:start; width:240px" ${data}>${spec.options.map(o => {
      const val = o && typeof o === 'object' ? o.value : o, lab = o && typeof o === 'object' ? o.label : o;
      return `<option value="${esc(val)}" ${String(val) === String(v) ? 'selected' : ''}>${esc(lab)}</option>`;
    }).join('')}</select>`;
    case 'number': return `<input class="field" type="number" style="justify-self:start; width:120px" value="${esc(v)}" ${data}>`;
    case 'date': return `<input class="field" type="date" style="justify-self:start; width:160px" value="${esc(v)}" ${data}>`;
    case 'time': return `<input class="field" type="time" style="justify-self:start; width:120px" value="${esc(v)}" ${data}>`;
    case 'lines': return `<textarea class="field" rows="4" spellcheck="false" style="width:100%; height:auto; padding:6px 8px; resize:vertical; line-height:1.4" ${data}>${esc((v || []).join('\n'))}</textarea>`;
    default: return `<input class="field" style="width:100%; max-width:340px" value="${esc(v)}" ${data} spellcheck="false" autocomplete="off">`;
  }
}

function secretRow(x, s) {
  const k = x.id + '|' + s.key, set = x.secrets && x.secrets[s.key] && x.secrets[s.key].set;
  const shown = SR.revealed[k];
  const control = set && !SR.replacing[k]
    ? `<input class="field mono" style="width:220px; font-size:12px" readonly value="${esc(shown != null ? shown : '••••••••••••')}">
       <button class="btn" data-act="srcReveal" data-v="${esc(k)}">${shown != null ? 'Hide' : 'Show'}</button>
       <button class="btn" data-act="srcReplace" data-v="${esc(k)}">Replace</button>`
    : `<input class="field" type="password" style="width:220px" placeholder="${set ? 'New value' : 'Paste it here'}" data-src-secret="${esc(k)}" autocomplete="off" spellcheck="false" value="${esc(SR.drafts[k] || '')}">
       <button class="btn" data-act="srcSecretSave" data-v="${esc(k)}">Save</button>${set ? `<button class="btn" data-act="srcReplaceCancel" data-v="${esc(k)}">Cancel</button>` : ''}`;
  return `<div class="lbl">${esc(s.label)}</div><div style="display:flex; align-items:center; gap:6px; min-width:0">${control}</div>`;
}

function editor(x, p) {
  if (!p) return `<div style="padding:0 14px 12px" class="hint">Its plugin (${esc(x.plugin)}) was removed.</div>`;
  const countdown = p.id === 'rattleboard.countdown', feed = p.kind === 'rss' || p.kind === 'json';
  const specs = (p.settings || []).filter(s => !countdown || !['preset', 'title', 'date', 'time', 'yearly'].includes(s.key));
  const sch = x.schedule || { days: [], from: null, to: null }, timed = !!(x.schedule && (x.schedule.days.length || x.schedule.from));
  const dwells = DWELLS.some(dw => dw[0] === x.dwell) ? DWELLS : [...DWELLS, [x.dwell, fmtDur(x.dwell)]];
  return `<div class="grid" style="grid-template-columns:130px minmax(0,1fr); gap:10px 14px; padding:4px 16px 16px 68px">
    ${p.description ? `<div></div><div class="hint" style="margin-top:-2px">${esc(p.description)}</div>` : ''}
    <div class="lbl">Name</div><input class="field" style="justify-self:start; width:240px" value="${esc(x.name)}" data-src-name="${esc(x.id)}" placeholder="${esc(p.name)}">
    ${countdown ? countdownFields(x) : ''}
    ${specs.map(s => `<div class="lbl" ${s.type === 'lines' ? 'style="align-self:start; padding-top:4px"' : ''}>${esc(s.label)}</div>${field(x, s)}`).join('')}
    ${(p.secrets || []).map(s => secretRow(x, s)).join('')}
    <div class="lbl">Show for</div>
    <div style="display:flex; align-items:center; gap:10px"><select class="field" style="width:140px" data-src-dwell="${esc(x.id)}">${dwells.map(([v, l]) => `<option value="${v}" ${v === x.dwell ? 'selected' : ''}>${esc(v === 0 && p.dwell ? `Default (${fmtDur(dur(p.dwell))})` : l)}</option>`).join('')}</select>
      <span class="hint">then the next source</span></div>
    <div class="lbl">Weight</div>
    <div style="display:flex; align-items:center; gap:10px">${seg([[1, '1'], [2, '2'], [3, '3'], [5, '5'], [10, '10']], x.weight, 'srcWeight')}<span class="hint">Comes round this many times as often</span></div>
    <div class="lbl">Colour</div>
    <select class="field" style="justify-self:start; width:160px" data-src-chip="${esc(x.id)}">${CHIP_NAMES.map(([v, l]) => `<option value="${v}" ${(x.chip || '') === v ? 'selected' : ''}>${l}${!v && pluginChip(p) ? ` (${pluginChip(p)})` : ''}</option>`).join('')}</select>
    <div class="lbl">Schedule</div>
    <div style="display:flex; flex-direction:column; gap:8px; align-items:flex-start">
      <div style="display:flex; align-items:center; gap:10px">${togv(timed, 'srcTimed', x.id)}<span class="hint">${timed ? 'Only on these days, at these times' : 'Any time'}</span></div>
      ${timed ? `<div style="display:flex; align-items:center; gap:4px; flex-wrap:wrap">
        ${DAYS.map(([v, l]) => `<button class="chip ${sch.days.includes(v) ? 'on' : ''}" style="width:28px; padding:3px 0" data-act="srcDay" data-v="${v}" title="${v}">${l}</button>`).join('')}
        <input class="field" type="time" style="width:110px; margin-left:8px" value="${esc(sch.from || '')}" data-src-sched="from">
        <span class="hint">to</span>
        <input class="field" type="time" style="width:110px" value="${esc(sch.to || '')}" data-src-sched="to"></div>
        <span class="hint">No days ticked means every day. A time span can run past midnight.</span>` : ''}
    </div>
    ${feed ? `<div class="lbl">Only items with</div><input class="field" style="justify-self:start; width:240px" value="${esc((x.include || []).join(', '))}" data-src-words="include" placeholder="Any words, comma-separated">
      <div class="lbl">Skip items with</div><input class="field" style="justify-self:start; width:240px" value="${esc((x.exclude || []).join(', '))}" data-src-words="exclude" placeholder="Words to leave out">
      <div class="lbl">Alert on</div><div style="display:flex; align-items:center; gap:10px"><input class="field" style="width:240px" value="${esc((x.alertWords || []).join(', '))}" data-src-words="alertWords" placeholder="e.g. outage, launch"><span class="hint">Interrupts, then goes back</span></div>` : ''}
    ${p.kind === 'command' ? `<div class="lbl">Program</div><div style="display:flex; align-items:center; gap:10px; min-width:0"><span class="mono" style="font-size:11.5px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc([p.run.path, ...(p.run.args || [])].join(' '))}</span>
      ${p.approved ? `<span class="hint" style="color:#2f8a4c">Approved</span><button class="btn" data-act="srcRevoke" data-v="${esc(p.id)}">Revoke</button>` : `<button class="btn" data-act="srcApprove" data-v="${esc(p.id)}">Review and allow…</button>`}</div>` : ''}
    <div></div>
    <div style="display:flex; gap:6px; margin-top:4px">
      <button class="btn" data-act="srcPreviewNow" data-v="${esc(x.id)}" ${SR.refreshing === x.id ? 'disabled' : ''}>${SR.refreshing === x.id ? 'Refreshing…' : 'Refresh preview'}</button>
      <button class="btn" data-act="srcExport" data-v="${esc(x.id)}" title="Save it to Downloads, without its secrets">Export</button>
      <div style="flex:1"></div>
      <button class="btn" style="color:#c0392b" data-act="srcRemove" data-v="${esc(x.id)}">Remove</button>
    </div>
  </div>`;
}

// The countdown's own form: a preset, or a name and a date, and whether it comes round every year.
function countdownFields(x) {
  const s = x.settings, preset = s.preset || (/birthday/i.test(s.title || '') ? 'birthday' : '');
  const custom = !['nye', 'christmas', 'halloween'].includes(s.preset);
  return `<div class="lbl">Counts to</div>
    <div style="display:flex; flex-wrap:wrap; gap:6px">${PRESETS.map(([v, l]) => `<button class="chip ${(v === 'birthday' ? preset === 'birthday' : v === '' ? custom && preset !== 'birthday' : s.preset === v) ? 'on' : ''}" data-act="srcPreset" data-v="${v}">${l}</button>`).join('')}</div>
    ${custom ? `<div class="lbl">Name</div><input class="field" style="justify-self:start; width:240px" value="${esc(s.title || '')}" data-src-set="title" data-inst="${esc(x.id)}" placeholder="${preset === 'birthday' ? "Sam's birthday" : 'Launch day'}">
    <div class="lbl">Date</div><div style="display:flex; align-items:center; gap:8px"><input class="field" type="date" style="width:160px" value="${esc(s.date || '')}" data-src-set="date" data-inst="${esc(x.id)}">
      <input class="field" type="time" style="width:110px" value="${esc(s.time || '')}" data-src-set="time" data-inst="${esc(x.id)}"><span class="hint">time optional</span></div>
    <div class="lbl">Every year</div><div style="display:flex; align-items:center; gap:10px">${togv(!!s.yearly, 'srcSetToggle', x.id + '|yearly')}<span class="hint">Birthdays and anniversaries roll over to next year</span></div>` : ''}`;
}

// ---------------------------------------------------------------- add source

function picker() {
  const d = SR.data, builtin = d.plugins.filter(p => p.builtin), mine = d.plugins.filter(p => !p.builtin);
  const row = (p, i) => `<div style="display:flex; align-items:center; gap:12px; padding:9px 14px; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}">
      <div class="dot" style="background:${chipHex(pluginChip(p)) || '#b9bbc0'}; width:10px; height:10px; border-radius:2px"></div>
      <div style="display:flex; flex-direction:column; gap:2px; flex:1; min-width:0">
        <span><b style="font-weight:600">${esc(p.name)}</b> <span class="hint">${esc(KINDS[p.kind] || p.kind)}${p.kind === 'command' ? (p.approved ? ' · approved' : ' · needs approval') : ''}</span></span>
        ${p.description ? `<span style="font-size:11.5px; color:#6b6d72; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(p.description)}</span>` : ''}
      </div>
      ${p.builtin ? '' : `<button class="btn" data-act="srcExport" data-v="${esc(p.id)}">Export</button><button class="btn" data-act="srcUninstall" data-v="${esc(p.id)}">Delete</button>`}
      <button class="btn" data-act="srcAdd" data-v="${esc(p.id)}">Add</button>
    </div>`;
  const box = list => `<div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1)">${list.map(row).join('')}</div>`;
  const c = SR.linkConfirm;
  return `<div style="display:flex; flex-direction:column; gap:8px; margin-top:4px">
    <span class="hint" style="font-weight:600; color:#3b3c40">Built in</span>${box(builtin)}
    <span class="hint" style="font-weight:600; color:#3b3c40">Installed plugins</span>
    ${mine.length ? box(mine) : `<span class="hint">None yet. Import a .rattleboard-plugin.json file, or paste a plugin link.</span>`}
    <span class="hint">More plugins: <a href="#" data-act="cmOpen" style="color:${RB.ACC}; text-decoration:none">Community plugins</a> · <a href="#" data-act="cmGitHub" data-v="contribute" style="color:${RB.ACC}; text-decoration:none">Contribute on GitHub</a></span>
    <div style="display:flex; align-items:center; gap:8px; flex-wrap:wrap">
      <input class="field" style="flex:1; min-width:220px" value="${esc(SR.link)}" data-src-link="1" placeholder="rattleboard://plugin?url=https://…" spellcheck="false">
      <button class="btn" data-act="srcLink">Check link</button>
    </div>
    ${c ? linkConfirmBox(c, 'srcLinkYes', 'srcLinkNo') : ''}
  </div>`;
}
// Asking before installing a plugin fetched from a link (or from Community plugins).
function linkConfirmBox(c, yes, no, verb = 'Install') {
  return `<div style="display:flex; align-items:center; gap:10px; flex-wrap:wrap; background:#fff; border-radius:8px; padding:10px 14px; box-shadow:0 0 0 1px rgba(0,0,0,.1)">
      <span style="flex:1; min-width:0">${verb} <b style="font-weight:600">${esc(c.name)}</b> (${esc(KINDS[c.kind] || c.kind)})${c.url ? ` from <span class="mono" style="font-size:11.5px">${esc(String(c.url).replace(/^https?:\/\//, '').split('/')[0])}</span>` : ''}?${c.description ? ` <span class="hint">${esc(c.description)}</span>` : ''}</span>
      <button class="btn" data-act="${yes}">${verb}</button><button class="btn" data-act="${no}">Cancel</button></div>`;
}

function upNext() {
  const list = (SR.data.upNext || []).slice(0, 12);
  if (!list.length) return `<div class="hint" style="padding-top:2px">${S.sources && S.sources.paused ? esc(S.sources.paused) : 'Nothing coming up in the next hour.'}</div>`;
  return `<div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); overflow:hidden">${list.map((e, i) =>
    `<div style="display:grid; grid-template-columns:62px minmax(0,1fr) auto; gap:10px; align-items:center; padding:6px 14px; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}; background:${e.now ? 'rgba(52,176,90,.08)' : 'transparent'}">
      <span style="font-variant-numeric:tabular-nums; color:#55575c">${e.now ? 'Now' : clock(e.at)}</span>
      <span style="white-space:nowrap; overflow:hidden; text-overflow:ellipsis; font-weight:${e.now ? 600 : 400}">${esc(e.title)}</span>
      <span class="hint" style="white-space:nowrap">${esc((r => r.charAt(0).toUpperCase() + r.slice(1))(e.reason.replace(/^On now · /, '')))}</span></div>`).join('')}</div>`;
}

function engineRows() {
  const d = SR.data, q = d.quietHours, targets = d.targets || ['this'];
  const chips = RB.targets(S).filter(t => t[0] !== 'all').map(([v, l]) => `<button class="chip ${targets.includes(v) ? 'on' : ''}" data-act="srcTarget" data-v="${esc(v)}">${esc(l.replace(/ \(group\)$/, ''))}</button>`).join('');
  const remote = targets.filter(t => t !== 'this').length;
  return `<div class="lbl" style="align-self:start; padding-top:4px">Show sources on</div>
    <div style="display:flex; flex-direction:column; gap:6px; align-items:flex-start">
      <div style="display:flex; flex-wrap:wrap; gap:6px">${chips}</div>
      <span class="hint">${remote ? 'This computer sends each card to the boards ticked here as well: at least 5 s apart for a Vestaboard on its Local API, 15 s on the Cloud. A board that’s offline is tried again later and never holds the others up.' : 'Tick saved boards or groups to drive them from here too, as a hub.'}</span>
    </div>
    <div class="lbl">Order</div>${seg([['turn', 'In turn'], ['shuffle', 'Shuffle']], d.shuffle ? 'shuffle' : 'turn', 'srcShuffle')}
    <div class="lbl">Hold messages for</div>
    <div style="display:flex; align-items:center; gap:10px">${seg([[0, 'Off'], [5, '5 min'], [15, '15 min'], [30, '30 min'], [60, '1 h'], [120, '2 h']], d.holdMinutes, 'srcHold', 'poll')}</div>
    <div></div><div class="hint" style="margin-top:-10px">A message from Compose, the API or a sequence pauses the rotation this long.</div>
    <div class="lbl">Alerts stay up</div>${seg([[15, '15 s'], [30, '30 s'], [60, '1 min'], [120, '2 min'], [300, '5 min']], d.alertSeconds, 'srcAlert', 'poll')}
    <div class="lbl">Card changes</div>
    <div style="display:flex; align-items:center; gap:10px"><select class="field" style="width:110px" data-src-cfg="flipsPerHour">${[6, 12, 20, 30, 60, 120].map(n => `<option value="${n}" ${n === d.flipsPerHour ? 'selected' : ''}>${n} an hour</option>`).join('')}</select><span class="hint">at most, so the board stays calm</span></div>
    <div class="lbl">Quiet hours</div>
    <div style="display:flex; align-items:center; gap:8px">${tog(q.enabled, 'srcQuiet')}
      <input class="field" type="time" style="width:110px" value="${esc(q.from)}" data-src-quiet="from" ${q.enabled ? '' : 'disabled'}>
      <span class="hint">to</span>
      <input class="field" type="time" style="width:110px" value="${esc(q.to)}" data-src-quiet="to" ${q.enabled ? '' : 'disabled'}></div>
    <div class="lbl">While mirroring</div>
    <div style="display:flex; align-items:center; gap:10px">${tog(d.showWhileMirroring, 'srcMirror')}<span class="hint">Show sources while this board mirrors another one</span></div>
    <div class="lbl">24-hour time</div>
    <div style="display:flex; align-items:center; gap:10px">${tog(d.clock24, 'srcClock24')}<span class="hint">On clocks and the time in a card's corner</span></div>`;
}

// ---------------------------------------------------------------- dialogs

function modal(body) {
  return `<div style="position:fixed; inset:0; background:rgba(0,0,0,.28); display:flex; align-items:center; justify-content:center; z-index:20; padding:24px">
    <div style="background:#fff; border-radius:12px; box-shadow:0 20px 50px rgba(0,0,0,.35); max-width:500px; width:100%; padding:20px 22px; display:flex; flex-direction:column; gap:12px; max-height:90vh; overflow:auto">${body}</div></div>`;
}

function approveDialog() {
  const p = plugin(SR.approve.id); if (!p) return '';
  const args = (p.run.args || []).map(a => /[\s"']/.test(a) || !a ? JSON.stringify(a) : a);
  const every = p.run.every || p.every || '5m', timeout = p.run.timeout || '10s';
  return modal(`<div style="display:flex; gap:12px; align-items:flex-start">
      <div style="width:34px; height:34px; border-radius:50%; background:#fdecea; color:#c0392b; display:flex; align-items:center; justify-content:center; font-weight:700; font-size:18px; flex:none">!</div>
      <div style="display:flex; flex-direction:column; gap:4px"><div style="font-size:15px; font-weight:600">Allow “${esc(p.name)}” to run a program on this computer?</div>
      <div class="hint" style="line-height:1.45; text-wrap:pretty">This plugin runs the program below every ${esc(fmtDur(dur(every)) || every)}, as you, with access to everything you can open. Only allow it if you trust where it came from and you know what the program does.</div></div>
    </div>
    <div style="display:grid; grid-template-columns:90px minmax(0,1fr); gap:6px 12px; background:#f6f6f4; border-radius:8px; padding:12px 14px">
      <span class="lbl">Program</span><span class="mono" style="font-size:12px; overflow-wrap:anywhere; user-select:text">${esc(p.run.path)}</span>
      <span class="lbl">Arguments</span><span class="mono" style="font-size:12px; overflow-wrap:anywhere; user-select:text">${args.length ? esc(args.join(' ')) : '<span class="hint">none</span>'}</span>
      <span class="lbl">Runs</span><span>every ${esc(fmtDur(dur(every)) || every)}, stopped after ${esc(fmtDur(dur(timeout)) || timeout)}</span>
      <span class="lbl">Plugin</span><span class="mono" style="font-size:12px">${esc(p.id)}</span>
    </div>
    <div class="hint" style="line-height:1.45">What it prints goes on the board. It never runs because of the API or a link. If the plugin file changes, Rattleboard asks you again.</div>
    <div style="display:flex; justify-content:flex-end; gap:8px; margin-top:4px">
      <button class="btn" style="font-size:13px; padding:5px 14px" data-act="srcApproveNo">Don't allow</button>
      <button class="primary" style="background:#c0392b" data-act="srcApproveYes">Allow</button>
    </div>`);
}

// Adding a countdown: a preset, or a name and a date.
function countdownDialog() {
  const c = SR.countdown, custom = c.preset === '' || c.preset === 'birthday';
  return modal(`<div style="font-size:15px; font-weight:600">New countdown</div>
    <div style="display:flex; flex-wrap:wrap; gap:6px">${PRESETS.map(([v, l]) => `<button class="chip ${c.preset === v ? 'on' : ''}" data-act="srcCdPreset" data-v="${v}">${l}</button>`).join('')}</div>
    ${custom ? `<div class="grid" style="grid-template-columns:90px minmax(0,1fr); gap:10px 12px">
      <div class="lbl">Name</div><input class="field" style="width:100%" value="${esc(c.title)}" data-src-cd="title" placeholder="${c.preset === 'birthday' ? "Sam's birthday" : 'Launch day'}">
      <div class="lbl">Date</div><div style="display:flex; align-items:center; gap:8px"><input class="field" type="date" style="width:160px" value="${esc(c.date)}" data-src-cd="date">
        <input class="field" type="time" style="width:110px" value="${esc(c.time)}" data-src-cd="time"><span class="hint">optional</span></div>
      <div class="lbl">Every year</div><div style="display:flex; align-items:center; gap:10px">${tog(c.yearly, 'srcCdYearly')}<span class="hint">Rolls over to next year after the day</span></div>
    </div>` : `<div class="hint">Counts down to ${esc(PRESETS.find(p => p[0] === c.preset)[1])} every year, with a celebration card on the day.</div>`}
    <div class="hint">On the day it shows a celebration${c.preset === 'birthday' || /birthday/i.test(c.title) ? ' (“Happy birthday …”)' : ''}, once, as an alert.</div>
    <div style="display:flex; justify-content:flex-end; gap:8px">
      <button class="btn" style="font-size:13px; padding:5px 14px" data-act="srcCdCancel">Cancel</button>
      <button class="primary" data-act="srcCdAdd">Add countdown</button>
    </div>`);
}

// ---------------------------------------------------------------- import

function importText(text, from) {
  let v;
  try { v = JSON.parse(text); } catch (e) { return flash(`${from} isn't a plugin file (it isn't JSON)`); }
  return RB.invoke('import_plugin', { manifest: v })
    .then(r => {
      const p = r.plugin ? v : v.plugin;
      const cmd = (p && p.kind === 'command') || (v.kind === 'command');
      flash(r.source ? `Added ${v.source && v.source.name || 'the source'} to the rotation` : `Installed ${v.name}${cmd ? '. It runs a program, so it needs your approval before it does anything: Add it to review it.' : '. Add it from Add source.'}`, true);
      if (r.source) SR.sel = SR.open = r.source; else SR.picker = true;
      return srcRefresh().then(() => srcPreview(true));
    })
    .catch(e => flash(e));
}
function importFiles(files) {
  [...files].slice(0, 5).forEach(f => {
    if (f.size > 256 * 1024) return flash(`${f.name} is too large to be a plugin`);
    const r = new FileReader();
    r.onload = () => importText(String(r.result), f.name);
    r.readAsText(f);
  });
}

// ---------------------------------------------------------------- events

function addSource(id, settings) {
  return RB.invoke('add_source', { plugin: id, settings: settings || null })
    .then(nid => { SR.sel = nid; SR.open = nid; SR.picker = false; return srcRefresh(); })
    .then(() => srcPreview(true))
    .catch(e => flash(e));
}
const currentSched = () => { const x = inst(SR.open); return (x && x.schedule) || { days: [], from: null, to: null }; };

RB.delegate(root, 'click', {
  srcNext: () => RB.invoke('sources_next'),
  srcSelect: (el, e) => {
    if (e.target.closest('[data-act]') !== el) return;
    if (SR.sel !== el.dataset.v) { SR.sel = el.dataset.v; srcPreview(true); render(true); }
  },
  srcEdit: el => { const id = el.dataset.v; SR.open = SR.open === id ? null : id; if (SR.sel !== id) { SR.sel = id; srcPreview(true); } render(true); },
  srcEnabled: el => {
    const x = inst(el.dataset.v), p = x && plugin(x.plugin);
    if (x && !x.enabled && p && p.kind === 'command' && !p.approved) { SR.approve = { id: p.id, then: () => saveInst(x.id, { enabled: true }) }; return render(true); }
    if (x) saveInst(x.id, { enabled: !x.enabled });
  },
  srcPage: () => { SR.page = (SR.page + 1) % SR.preview.pages.length; srcShow(); render(true); },
  srcReplay: () => { srcBoard.setAttribute('seq', ++SR.pseq); },
  srcPreviewNow: el => srcRefreshNow(el.dataset.v),
  srcPicker: () => { SR.picker = !SR.picker; SR.linkConfirm = null; render(true); },
  srcImport: () => srcFile.click(),
  srcAdd: el => {
    const p = plugin(el.dataset.v); if (!p) return;
    if (p.id === 'rattleboard.countdown') { SR.countdown = { preset: 'birthday', title: '', date: '', time: '', yearly: true }; return render(true); }
    if (p.kind === 'command' && !p.approved) { SR.approve = { id: p.id, then: () => addSource(p.id) }; return render(true); }
    addSource(p.id);
  },
  srcUninstall: el => {
    const p = plugin(el.dataset.v), n = SR.data.rotation.filter(x => x.plugin === el.dataset.v).length;
    RB.invoke('remove_plugin', { id: el.dataset.v }).then(() => { flash(`Removed ${p.name}${n ? ` and its ${n} source${n > 1 ? 's' : ''}` : ''}`, true); return srcRefresh(); }).catch(e => flash(e));
  },
  srcExport: el => RB.invoke('export_source', { id: el.dataset.v }).then(path => flash(`Saved to ${path.replace(/^.*[\\/](Downloads[\\/])/, '$1')}`, true)).catch(e => flash(e)),
  srcRemove: el => {
    const x = inst(el.dataset.v);
    SR.open = null;
    saveRot(rot => rot.splice(rot.findIndex(r => r.id === x.id), 1));
    flash(`Removed ${x.label || x.name}`, true);
  },
  srcWeight: el => saveInst(SR.open, { weight: +el.dataset.v }),
  srcSetToggle: el => { const [id, key] = el.dataset.v.split('|'); const x = inst(id); saveSetting(id, key, !x.settings[key]); },
  srcTimed: el => { const x = inst(el.dataset.v); const timed = !!(x.schedule && (x.schedule.days.length || x.schedule.from)); saveInst(x.id, { schedule: timed ? null : { days: ['mon', 'tue', 'wed', 'thu', 'fri'], from: '07:00', to: '10:00' } }); },
  srcDay: el => { const s = currentSched(), d = el.dataset.v; saveInst(SR.open, { schedule: { ...s, days: s.days.includes(d) ? s.days.filter(x => x !== d) : [...s.days, d] } }); },
  srcPreset: el => {
    const v = el.dataset.v, x = inst(SR.open);
    const names = { nye: 'New Year', christmas: 'Christmas', halloween: 'Halloween' };
    if (names[v]) saveRot(rot => { const r = rot.find(r => r.id === x.id); Object.assign(r.settings, { preset: v, title: '' }); });
    else saveRot(rot => { const r = rot.find(r => r.id === x.id); Object.assign(r.settings, { preset: '', yearly: v === 'birthday' ? true : r.settings.yearly, title: v === 'birthday' && !/birthday/i.test(r.settings.title || '') ? 'Birthday' : r.settings.title }); });
  },
  srcReveal: el => {
    const k = el.dataset.v, [id, key] = k.split('|');
    if (SR.revealed[k] != null) { delete SR.revealed[k]; return render(true); }
    RB.invoke('reveal_source_secret', { instance: id, key }).then(v => { SR.revealed[k] = v; render(true); });
  },
  srcReplace: el => { SR.replacing[el.dataset.v] = true; delete SR.revealed[el.dataset.v]; render(true); const i = root.querySelector(`[data-src-secret="${CSS.escape(el.dataset.v)}"]`); if (i) i.focus(); },
  srcReplaceCancel: el => { delete SR.replacing[el.dataset.v]; delete SR.drafts[el.dataset.v]; render(true); },
  srcSecretSave: el => {
    const k = el.dataset.v, [id, key] = k.split('|'), i = root.querySelector(`[data-src-secret="${CSS.escape(k)}"]`);
    const value = (i ? i.value : SR.drafts[k] || '').trim();
    RB.invoke('set_source_secret', { instance: id, key, value })
      .then(() => { delete SR.replacing[k]; delete SR.drafts[k]; delete SR.revealed[k]; flash(value ? 'Saved to the credential store' : 'Cleared', true); return srcRefresh(); })
      .then(() => srcPreview(false))
      .catch(e => flash(e));
  },
  srcApprove: el => { SR.approve = { id: el.dataset.v, then: null }; render(true); },
  srcRevoke: el => RB.invoke('approve_plugin', { id: el.dataset.v, fingerprint: '', approve: false }).then(srcRefresh).catch(e => flash(e)),
  srcApproveNo: () => { SR.approve = null; render(true); },
  srcApproveYes: () => {
    const a = SR.approve, p = plugin(a.id); SR.approve = null;
    RB.invoke('approve_plugin', { id: p.id, fingerprint: p.fingerprint, approve: true })
      .then(() => (a.then ? a.then() : srcRefresh()))
      .then(() => { flash(`${p.name} is allowed to run`, true); })
      .catch(e => { flash(e); srcRefresh(); });
  },
  srcCdPreset: el => { SR.countdown.preset = el.dataset.v; if (el.dataset.v === 'birthday') SR.countdown.yearly = true; render(true); },
  srcCdYearly: () => { SR.countdown.yearly = !SR.countdown.yearly; render(true); },
  srcCdCancel: () => { SR.countdown = null; render(true); },
  srcCdAdd: () => {
    const c = SR.countdown, custom = c.preset === '' || c.preset === 'birthday';
    if (custom && (!c.title.trim() || !c.date)) return flash('Give the countdown a name and a date');
    SR.countdown = null;
    addSource('rattleboard.countdown', custom ? { preset: '', title: c.title.trim(), date: c.date, time: c.time, yearly: c.yearly } : { preset: c.preset });
  },
  srcLink: () => RB.invoke('read_plugin_link', { link: SR.link }).then(c => { SR.linkConfirm = c; render(true); }).catch(e => flash(e)),
  srcLinkNo: () => { SR.linkConfirm = null; render(true); },
  srcLinkYes: () => { const c = SR.linkConfirm; SR.linkConfirm = null; SR.link = ''; importText(JSON.stringify(c.manifest), 'The link'); },
  srcTarget: el => { const t = SR.data.targets || ['this'], v = el.dataset.v; saveCfg({ targets: t.includes(v) ? t.filter(x => x !== v) : [...t, v] }); },
  srcShuffle: el => saveCfg({ shuffle: el.dataset.v === 'shuffle' }),
  srcHold: el => saveCfg({ holdMinutes: +el.dataset.v }),
  srcAlert: el => saveCfg({ alertSeconds: +el.dataset.v }),
  srcQuiet: () => saveCfg({ quietHours: { ...SR.data.quietHours, enabled: !SR.data.quietHours.enabled } }),
  srcMirror: () => saveCfg({ showWhileMirroring: !SR.data.showWhileMirroring }),
  srcShowAnyway: () => saveCfg({ showWhileMirroring: true }),
  srcStandalone: () => { SR.confirmStandalone = true; render(true); },
  srcStandaloneNo: () => { SR.confirmStandalone = false; render(true); },
  srcStandaloneYes: () => { SR.confirmStandalone = false; save(sourcePatch('virtual')); },
  srcRunSetup: () => RB.invoke('open_window', { view: 'setup' }),
  srcClock24: () => saveCfg({ clock24: !SR.data.clock24 })
});

root.addEventListener('change', e => {
  const t = e.target, d = t.dataset;
  if (d.srcName) return saveInst(d.srcName, { name: t.value.trim() });
  if (d.srcSet) {
    const x = inst(d.inst), p = x && plugin(x.plugin), spec = p && (p.settings || []).find(s => s.key === d.srcSet);
    const v = spec && spec.type === 'lines' ? t.value.split('\n').map(l => l.trim()).filter(Boolean) : spec && spec.type === 'number' ? +t.value : t.value;
    return saveSetting(d.inst, d.srcSet, v);
  }
  if (d.srcDwell) return saveInst(d.srcDwell, { dwell: +t.value });
  if (d.srcChip != null) return saveInst(d.srcChip, { chip: t.value || null });
  if (d.srcSched) { const s = currentSched(); return saveInst(SR.open, { schedule: { ...s, [d.srcSched]: t.value || null } }); }
  if (d.srcWords) return saveInst(SR.open, { [d.srcWords]: t.value.split(',').map(w => w.trim()).filter(Boolean) });
  if (d.srcCfg) return saveCfg({ [d.srcCfg]: +t.value });
  if (d.srcQuiet) return saveCfg({ quietHours: { ...SR.data.quietHours, [d.srcQuiet]: t.value } });
  if (d.srcCd) { SR.countdown[d.srcCd] = t.value; return; }
});
root.addEventListener('input', e => {
  const d = e.target.dataset;
  if (d.srcSecret) SR.drafts[d.srcSecret] = e.target.value;
  if (d.srcLink) { SR.link = e.target.value; SR.linkConfirm = null; }
  if (d.srcCd) SR.countdown[d.srcCd] = e.target.value;
});
root.addEventListener('keydown', e => {
  if (e.key === 'Enter' && e.target.dataset && e.target.dataset.srcSecret) { e.preventDefault(); const b = root.querySelector(`[data-act="srcSecretSave"][data-v="${CSS.escape(e.target.dataset.srcSecret)}"]`); if (b) b.click(); }
}, true);

// Drag to reorder the rotation, and drop plugin files to import them.
let dragFrom = -1;
root.addEventListener('dragstart', e => {
  const g = e.target.closest && e.target.closest('.srcGrip');
  if (!g) return;
  dragFrom = +g.dataset.i; SR.dragging = true;
  e.dataTransfer.effectAllowed = 'move';
  e.dataTransfer.setData('text/plain', String(dragFrom));
  const row = g.closest('.srcRow'); if (row) e.dataTransfer.setDragImage(row, 20, 16);
});
const isFile = e => [...(e.dataTransfer ? e.dataTransfer.types : [])].includes('Files');
root.addEventListener('dragover', e => {
  if (L.view !== 'settings' || L.tab !== 'sources') return;
  if (isFile(e)) { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; if (!SR.drop) { SR.drop = true; const r = document.getElementById('srcRoot'); if (r) r.style.outline = `2px dashed ${RB.ACC}`; } return; }
  const row = e.target.closest && e.target.closest('.srcRow');
  if (dragFrom < 0 || !row) return;
  e.preventDefault();
  root.querySelectorAll('.srcRow').forEach(r => { r.style.boxShadow = ''; });
  const before = e.offsetY < row.offsetHeight / 2 || e.clientY < row.getBoundingClientRect().top + row.offsetHeight / 2;
  row.style.boxShadow = before ? `inset 0 2px 0 ${RB.ACC}` : `inset 0 -2px 0 ${RB.ACC}`;
  row.dataset.drop = before ? 'before' : 'after';
});
root.addEventListener('dragleave', e => {
  if (SR.drop && !e.relatedTarget) { SR.drop = false; const r = document.getElementById('srcRoot'); if (r) r.style.outline = ''; }
});
root.addEventListener('drop', e => {
  if (isFile(e)) { e.preventDefault(); SR.drop = false; const r = document.getElementById('srcRoot'); if (r) r.style.outline = ''; return importFiles(e.dataTransfer.files); }
  const row = e.target.closest && e.target.closest('.srcRow');
  if (dragFrom < 0 || !row) return;
  e.preventDefault();
  let to = +row.dataset.i + (row.dataset.drop === 'after' ? 1 : 0);
  const from = dragFrom; dragFrom = -1; SR.dragging = false;
  if (to > from) to--;
  if (to !== from) saveRot(rot => { const [m] = rot.splice(from, 1); rot.splice(to, 0, m); });
  else render(true);
});
root.addEventListener('dragend', () => { dragFrom = -1; SR.dragging = false; root.querySelectorAll('.srcRow').forEach(r => { r.style.boxShadow = ''; }); render(true); });
// Dropping a file outside the drop zone mustn't open it in the window.
window.addEventListener('dragover', e => { if (isFile(e)) e.preventDefault(); });
window.addEventListener('drop', e => { if (isFile(e)) e.preventDefault(); });
if (L.view === 'settings' && L.tab === 'sources') srcOpen();
