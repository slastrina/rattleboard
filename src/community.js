// Settings → Sources → Community plugins: browse github.com/slastrina/rattleboard-plugins,
// read about a plugin, see its preview card, and install it through the same confirm step as
// a plugin link. sources.js draws this in place of the Sources tab while CM.open; the
// fetching (only from that repository) and the day-long cache are in src-tauri/src/community.rs.
const CM = { open: false, data: null, loading: false, error: null, q: '', tag: '', id: null, detail: {}, confirm: null, busy: null, msg: null, added: null };
const CM_TAGS = ['news', 'weather', 'sport', 'finance', 'transport', 'space', 'calendar', 'fun', 'home', 'tech', 'other'];
const cmBoard = document.createElement('split-flap-board');
cmBoard.id = 'cmBoard';

const cmEntry = id => (CM.data ? CM.data.plugins.find(p => p.id === id) : null);
const cmA = (act, v, text, title) => `<a href="#" data-act="${act}" data-v="${esc(v)}" ${title ? `title="${esc(title)}"` : ''} style="color:${RB.ACC}; text-decoration:none">${esc(text)}</a>`;
const cmBadge = (text, bg, fg) => `<span style="font-size:10.5px; font-weight:600; padding:1px 7px; border-radius:8px; background:${bg}; color:${fg}; white-space:nowrap">${esc(text)}</span>`;
const cmState = p => (p.update ? cmBadge('Update available', '#fff1cc', '#8a5a00') : p.installed ? cmBadge('Installed', '#e2f3e7', '#2f8a4c') : '');
const cmKey = p => (p.needsSecrets ? cmBadge('Needs a key', '#eef0f3', '#55575c') : '');
const cmBox = inner => `<div style="display:flex; flex-direction:column; border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); overflow:hidden">${inner}</div>`;
const cmSizeKey = id => { const z = RB.sizeOf(S.settings); return `${id}@${z.rows}x${z.cols}`; };

function cmLoad(refresh) {
  CM.loading = true; CM.error = null; render(true);
  return RB.invoke('community_index', { refresh: !!refresh })
    .then(d => { CM.data = d; })
    .catch(e => { CM.error = String(e); if (refresh) CM.data = null; })
    .finally(() => { CM.loading = false; render(true); });
}
function cmOpen() {
  Object.assign(CM, { open: true, id: null, confirm: null, msg: null, added: null });
  SR.picker = false;
  render(true);
  cmLoad(false);
}
function cmShow(id) {
  Object.assign(CM, { id, confirm: null, msg: null });
  render(true);
  const k = cmSizeKey(id);
  if (CM.detail[k] && !CM.detail[k].error) return;
  CM.detail[k] = { loading: true };
  RB.invoke('community_detail', { id })
    .then(d => { CM.detail[k] = d; })
    .catch(e => { CM.detail[k] = { error: String(e) }; })
    .finally(() => render(true));
}

// ---------------------------------------------------------------- the list

