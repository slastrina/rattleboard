// The menu-bar / system-tray popover.
const menu = document.getElementById('menu');
let S = null, lastH = 0;

document.documentElement.classList.add(navigator.userAgent.includes('Mac') ? 'mac' : 'win');
const isMac = () => (S ? S.os === 'mac' : document.documentElement.classList.contains('mac'));

function checkedLabel(now) {
  if (!S.lastChecked) return 'not yet';
  const s = Math.floor((now - S.lastChecked) / 1000);
  return s <= 0 ? 'just now' : s < 60 ? s + 's ago' : RB.ago(now - S.lastChecked);
}

function render() {
  if (!S) return;
  const s = S.settings, now = Date.now(), mac = isMac(), fin = RB.fin(s.finish);
  // Pausing stops reading the mirrored board; a standalone (virtual) board has nothing to pause.
  const virt = s.source === 'virtual', paused = !s.polling && !virt;
  const connDot = paused ? '#9a9ca1' : S.conn === 'testing' ? '#e0a526' : S.conn === 'error' ? '#e0553f' : S.conn === 'ok' ? '#34b05a' : '#9a9ca1';
  const status = !s.setupDone ? 'Not set up yet'
    : paused ? 'Checking paused'
    : S.conn === 'error' ? (S.error || "Can't reach the board")
    : `Checked ${checkedLabel(now)} · changed ${S.lastChangeAt ? RB.ago(now - S.lastChangeAt) : '—'}`;
  const kbd = (m, w) => (mac ? m : w);
  // The board (or group) last sent to from Compose or Message History, for a one-click resend.
  const src = S.sources || {};
  const to = s.sendTarget !== 'this' && RB.targets(S).some(t => t[0] === s.sendTarget) && { name: RB.targetName(S, s.sendTarget) };
  const groups = [
    [['show', 'Show board', kbd('⌘B', 'Ctrl+B')], ...(src.active ? [['next', src.showing ? `Next source <span style="color:#8a8c91">· now ${RB.esc(src.showing)}</span>` : 'Next source', kbd('⌘→', 'Ctrl+→')]] : []), ['replay', 'Replay last change', ''], ...(to ? [['resend', `Send this to ${RB.esc(to.name)}`, '']] : [])],
    [['pin', 'Pin as widget', '', s.mode === 'float'], ['full', 'Full screen', kbd('⌃⌘F', 'F11'), s.mode === 'fullscreen'], ...(virt ? [] : [['pause', paused ? 'Resume checking' : 'Pause checking', '', false]])],
    [['compose', 'Compose…', kbd('⌘N', 'Ctrl+N')], ['history', 'Message history…', ''], ['settings', 'Settings…', kbd('⌘,', '')], ['setup', 'Run setup again…', '']],
    [['site', 'Rattleboard on iokig.com', ''], ['kofi', 'Support Rattleboard on Ko-fi', '']],
    [['quit', 'Quit Rattleboard', kbd('⌘Q', '')]]
  ];
  // A newer release (see updates.rs): first thing in the menu; it opens the release page.
  const up = S.update && S.update.available;
  menu.style.margin = mac ? '0 auto' : '0';
  menu.innerHTML = `
    ${paused && s.setupDone ? `<div class="update-group"><div class="item update" data-act="resume"><span class="chk">❙❙</span><span>Checking paused · Resume</span><span class="key"></span></div></div>` : ''}
    ${up ? `<div class="update-group"><div class="item update" data-act="update"><span class="chk">↑</span><span>Update available: v${RB.esc(up)}…</span><span class="key"></span></div></div>` : ''}
    <div style="padding:8px 10px 10px; display:flex; flex-direction:column; gap:8px">
      <div style="display:flex; justify-content:space-between; align-items:baseline">
        <span style="font-weight:600">${RB.esc(s.boardName || 'Rattleboard')}</span>
        <span style="font-size:11.5px; color:#6b6d72">${RB.esc(RB.sourceLabel(S))}</span>
      </div>
      <div style="display:flex; align-items:center; gap:6px; font-size:11.5px; color:#55575c">
        <div style="width:7px; height:7px; border-radius:50%; background:${connDot}; flex:none"></div><span style="white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${RB.esc(status)}</span>
      </div>
      <div data-act="show" style="background:${fin.frame}; padding:8px; border-radius:6px; display:flex; flex-direction:column; gap:3px; align-items:center; cursor:pointer">
        ${RB.mini(S.display, S)}
      </div>
      ${playing()}
    </div>
    ${groups.map(g => `<div class="group">${g.map(([act, label, key, on]) =>
      `<div class="item" data-act="${act}"><span class="chk">${on ? '✓' : ''}</span><span>${label}</span><span class="key">${key}</span></div>`).join('')}</div>`).join('')}`;
  const h = Math.ceil(menu.offsetHeight + 28);
  if (h !== lastH) { lastH = h; RB.invoke('tray_resize', { height: h }); }
}

