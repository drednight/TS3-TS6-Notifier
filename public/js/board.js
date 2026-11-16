/* Standalone soundboard panel (RP-Soundboard style): grid of sound buttons
 * with a top menu (add / delete / stop / hotkeys / audio mix).
 *
 * Click → play locally + to the channel (per the "мне" / "в канал" settings).
 * Hotkeys (desktop) broadcast {type:'sound'} — the overlay page plays those;
 * this panel flashes the matching button.
 */
(function () {
  const $ = (id) => document.getElementById(id);
  const t = (key) => window.I18N.t(key);

  let config = null;
  let gridSig = null;
  let deleteMode = false;

  /* ---------------- grid ---------------- */

  function renderGrid() {
    const grid = $('boardGrid');
    const empty = $('boardEmpty');
    const items = config?.sounds?.items || [];
    const sig = items.map((i) => `${i.id}:${i.name}:${i.file}:${i.hotkey}`).join(',');
    if (sig === gridSig) return;
    gridSig = sig;
    empty.hidden = items.length > 0;
    grid.hidden = items.length === 0;
    grid.innerHTML = items
      .map(
        (i) =>
          `<button class="board-btn" data-id="${i.id}">
            <span class="board-btn-name">${String(i.name).replace(/</g, '&lt;')}</span>
            <span class="board-btn-key">${String(i.hotkey || '').replace(/</g, '&lt;')}</span>
          </button>`,
      )
      .join('');
    grid.querySelectorAll('.board-btn').forEach((btn) => {
      btn.addEventListener('click', () => {
        const item = (config?.sounds?.items || []).find((x) => x.id === btn.dataset.id);
        if (!item) return;
        if (deleteMode) {
          deleteItem(item);
          return;
        }
        flash(btn);
        if (window.TSN?.Soundboard) void window.TSN.Soundboard.playFile(item.file, item.volume ?? 1, item.id);
      });
      btn.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        window.TSN?.Soundboard?.stopAll();
      });
    });
  }

  function flash(btn) {
    btn.classList.remove('flash');
    void btn.offsetWidth;
    btn.classList.add('flash');
  }

  /* ---------------- items: add / delete ---------------- */

  function saveItems(items, patch = {}) {
    if (!config?.sounds) return;
    config.sounds = { ...config.sounds, items, ...patch };
    fetch('/api/config', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ sounds: { items, ...patch } }),
    }).catch(() => {});
    gridSig = null;
    renderGrid();
  }

  function deleteItem(item) {
    const file = String(item.file || '').replace(/^\/sounds\//, '');
    if (item.file && item.file.startsWith('/sounds/')) {
      fetch(`/api/sounds/${encodeURIComponent(file)}`, { method: 'DELETE' }).catch(() => {});
    }
    window.TSN?.Soundboard?.stopAll();
    saveItems((config?.sounds?.items || []).filter((x) => x.id !== item.id));
  }

  function wireUpload() {
    const upload = $('boardUpload');
    $('menuAdd').addEventListener('click', () => upload.click());
    upload.addEventListener('change', async () => {
      const file = upload.files?.[0];
      if (!file || !config?.sounds) return;
      const reader = new FileReader();
      reader.onload = async () => {
        const data = String(reader.result).split(',')[1] || '';
        try {
          const res = await fetch('/api/sounds', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name: file.name, data }),
          });
          const out = await res.json();
          if (out.ok && out.file) {
            const items = [...(config.sounds.items || [])];
            items.push({
              id: `u-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`,
              name: out.name || file.name,
              file: out.file,
              hotkey: '',
              volume: 1,
            });
            saveItems(items);
          }
        } catch (err) {
          console.warn('[board] upload failed:', err);
        }
      };
      reader.readAsDataURL(file);
      upload.value = '';
    });
  }

  function setDeleteMode(on) {
    deleteMode = on;
    $('menuDelete').classList.toggle('on', on);
    $('boardGrid').classList.toggle('deleting', on);
    $('boardHint').textContent = on ? t('deleteModeHint') : t('boardHint');
  }

  /* ---------------- audio popover (мне / в канал) ---------------- */

  function syncAudioPop() {
    const s = config?.sounds || {};
    $('popLocalEnabled').checked = s.localEnabled !== false;
    $('popChannelEnabled').checked = s.enabled !== false;
    $('popLocalVol').value = String(s.localVolume ?? 0.8);
    $('popChannelVol').value = String(s.volume ?? 0.8);
    $('popLocalDevice').value = s.localDevice || '';
    $('popChannelDevice').value = s.outputDevice || '';
    const hk = $('menuHotkeys');
    if (hk) hk.classList.toggle('on', s.hotkeysEnabled !== false);
  }

  function putSounds(fields) {
    if (!config?.sounds) return;
    config.sounds = { ...config.sounds, ...fields };
    fetch('/api/config', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ sounds: fields }),
    }).catch(() => {});
    syncAudioPop();
    if (window.TSN?.Soundboard && 'localDevice' in fields) void window.TSN.Soundboard.setLocalDevice(fields.localDevice || '');
  }

  async function fillDevices() {
    try {
      const list = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'audiooutput');
      for (const sel of [$('popLocalDevice'), $('popChannelDevice')]) {
        if (!sel) continue;
        sel.innerHTML =
          `<option value="">${t('outputDeviceDefault')}</option>` +
          list
            .filter((d) => d.deviceId && d.deviceId !== 'default')
            .map((d) => `<option value="${d.deviceId}">${d.label || d.deviceId.slice(0, 12)}</option>`)
            .join('');
      }
      syncAudioPop();
    } catch {
      /* no permission / unsupported */
    }
  }

  function wireMenu() {
    wireUpload();
    $('menuDelete').addEventListener('click', () => setDeleteMode(!deleteMode));
    $('menuStop').addEventListener('click', () => window.TSN?.Soundboard?.stopAll());
    $('menuHotkeys').addEventListener('click', () => {
      const on = config?.sounds?.hotkeysEnabled !== false;
      putSounds({ hotkeysEnabled: !on });
    });
    $('menuAudio').addEventListener('click', () => {
      const pop = $('audioPop');
      pop.hidden = !pop.hidden;
      $('menuAudio').classList.toggle('on', !pop.hidden);
      if (!pop.hidden) void fillDevices();
    });
    $('popLocalEnabled').addEventListener('change', (e) => putSounds({ localEnabled: e.target.checked }));
    $('popChannelEnabled').addEventListener('change', (e) => putSounds({ enabled: e.target.checked }));
    $('popLocalVol').addEventListener('input', (e) => putSounds({ localVolume: Number(e.target.value) }));
    $('popChannelVol').addEventListener('input', (e) => putSounds({ volume: Number(e.target.value) }));
    $('popLocalDevice').addEventListener('change', (e) => putSounds({ localDevice: e.target.value }));
    $('popChannelDevice').addEventListener('change', (e) => {
      putSounds({ outputDevice: e.target.value });
      if (window.TSN?.Soundboard) void window.TSN.Soundboard.setOutputDevice(e.target.value);
    });
    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        if (deleteMode) setDeleteMode(false);
        $('audioPop').hidden = true;
        $('menuAudio').classList.remove('on');
      }
    });
  }

  /* ---------------- config / ws ---------------- */

  function applyConfig(cfg) {
    config = cfg;
    document.body.dataset.theme = cfg?.appearance?.theme || 'glass';
    const accent = cfg?.appearance?.accent || '#7c6cff';
    document.documentElement.style.setProperty('--accent', accent);
    document.documentElement.style.setProperty(
      '--accent-rgb',
      accent
        .replace('#', '')
        .match(/../g)
        ?.map((h) => parseInt(h, 16))
        .join(', ') || '124, 108, 255',
    );
    renderGrid();
    syncAudioPop();
  }

  function connectWS() {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.onmessage = (event) => {
      let msg;
      try {
        msg = JSON.parse(event.data);
      } catch {
        return;
      }
      if (msg.type === 'hello' || msg.type === 'config') {
        applyConfig(msg.config);
      } else if (msg.type === 'sound') {
        // Hotkey trigger — the overlay page plays it; flash the button here.
        const btn = document.querySelector(`.board-btn[data-id="${msg.id}"]`);
        if (btn) flash(btn);
      }
    };
    ws.onclose = () => setTimeout(connectWS, 1600);
    ws.onerror = () => ws.close();
  }

  function init() {
    window.I18N.applyDom?.();
    $('boardGear').innerHTML = window.TSIcons.icon('gear', 15);
    $('menuAdd').insertAdjacentHTML('afterbegin', window.TSIcons.icon('upload', 14));
    $('menuDelete').insertAdjacentHTML('afterbegin', window.TSIcons.icon('trash', 14));
    $('menuStop').insertAdjacentHTML('afterbegin', window.TSIcons.icon('x', 14));
    $('menuHotkeys').insertAdjacentHTML('afterbegin', window.TSIcons.icon('keyboard', 14));
    $('menuAudio').insertAdjacentHTML('afterbegin', window.TSIcons.icon('speaker', 14));
    $('boardGear').addEventListener('click', () => {
      window.open('/?open=settings&bare=1', '_blank', 'width=420,height=800');
    });
    wireMenu();
    renderGrid(); // show the empty state right away
    fetch('/api/config')
      .then((r) => r.json())
      .then(applyConfig)
      .catch(() => applyConfig(null));
    connectWS();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
