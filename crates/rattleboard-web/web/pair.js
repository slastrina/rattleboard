// Asks the server for the pairing QR code with the key typed here; nothing is stored.
(() => {
  const out = document.getElementById('out'), key = document.getElementById('key');
  let timer = 0;
  document.getElementById('f').addEventListener('submit', e => {
    e.preventDefault();
    clearTimeout(timer);
    fetch('rattleboard/pair', { headers: { Authorization: 'Bearer ' + key.value.trim() }, cache: 'no-store' })
      .then(r => r.json().then(j => ({ ok: r.ok, j })))
      .then(({ ok, j }) => {
        if (!ok) { out.innerHTML = ''; const p = document.createElement('p'); p.className = 'err'; p.textContent = j.error || 'Something went wrong'; out.append(p); return; }
        out.innerHTML = `<div class="qr">${j.svg}</div>`;
        const s = document.createElement('div'); s.className = 'small';
        s.textContent = 'The code includes the key, so only show it to devices you trust. It hides in 2 minutes.';
        out.append(s);
        // Out of sight again after a while, as on the desktop.
        timer = setTimeout(() => { out.innerHTML = ''; key.value = ''; }, 120000);
      })
      .catch(() => { out.textContent = "Couldn't reach the board"; });
  });
})();
