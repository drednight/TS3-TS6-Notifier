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
    const sig = items
      .map((i) => `${i.id}:${i.name}:${i.file}:${i.hotkey}:${i.group || ''}:${i.start ?? ''}:${i.end ?? ''}:${i.volume ?? ''}`)
      .join(',');
    if (sig === gridSig) return;
    gridSig = sig;
    empty.hidden = items.length > 0;
    grid.hidden = items.length === 0;

    // group into "folders"; ungrouped sounds go under their own section
    const sections = [];
    const byName = new Map();
    for (const item of items) {
      const name = item.group || '';
      if (!byName.has(name)) {
        byName.set(name, []);
        sections.push(name);
      }
      byName.get(name).push(item);
    }
    grid.innerHTML = sections
      .map((name) => {
        const list = byName.get(name) || [];
        const title = name
          ? `<div class="board-group-title">${String(name).replace(/</g, '&lt;')}</div>`
          : sections.length > 1
            ? `<div class="board-group-title">${t('noGroup')}</div>`
            : '';
        return `<section class="board-group">${title}<div class="board-group-grid">${list
          .map(
            (i) =>
              `<button class="board-btn" data-id="${i.id}">
                <span class="board-btn-name">${String(i.name).replace(/</g, '&lt;')}</span>
                <span class="board-btn-key">${String(i.hotkey || '').replace(/</g, '&lt;')}</span>
              </button>`,
          )
          .join('')}</div></section>`;
      })
      .join('');

    grid.querySelectorAll('.board-group-title').forEach((title) => {
      title.addEventListener('click', () => title.closest('.board-group').classList.toggle('collapsed'));
    });
    grid.querySelectorAll('.board-btn').forEach((btn) => {
      btn.addEventListener('click', () => {
        const item = (config?.sounds?.items || []).find((x) => x.id === btn.dataset.id);
        if (!item) return;
        if (deleteMode) {
          deleteItem(item);
          return;
        }
        if (editMode) {
          openEdit(item);
          return;
        }
        flash(btn);
        if (window.TSN?.Soundboard) {
          void window.TSN.Soundboard.playFile(item.file, item.volume ?? 1, item.id, { start: item.start, end: item.end });
        }
      });
      btn.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        window.TSN?.Soundboard?.stopAll();
      });
    });
  }

  /* ---------------- edit popover (name / group / volume / trim) ---------------- */

  let editMode = false;
  let editing = null;

  function openEdit(item) {
    editing = item;
    $('editTitle').textContent = item.name;
    $('editName').value = item.name;
    $('editGroup').value = item.group || '';
    $('editVol').value = String(item.volume ?? 1);
    $('editStart').value = String(item.start ?? 0);
    $('editEnd').value = String(item.end ?? 0);
    $('editKey').textContent = item.hotkey || '—';
    $('boardEdit').hidden = false;
  }

  const MODIFIERS = ['Control', 'Alt', 'Shift', 'Meta', 'OS', 'AltGraph', 'CapsLock'];

  function captureEditHotkey() {
    const btn = $('editKeySet');
    const cap = $('editKey');
    if (!editing) return;
    btn.classList.add('capturing');
    cap.classList.add('capturing');
    cap.textContent = t('hotkeyCapture');
    const banner = document.createElement('div');
    banner.className = 'capture-banner';
    banner.innerHTML = `<span class="capture-dot"></span><span>${t('hotkeyRecording')}</span>`;
    document.body.appendChild(banner);
    const cleanup = () => {
      btn.classList.remove('capturing');
      cap.classList.remove('capturing');
      banner.remove();
      window.removeEventListener('keydown', onKey, true);
    };
    const onKey = (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (MODIFIERS.includes(e.key)) {
        const pending = [];
        if (e.ctrlKey) pending.push('Ctrl');
        if (e.altKey) pending.push('Alt');
        if (e.shiftKey) pending.push('Shift');
        if (e.metaKey) pending.push('Super');
        cap.textContent = pending.length ? `${pending.join('+')}+…` : t('hotkeyCapture');
        return;
      }
      if (e.key === 'Escape') {
        cleanup();
        cap.textContent = editing.hotkey || '—';
        return;
      }
      const parts = [];
      if (e.ctrlKey) parts.push('Ctrl');
      if (e.altKey) parts.push('Alt');
      if (e.shiftKey) parts.push('Shift');
      if (e.metaKey) parts.push('Super');
      let key = (e.code || e.key || '').replace(/^Key/, '').replace(/^Digit/, '');
      if (key) parts.push(key.length === 1 ? key.toUpperCase() : key);
      if (parts.length && editing) {
        editing.hotkey = parts.join('+');
        saveItems([...(config?.sounds?.items || [])]);
      }
      cleanup();
      cap.textContent = editing?.hotkey || '—';
    };
    window.addEventListener('keydown', onKey, true);
  }

  function closeEdit() {
    editing = null;
    $('boardEdit').hidden = true;
    setEditMode(false);
  }

  function setEditMode(on) {
    editMode = on;
    $('menuEdit').classList.toggle('on', on);
    $('boardGrid').classList.toggle('editing', on);
  }

  function wireEdit() {
    $('menuEdit').addEventListener('click', () => {
      if (editMode) {
        setEditMode(false);
        $('boardEdit').hidden = true;
      } else {
        setDeleteMode(false);
        setEditMode(true);
      }
    });
    $('editKeySet').addEventListener('click', captureEditHotkey);
    $('editKeyClear').addEventListener('click', () => {
      if (!editing) return;
      editing.hotkey = '';
      $('editKey').textContent = '—';
      saveItems([...(config?.sounds?.items || [])]);
    });
    $('editCancel').addEventListener('click', closeEdit);
    $('editSave').addEventListener('click', () => {
      if (editing) {
        editing.name = $('editName').value.trim() || editing.name;
        editing.group = $('editGroup').value.trim();
        editing.volume = Number($('editVol').value) || 1;
        editing.start = Math.max(0, Number($('editStart').value) || 0);
        editing.end = Math.max(0, Number($('editEnd').value) || 0);
        saveItems([...(config?.sounds?.items || [])]);
      }
      closeEdit();
    });
    $('editDelete').addEventListener('click', () => {
      if (editing) deleteItem(editing);
      closeEdit();
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
    $('menuPresets').addEventListener('click', () => {
      window.TSN?.addPresets?.();
    });
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
    if (on) {
      setEditMode(false);
      $('boardEdit').hidden = true;
    }
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
        if (editMode) closeEdit();
        $('audioPop').hidden = true;
        $('menuAudio').classList.remove('on');
      }
    });
  }

  /* ---------------- config / ws ---------------- */

  function applyConfig(cfg) {
    config = cfg;
    if (window.TSN) window.TSN.config = cfg;
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
      window.open('/?open=settings&bare=1', 'tsn-settings', 'width=420,height=800');
    });
    wireMenu();
    wireEdit();
    $('menuEdit').insertAdjacentHTML('afterbegin', window.TSIcons.icon('gear', 14));
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