function cmMatches(p) {
  if (CM.tag && !p.tags.includes(CM.tag)) return false;
  const hay = [p.name, p.summary, p.tags.join(' '), p.author.name, KINDS[p.kind] || p.kind].join(' ').toLowerCase();
  return CM.q.toLowerCase().split(/\s+/).filter(Boolean).every(w => hay.includes(w));
}
function cmRows() {
  const list = CM.data.plugins.filter(cmMatches).sort((a, b) => a.name.localeCompare(b.name));
  if (!list.length) return `<div class="hint" style="padding:14px">${CM.data.plugins.length ? 'No plugins match.' : 'No plugins in the directory yet.'}</div>`;
  return list.map((p, i) => `<div class="cmRow" role="button" tabindex="0" data-act="cmShow" data-v="${esc(p.id)}" style="display:flex; align-items:center; gap:12px; padding:9px 14px; cursor:pointer; border-top:${i ? '1px solid rgba(0,0,0,.06)' : '0'}">
      <div style="display:flex; flex-direction:column; gap:3px; flex:1; min-width:0">
        <span style="display:flex; align-items:center; gap:6px; min-width:0"><b style="font-weight:600; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(p.name)}</b>${cmKey(p)}${cmState(p)}</span>
        <span style="font-size:11.5px; color:#55575c; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(p.summary)}</span>
        <span class="hint" style="font-size:11px">${esc(p.author.name || p.author.github || 'Unknown')} · ${esc(KINDS[p.kind] || p.kind)}</span>
      </div>
      <span style="color:#a0a2a7; font-size:15px">›</span>
    </div>`).join('');
}
function cmHeader(back, title) {
  const d = CM.data;
  return `<div style="display:flex; align-items:center; gap:10px">
      <button class="btn" data-act="${back}">‹ ${back === 'cmClose' ? 'Sources' : 'All plugins'}</button>
      <span style="font-weight:600; font-size:14px; flex:1; min-width:0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis">${esc(title)}</span>
      ${d && d.fetched ? `<span class="hint">Updated ${esc(clock(d.fetched))}</span>` : ''}
      <button class="btn" data-act="cmRefresh" ${CM.loading ? 'disabled' : ''}>${CM.loading ? 'Loading…' : 'Refresh'}</button>
    </div>`;
}
const cmFooter = () => `<div class="hint" style="display:flex; gap:6px; flex-wrap:wrap; align-items:center; padding-top:2px">
    ${cmA('cmGitHub', 'repo', 'Browse on GitHub')}<span>·</span>${cmA('cmGitHub', 'contribute', 'Contribute a plugin')}
    <span style="flex-basis:100%; text-wrap:pretty">Made by people in the community and reviewed before they're listed. Each one is shown to you before it's installed, and it only fetches from the addresses it lists.</span></div>`;

function viewCommunity() {
  const d = CM.data, p = CM.id && cmEntry(CM.id);
  if (p) return viewCommunityDetail(p);
  let body;
  if (!d && CM.error) {
    body = `<div style="display:flex; flex-direction:column; align-items:center; gap:10px; padding:28px 12px; text-align:center">
      <span>Couldn't load community plugins. Check your connection.</span>
      <span class="hint" style="text-wrap:pretty">${esc(CM.error.replace(/^Couldn't load community plugins\. Check your connection\.\s*/, '').replace(/^\((.*)\)$/, '$1'))}</span>
      <button class="btn" data-act="cmRetry">Retry</button></div>`;
  } else if (!d) {
    body = `<div class="hint" style="padding:28px 0; text-align:center">Loading community plugins…</div>`;
  } else {
    const tags = CM_TAGS.filter(t => d.plugins.some(p => p.tags.includes(t)));
    body = `${d.warning ? `<div style="font-size:12px; color:#8a5a00">${esc(d.warning)}</div>` : ''}
      ${CM.error ? `<div style="font-size:12px; color:#c0392b">${esc(CM.error)}</div>` : ''}
      <input class="field" style="width:100%" placeholder="Search by name, description or tag" value="${esc(CM.q)}" data-cm-q="1" spellcheck="false" autocomplete="off">
      <div style="display:flex; flex-wrap:wrap; gap:6px">
        <button class="chip ${CM.tag ? '' : 'on'}" data-act="cmTag" data-v="">All</button>
        ${tags.map(t => `<button class="chip ${CM.tag === t ? 'on' : ''}" data-act="cmTag" data-v="${esc(t)}">${esc(t.charAt(0).toUpperCase() + t.slice(1))}</button>`).join('')}
      </div>
      ${cmBox(`<div id="cmList">${cmRows()}</div>`)}`;
  }
  return `<div style="display:flex; flex-direction:column; gap:12px" id="cmRoot">
    ${cmHeader('cmClose', 'Community plugins')}
    ${body}
    ${cmFooter()}
  </div>`;
}

// ---------------------------------------------------------------- one plugin

