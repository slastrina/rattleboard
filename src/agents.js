// Settings → Agents: connect AI agent apps (Claude Code, Cursor, Codex, …) so they can put
// things on the board as MCP tool calls. panel.js draws it as one of its tabs (viewAgents);
// the config edits themselves are in src-tauri/src/agents/. Every change is previewed first
// and needs Confirm; a backup of each edited file is kept next to it.
const AG = {
  data: null, loading: false,
  // { id, name, mcp, alerts, files, token } while a preview waits for Confirm
  preview: null, busy: false,
  // { id, ok, text } after a change, shown under that app
  msg: null,
  // The default target picked for an app that isn't connected yet
  targets: {}
};

// Where tool calls that name no board go: this board, a saved board, a group or all.
const agTargets = () => [['this', 'This board'],
  ...(S.boards || []).map(b => [b.id, b.name]),
  ...(S.groups || []).map(g => ['group:' + g.id, g.name + ' (group)']),
  ...((S.boards || []).length ? [['all', 'All boards']] : [])];

function agentsOpen() {
  if (AG.loading) return;
  AG.loading = true;
  RB.invoke('agents_list')
    .then(d => { AG.data = d; })
    .catch(e => { AG.msg = { id: '', ok: false, text: String(e) }; })
    .finally(() => { AG.loading = false; render(true); });
}

function agentsAsk(id, mcp, alerts, target) {
  const h = AG.data.harnesses.find(x => x.id === id);
  AG.msg = null; AG.busy = true; render(true);
  RB.invoke('agents_preview', { id, mcp, alerts, target: target || null })
    .then(p => {
      if (!p.files.length) { AG.msg = { id, ok: true, text: 'Nothing to change: it’s already set up that way.' }; return; }
      AG.preview = { id, name: h.name, mcp, alerts, target: target || null, files: p.files, token: p.token };
    })
    .catch(e => { AG.msg = { id, ok: false, text: String(e) }; })
    .finally(() => { AG.busy = false; render(true); });
}

function agentsConfirm() {
  const p = AG.preview; if (!p) return;
  AG.busy = true; render(true);
  RB.invoke('agents_apply', { id: p.id, mcp: p.mcp, alerts: p.alerts, target: p.target, token: p.token })
    .then(d => {
      AG.data = d; AG.preview = null;
      const h = d.harnesses.find(x => x.id === p.id);
      AG.msg = { id: p.id, ok: true, text: p.mcp ? `Done. ${h ? h.note : ''}` : `Disconnected. ${p.name} no longer has Rattleboard.` };
    })
    .catch(e => { AG.msg = { id: p.id, ok: false, text: String(e) }; })
    .finally(() => { AG.busy = false; render(true); });
}

function agentsDiff(lines) {
  const col = l => (l[0] === '+' ? '#9fd3a8' : l[0] === '-' ? '#f0a39a' : '#8e9096');
  return `<div class="mono" style="background:#1d1e21; border-radius:6px; padding:8px 10px; font-size:11px; line-height:1.55; max-height:220px; overflow:auto; user-select:text">
    ${lines.map(l => `<div style="color:${col(l)}; white-space:pre">${esc(l)}</div>`).join('') || '<div style="color:#8e9096">(empty)</div>'}</div>`;
}

function agentsPreviewBox() {
  const p = AG.preview;
  const verb = p.mcp ? (p.alerts ? 'connects with alerts' : 'connects') : 'disconnects';
  const act = { create: 'New file', edit: 'Edit', delete: 'Remove' };
  return `<div style="grid-column:1 / -1; background:#fff; border-radius:10px; padding:14px; box-shadow:0 0 0 1px rgba(0,0,0,.1); display:flex; flex-direction:column; gap:10px">
    <div style="font-weight:600">Review: this ${verb} ${esc(p.name)}</div>
    <div class="hint" style="line-height:1.45">Each file you already have is backed up next to it first (<span class="mono">….rattleboard-backup-<i>date</i></span>). Nothing else in it changes.</div>
    ${p.files.map(f => `<div style="display:flex; flex-direction:column; gap:4px; min-width:0">
      <div style="font-size:12px"><b>${act[f.action] || f.action}</b> <span class="mono" style="font-size:11.5px; user-select:text; word-break:break-all">${esc(f.path)}</span></div>
      ${agentsDiff(f.diff)}</div>`).join('')}
    <div style="display:flex; gap:8px; justify-content:flex-end">
      <button class="btn" data-act="agCancel" ${AG.busy ? 'disabled' : ''}>Cancel</button>
      <button class="primary" data-act="agConfirm" ${AG.busy ? 'disabled' : ''}>Confirm</button>
    </div>
  </div>`;
}