// Sequences this computer is playing, each with a Stop button.
// A group's players show as one line, with one Stop for all of them.
function playing() {
  const lines = [];
  for (const p of S.playing || []) {
    const many = p.via === 'all' || p.via.startsWith('group:'), line = many && lines.find(l => l.via === p.via);
    if (line) line.count++;
    else lines.push({ ...p, count: 1, stop: many ? p.via : p.target, where: many ? p.viaName : p.target === 'this' ? '' : p.targetName });
  }
  const rows = lines.map(p => {
    const name = p.name ? `‘${RB.esc(p.name)}’` : 'sequence';
    const where = p.where ? ` <span style="color:#6b6d72">on ${RB.esc(p.where)}${p.count > 1 ? ` · ${p.count} boards` : ''}</span>` : '';
    return `<div style="display:flex; align-items:center; gap:8px; font-size:12px; padding:5px 6px 5px 8px; border-radius:6px; background:rgba(0,0,0,.05)">
      <div style="width:0; height:0; border-left:7px solid ${RB.ACC}; border-top:4.5px solid transparent; border-bottom:4.5px solid transparent; flex:none"></div>
      <span style="flex:1; min-width:0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">Playing ${name} ${p.frame}/${p.frames}${p.loop ? ' · loop' : ''}${where}</span>
      <button data-act="stopSeq" data-v="${RB.esc(p.stop)}" style="border:1px solid rgba(0,0,0,.15); background:#fff; border-radius:5px; font:inherit; font-size:11.5px; padding:2px 9px; cursor:pointer">Stop</button>
    </div>`;
  }).join('');
  const err = !rows && S.playError ? `<div style="font-size:11.5px; color:#c0392b; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">Sequence stopped · ${RB.esc(S.playError)}</div>` : '';
  return rows + err;
}

const act = {
  show: () => RB.invoke('show_board'),
  next: () => RB.invoke('sources_next'),
  replay: () => RB.invoke('replay'),
  resend: () => { RB.invoke('send_message', { target: S.settings.sendTarget, message: { characters: S.display } }).catch(() => {}); RB.invoke('close_window', { label: 'tray' }); },
  pin: () => { RB.invoke('update_settings', { patch: { mode: S.settings.mode === 'float' ? 'toast' : 'float' } }); RB.invoke('show_board'); },
  full: () => RB.invoke('toggle_fullscreen'),
  pause: () => { RB.invoke('update_settings', { patch: { polling: !S.settings.polling } }); RB.invoke('close_window', { label: 'tray' }); },
  compose: () => RB.invoke('open_window', { view: 'compose' }),
  history: () => RB.invoke('open_window', { view: 'history' }),
  settings: () => RB.invoke('open_window', { view: 'settings' }),
  setup: () => RB.invoke('open_window', { view: 'setup' }),
  site: () => { RB.invoke('open_site'); RB.invoke('close_window', { label: 'tray' }); },
  kofi: () => { RB.invoke('open_kofi'); RB.invoke('close_window', { label: 'tray' }); },
  update: () => { RB.invoke('open_update'); RB.invoke('close_window', { label: 'tray' }); },
  resume: () => RB.invoke('update_settings', { patch: { polling: true } }),
  quit: () => RB.invoke('quit')
};
RB.delegate(menu, 'click', Object.fromEntries(Object.entries(act).map(([k, f]) => [k, () => f()])));
RB.delegate(menu, 'click', { stopSeq: el => RB.invoke('stop_sequence', { target: el.dataset.v }) });

document.addEventListener('keydown', e => {
  const mod = e.metaKey || e.ctrlKey, k = e.key.toLowerCase();
  if (e.key === 'Escape') RB.invoke('close_window', { label: 'tray' });
  else if (e.key === 'F11') { e.preventDefault(); act.full(); }
  else if (mod && k === 'b') act.show();
  else if (mod && e.key === 'ArrowRight' && S && S.sources && S.sources.active) act.next();
  else if (mod && k === ',') act.settings();
  else if (mod && k === 'n') act.compose();
  else if (mod && k === 'q') act.quit();
});

RB.onState(snap => { S = snap; render(); });
setInterval(render, 1000);
