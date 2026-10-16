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
  const connDot = !s.polling ? '#9a9ca1' : S.conn === 'testing' ? '#e0a526' : S.conn === 'error' ? '#e0553f' : S.conn === 'ok' ? '#34b05a' : '#9a9ca1';
  const status = !s.setupDone ? 'Not set up yet'
    : !s.polling ? 'Checking paused'
    : S.conn === 'error' ? (S.error || "Can't reach the board")
    : `Checked ${checkedLabel(now)} · changed ${S.lastChangeAt ? RB.ago(now - S.lastChangeAt) : '—'}`;
  const kbd = (m, w) => (mac ? m : w);
  const groups = [
    [['show', 'Show board', kbd('⌘B', 'Ctrl+B')], ['replay', 'Replay last change', '']],
    [['pin', 'Pin as widget', '', s.pinned], ['pause', 'Pause checking', '', !s.polling]],
    [['history', 'Message history…', ''], ['settings', 'Settings…', kbd('⌘,', '')], ['setup', 'Run setup again…', '']],
    [['quit', 'Quit Rattleboard', kbd('⌘Q', '')]]
  ];
  menu.style.margin = mac ? '0 auto' : '0';
  menu.innerHTML = `
    <div style="padding:8px 10px 10px; display:flex; flex-direction:column; gap:8px">
      <div style="display:flex; justify-content:space-between; align-items:baseline">
        <span style="font-weight:600">${RB.esc(s.boardName || 'Vestaboard')}</span>
        <span style="font-size:11.5px; color:#6b6d72">${RB.esc(s.source === 'local' ? 'Local · ' + s.host : s.source === 'demo' ? 'Demo mode' : 'Cloud')}</span>
      </div>
      <div style="display:flex; align-items:center; gap:6px; font-size:11.5px; color:#55575c">
        <div style="width:7px; height:7px; border-radius:50%; background:${connDot}; flex:none"></div><span style="white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${RB.esc(status)}</span>
      </div>
      <div data-act="show" style="background:${fin.frame}; padding:8px; border-radius:6px; display:flex; flex-direction:column; gap:3px; align-items:center; cursor:pointer">
        ${RB.mini(S.display, S)}
      </div>
    </div>
    ${groups.map(g => `<div class="group">${g.map(([act, label, key, on]) =>
      `<div class="item" data-act="${act}"><span class="chk">${on ? '✓' : ''}</span><span>${label}</span><span class="key">${key}</span></div>`).join('')}</div>`).join('')}`;
  const h = Math.ceil(menu.offsetHeight + 28);
  if (h !== lastH) { lastH = h; RB.invoke('tray_resize', { height: h }); }
}

const act = {
  show: () => RB.invoke('show_board'),
  replay: () => RB.invoke('replay'),
  pin: () => { RB.invoke('update_settings', { patch: { pinned: !S.settings.pinned } }); RB.invoke('show_board'); },
  pause: () => { RB.invoke('update_settings', { patch: { polling: !S.settings.polling } }); RB.invoke('close_window', { label: 'tray' }); },
  history: () => RB.invoke('open_window', { view: 'history' }),
  settings: () => RB.invoke('open_window', { view: 'settings' }),
  setup: () => RB.invoke('open_window', { view: 'setup' }),
  quit: () => RB.invoke('quit')
};
RB.delegate(menu, 'click', Object.fromEntries(Object.entries(act).map(([k, f]) => [k, () => f()])));

document.addEventListener('keydown', e => {
  const mod = e.metaKey || e.ctrlKey, k = e.key.toLowerCase();
  if (e.key === 'Escape') RB.invoke('close_window', { label: 'tray' });
  else if (mod && k === 'b') act.show();
  else if (mod && k === ',') act.settings();
  else if (mod && k === 'q') act.quit();
});

RB.onState(snap => { S = snap; render(); });
setInterval(render, 1000);