function viewCommunityDetail(p) {
  const det = CM.detail[cmSizeKey(p.id)] || { loading: true }, z = RB.sizeOf(S.settings);
  const rows = det.rows || z.rows, cols = det.cols || z.cols;
  const author = p.author.github ? cmA('cmGitHub', 'author:' + p.author.github, p.author.name || p.author.github, `github.com/${p.author.github}`) : esc(p.author.name || 'Unknown');
  const line = (label, html) => `<div class="lbl">${label}</div><div style="min-width:0; overflow-wrap:anywhere">${html}</div>`;
  const fieldList = (list, none) => (list.length ? list.map(f => esc(f.label || f.key)).join(', ') : `<span class="hint">${none}</span>`);
  const installedText = p.installed && p.installedVersion && p.installedVersion !== p.version ? ` <span class="hint">(${esc(p.installedVersion)} installed)</span>` : '';
  const action = CM.busy === p.id ? `<button class="primary" disabled>Checking…</button>`
    : p.update ? `<button class="primary" data-act="cmInstall" data-v="${esc(p.id)}">Update to ${esc(p.version)}</button>`
    : p.installed ? `<button class="btn" disabled style="color:#2f8a4c; font-size:13px; padding:5px 14px">Installed ✓</button>`
    : `<button class="primary" data-act="cmInstall" data-v="${esc(p.id)}">Install</button>`;
  const addRot = p.installed ? `<button class="${CM.added === p.id ? 'primary' : 'btn'}" ${CM.added === p.id ? '' : 'style="font-size:13px; padding:5px 14px"'} data-act="cmAddRot" data-v="${esc(p.id)}">Add to rotation</button>${p.inRotation ? `<span class="hint">In the rotation${p.inRotation > 1 ? ` ×${p.inRotation}` : ''}</span>` : ''}` : '';
  const preview = det.preview
    ? `<div id="cmPreview" style="min-height:${srcPreviewHeight(rows, cols)}px; display:flex; align-items:center; justify-content:center"></div>`
    : `<div class="hint" style="padding:18px 0">${det.loading ? 'Loading the preview…' : det.previewError ? `No preview: ${esc(det.previewError)}` : det.error ? esc(det.error) : 'No preview card for this plugin. Install it to see it live.'}</div>`;
  return `<div style="display:flex; flex-direction:column; gap:12px" id="cmRoot">
    ${cmHeader('cmBack', p.name)}
    <div style="display:flex; align-items:center; gap:6px; flex-wrap:wrap"><span style="text-wrap:pretty">${esc(p.summary)}</span>${cmKey(p)}${cmState(p)}</div>
    <div style="display:flex; justify-content:center; padding:12px 0; border-radius:10px; background:rgba(0,0,0,.05); overflow:hidden">${preview}</div>
    ${det.preview ? `<div class="hint" style="margin-top:-6px; text-align:center">A sample card, at this board's size (${rows} × ${cols}).</div>` : ''}
    <div style="display:flex; align-items:center; gap:8px; flex-wrap:wrap">${action}${addRot}<div style="flex:1"></div>${p.folderUrl ? cmA('cmGitHub', 'folder:' + p.id, 'View on GitHub') : ''}</div>
    ${CM.confirm && CM.confirm.id === p.id ? linkConfirmBox(CM.confirm, 'cmInstallYes', 'cmInstallNo', p.installed ? 'Update' : 'Install') : ''}
    ${CM.msg ? `<div style="font-size:12px; color:${CM.msg.ok ? '#2f8a4c' : '#c0392b'}; text-wrap:pretty">${esc(CM.msg.text)}</div>` : ''}
    <div class="grid" style="grid-template-columns:110px minmax(0,1fr); gap:8px 14px">
      ${line('Author', author)}
      ${line('Version', esc(p.version) + installedText + (p.updated ? ` <span class="hint">· ${esc(p.updated)}</span>` : ''))}
      ${line('License', esc(p.license || 'MIT'))}
      ${line('Kind', esc(KINDS[p.kind] || p.kind))}
      ${line('Tags', p.tags.map(t => `<span class="chip" style="cursor:default; padding:1px 8px; font-size:11px">${esc(t)}</span>`).join(' '))}
      ${line('Contacts', p.hosts.length ? p.hosts.map(h => `<span class="mono" style="font-size:11.5px">${esc(h)}</span>`).join(', ') : '<span class="hint">Nothing; it makes its card on this computer</span>')}
      ${line('Settings', fieldList(p.settings, 'None'))}
      ${line('Secrets', p.secrets.length ? fieldList(p.secrets, '') + ' <span class="hint">· kept in ' + esc(RB.keychain(S)) + '</span>' : '<span class="hint">None</span>')}
      ${p.homepage ? line('Homepage', `<span class="mono" style="font-size:11.5px; user-select:text">${esc(p.homepage.replace(/^https:\/\//, ''))}</span>`) : ''}
    </div>
    ${det.readme ? `<div style="border-radius:8px; background:#fff; box-shadow:0 0 0 1px rgba(0,0,0,.1); padding:12px 16px; line-height:1.5; user-select:text; -webkit-user-select:text; overflow-wrap:anywhere">${cmMarkdown(det.readme)}</div>` : det.readmeError ? `<div class="hint">Couldn't load its README: ${esc(det.readmeError)}</div>` : ''}
    ${cmFooter()}
  </div>`;
}

function communityAfter() {
  const slot = document.getElementById('cmPreview'), det = CM.id && CM.detail[cmSizeKey(CM.id)];
  if (!slot || !det || !det.preview) return;
  slot.appendChild(cmBoard);
  const s = S.settings, set = (k, v) => { if (cmBoard.getAttribute(k) !== String(v)) cmBoard.setAttribute(k, v); };
  set('rows', det.rows); set('cols', det.cols); set('tile', srcTile(det.cols)); set('finish', s.finish); set('speed', s.speed); set('sound', 'off');
  set('message', JSON.stringify(det.preview));
}

// A README as simple formatted text: headings, paragraphs, lists, code, bold and italics.
// Links show their text and address but aren't followed; images show their description.
function cmMarkdown(md) {
  const inline = t => {
    const code = [];
    t = esc(t).replace(/`([^`]+)`/g, (_, c) => { code.push(c); return `\u0000${code.length - 1}\u0000`; });
    t = t.replace(/!\[([^\]]*)\]\([^)]*\)/g, (_, a) => (a ? `[${a}]` : ''))
      .replace(/\[([^\]]+)\]\(([^)\s]+)[^)]*\)/g, (_, a, u) => (a === u ? `<span class="mono" style="font-size:11.5px">${u}</span>` : `${a} <span class="hint">(${u})</span>`))
      .replace(/\*\*([^*]+)\*\*/g, '<b style="font-weight:600">$1</b>').replace(/__([^_]+)__/g, '<b style="font-weight:600">$1</b>')
      .replace(/(^|[\s(])\*([^*\s][^*]*)\*/g, '$1<i>$2</i>').replace(/(^|[\s(])_([^_\s][^_]*)_/g, '$1<i>$2</i>');
    return t.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code class="mono" style="font-size:11.5px; background:rgba(0,0,0,.05); padding:0 3px; border-radius:3px">${code[i]}</code>`);
  };
  const out = [], lines = String(md).replace(/\r\n?/g, '\n').split('\n');
  let para = [], list = null;
  const flush = () => {
    if (para.length) { out.push(`<p style="margin:0 0 8px">${inline(para.join(' '))}</p>`); para = []; }
    if (list) { out.push(`<${list.tag} style="margin:0 0 8px; padding-left:20px">${list.items.map(i => `<li>${inline(i)}</li>`).join('')}</${list.tag}>`); list = null; }
  };
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    if (/^\s*```/.test(l)) {
      flush();
      const code = [];
      while (++i < lines.length && !/^\s*```/.test(lines[i])) code.push(lines[i]);
      out.push(`<pre class="mono" style="font-size:11.5px; background:rgba(0,0,0,.05); padding:8px 10px; border-radius:6px; margin:0 0 8px; white-space:pre-wrap">${esc(code.join('\n'))}</pre>`);
      continue;
    }
    let m;
    if ((m = l.match(/^\s*(#{1,6})\s+(.*)$/))) { flush(); out.push(`<div style="font-weight:600; font-size:${m[1].length <= 2 ? 14 : 13}px; margin:${out.length ? 6 : 0}px 0 6px">${inline(m[2].replace(/\s#+$/, ''))}</div>`); continue; }
    if ((m = l.match(/^\s*([-*+]|\d+[.)])\s+(.*)$/))) {
      const tag = /\d/.test(m[1]) ? 'ol' : 'ul';
      if (para.length || (list && list.tag !== tag)) flush();
      if (!list) list = { tag, items: [] };
      list.items.push(m[2]);
      continue;
    }
    if (!l.trim()) { flush(); continue; }
    if (/^\s*(---+|\*\*\*+)\s*$/.test(l)) { flush(); continue; }
    if (list && /^\s+\S/.test(l)) { list.items[list.items.length - 1] += ' ' + l.trim(); continue; }
    if (list) flush();
    para.push(l.trim().replace(/^>\s?/, ''));
  }
  flush();
  return out.join('');
}

// ---------------------------------------------------------------- events

function cmAfterInstall(p, verb) {
  return RB.invoke('community_installed', { id: p.id }).catch(() => {})
    .then(() => RB.invoke('community_index', { refresh: false })).then(d => { CM.data = d; }).catch(() => {})
    .then(() => {
      const now = cmEntry(p.id) || p;
      CM.added = now.inRotation ? null : p.id;
      CM.msg = { ok: true, text: now.inRotation ? `${verb} ${p.name}. Its sources in the rotation use it now.` : `${verb} ${p.name}. Add it to the rotation to put it on the board.` };
      render(true); srcRefresh();
    });
}

RB.delegate(root, 'click', {
  cmOpen: (el, e) => { e.preventDefault(); cmOpen(); },
  cmClose: () => { CM.open = false; CM.id = null; render(true); srcPreview(false); },
  cmBack: () => { Object.assign(CM, { id: null, confirm: null, msg: null }); render(true); },
  cmRefresh: () => { CM.detail = {}; cmLoad(true); },
  cmRetry: () => cmLoad(true),
  cmTag: el => { CM.tag = el.dataset.v; render(true); },
  cmShow: el => cmShow(el.dataset.v),
  cmGitHub: (el, e) => { e.preventDefault(); RB.invoke('open_community', { target: el.dataset.v }).catch(err => flash(err)); },
  cmInstall: el => {
    const p = cmEntry(el.dataset.v); if (!p) return;
    CM.busy = p.id; CM.msg = null; CM.confirm = null; render(true);
    RB.invoke('community_manifest', { id: p.id })
      .then(c => { CM.confirm = c; })
      .catch(e => { CM.msg = { ok: false, text: String(e) }; })
      .finally(() => { CM.busy = null; render(true); });
  },
  cmInstallNo: () => { CM.confirm = null; render(true); },
  cmInstallYes: () => {
    const c = CM.confirm, p = c && cmEntry(c.id); CM.confirm = null; if (!p) return render(true);
    const verb = p.installed ? 'Updated' : 'Installed';
    RB.invoke('import_plugin', { manifest: c.manifest })
      .then(() => cmAfterInstall(p, verb))
      .catch(e => { CM.msg = { ok: false, text: String(e) }; render(true); });
  },
  cmAddRot: el => {
    const p = cmEntry(el.dataset.v); if (!p) return;
    RB.invoke('add_source', { plugin: p.id, settings: null })
      .then(nid => {
        Object.assign(CM, { open: false, id: null, added: null });
        Object.assign(SR, { sel: nid, open: nid, picker: false });
        flash(`Added ${p.name} to the rotation${p.secrets.length ? '. It needs a key: paste it below.' : ''}`, true);
        return srcRefresh();
      })
      .then(() => srcPreview(true))
      .catch(e => { CM.msg = { ok: false, text: String(e) }; render(true); });
  }
});
root.addEventListener('input', e => {
  if (!e.target.dataset || !e.target.dataset.cmQ) return;
  CM.q = e.target.value;
  const list = document.getElementById('cmList');
  if (list && CM.data) list.innerHTML = cmRows();
});
root.addEventListener('keydown', e => {
  if ((e.key === 'Enter' || e.key === ' ') && e.target.classList && e.target.classList.contains('cmRow')) { e.preventDefault(); return cmShow(e.target.dataset.v); }
  if (e.key === 'Escape' && CM.open && L.view === 'settings' && L.tab === 'sources') {
    e.preventDefault(); e.stopPropagation();
    if (CM.confirm) CM.confirm = null; else if (CM.id) CM.id = null; else CM.open = false;
    render(true);
  }
}, true);