function agentsRow(h) {
  const dot = h.connected ? '#34b05a' : '#9a9ca1';
  const status = h.connected ? 'Connected' : 'Not connected';
  const busy = AG.busy || !!AG.preview;
  const msg = AG.msg && AG.msg.id === h.id
    ? `<div class="hint" style="grid-column:1 / -1; color:${AG.msg.ok ? '#2f7d46' : '#c0392b'}; line-height:1.45">${esc(AG.msg.text)}</div>` : '';
  const alerts = h.alertsSupported && h.connected
    ? `<div style="grid-column:1 / -1; display:flex; align-items:center; gap:10px; padding-left:18px">
        ${h.alertsBlocked && !h.alertsOn ? `<span class="hint">${esc(h.alertsBlocked)}</span>`
          : `<div class="tog ${h.alertsOn ? 'on' : ''}" data-act="agAlerts" data-v="${h.id}" style="${busy ? 'pointer-events:none; opacity:.5' : ''}"><div></div></div>
             <span class="hint">Alert me when the agent finishes or needs input</span>`}
      </div>` : '';
  const opts = agTargets(), cur = h.connected ? (h.target || 'this') : (AG.targets[h.id] || 'this');
  const known = opts.some(o => o[0] === cur);
  const picker = opts.length > 1 || cur !== 'this'
    ? `<div style="grid-column:1 / -1; display:flex; align-items:center; gap:10px; padding-left:18px">
        <span class="hint">Sends to</span>
        <select class="field" data-agtarget="${h.id}" ${busy ? 'disabled' : ''} title="Where messages go when the agent doesn't name a board">
          ${opts.map(([v, l]) => `<option value="${esc(v)}" ${v === cur ? 'selected' : ''}>${esc(l)}</option>`).join('')}
          ${known ? '' : `<option value="${esc(cur)}" selected>${esc(cur)} (removed)</option>`}
        </select>
      </div>` : '';
  return `<div style="display:grid; grid-template-columns:minmax(0,1fr) auto; gap:6px 12px; align-items:center; padding:10px 0; border-top:1px solid rgba(0,0,0,.07)">
    <div style="display:flex; align-items:center; gap:8px; min-width:0">
      <div class="dot" style="background:${dot}"></div>
      <div style="display:flex; flex-direction:column; gap:2px; min-width:0">
        <span style="font-weight:600">${esc(h.name)} <span class="hint" style="font-weight:400">· ${status}${h.skill && h.connected ? ' · skill installed' : ''}</span></span>
        <span class="mono hint" style="font-size:11px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; user-select:text" title="${esc(h.config)}">${esc(h.config)}</span>
      </div>
    </div>
    ${h.connected
      ? `<button class="btn" data-act="agDisconnect" data-v="${h.id}" ${busy ? 'disabled' : ''}>Disconnect</button>`
      : `<button class="btn" data-act="agConnect" data-v="${h.id}" ${busy ? 'disabled' : ''}>Connect</button>`}
    ${picker}${alerts}${msg}
  </div>`;
}

function viewAgents() {
  if (!AG.data) return `<div class="hint">${AG.loading ? 'Looking for agent apps…' : ''}${AG.msg ? esc(AG.msg.text) : ''}</div>`;
  const found = AG.data.harnesses.filter(h => h.installed || h.connected);
  const missing = AG.data.harnesses.filter(h => !h.installed && !h.connected);
  const s = S.settings;
  return `<div style="display:flex; flex-direction:column; gap:12px">
    <div style="color:#55575c; line-height:1.5; text-wrap:pretty">AI agent apps can put things on this board as tool calls (MCP): “Tests passed”, “Need your input”. Connecting adds Rattleboard to the app’s settings file, shows you the change first, and keeps a backup.</div>
    ${AG.data.warning ? `<div class="hint" style="color:#b25d00; line-height:1.45">${esc(AG.data.warning)}</div>` : ''}
    ${!s.serverEnabled ? `<div class="hint" style="line-height:1.45">Connecting turns on the API server (this computer only), which agents use to reach the board.</div>` : ''}
    ${AG.preview ? agentsPreviewBox() : ''}
    <div>
      ${found.length ? found.map(agentsRow).join('') : `<div class="hint" style="padding:10px 0">No agent apps found on this computer.</div>`}
      ${AG.msg && !AG.msg.id ? `<div class="hint" style="color:#c0392b">${esc(AG.msg.text)}</div>` : ''}
    </div>
    ${missing.length ? `<div class="hint" style="line-height:1.45">Not found: ${missing.map(h => esc(h.name)).join(', ')}. Install one and come back, or see “Agents (MCP)” in the README to set it up by hand.</div>` : ''}
    <div class="hint" style="line-height:1.45; border-top:1px solid rgba(0,0,0,.07); padding-top:10px">Agents start <span class="mono" style="font-size:11px; user-select:text">${esc(AG.data.exe)} --mcp</span>, which reaches the board through the API server on this computer. The API key stays in your keychain and never goes into an agent’s settings. Agents can only write to the board and read what it shows.</div>
  </div>`;
}

RB.delegate(root, 'click', {
  agConnect: el => {
    const h = AG.data.harnesses.find(x => x.id === el.dataset.v);
    agentsAsk(h.id, true, h.alertsOn, AG.targets[h.id]);
  },
  agDisconnect: el => agentsAsk(el.dataset.v, false, false),
  agAlerts: el => {
    const h = AG.data.harnesses.find(x => x.id === el.dataset.v);
    agentsAsk(h.id, true, !h.alertsOn, h.target);
  },
  agCancel: () => { AG.preview = null; render(true); },
  agConfirm: agentsConfirm
});

root.addEventListener('change', e => {
  const id = e.target.dataset && e.target.dataset.agtarget;
  if (!id || !AG.data) return;
  const h = AG.data.harnesses.find(x => x.id === id), t = e.target.value === 'this' ? null : e.target.value;
  // A connected app's config is changed (with a preview); otherwise it's used when connecting.
  if (h.connected) agentsAsk(id, true, h.alertsOn, t);
  else { AG.targets[id] = t; render(true); }
});
