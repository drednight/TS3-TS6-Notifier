/* Class icon mapping + soundboard (shared by overlay and settings). */
(function () {
  const t = (key) => window.I18N.t(key);

  /* ---------------- shared state refs (filled by app.js) ---------------- */

  const TSN = (window.TSN = window.TSN || {});
  TSN.config = TSN.config || null;
  TSN.state = TSN.state || null;

  /* ---------------- class mapping ---------------- */

  /** Resolve the game class key for a user (or null). */
  function classFor(user) {
    const cfg = TSN.config;
    if (!cfg?.classes?.enabled || !user) return null;
    const map = cfg.classes.map || {};
    const groups = String(user.serverGroups || '')
      .split(',')
      .map((s) => parseInt(s.trim(), 10))
      .filter((n) => Number.isFinite(n) && n > 0);
    const stateGroups = TSN.state?.groups || [];
    for (const gid of groups) {
      const hit = map[`group:${gid}`];
      if (hit) return hit;
      const g = stateGroups.find((x) => x.id === gid);
      if (g && g.iconId > 0) {
        const iconHit = map[`icon:${g.iconId}`];
        if (iconHit) return iconHit;
      }
    }
    // Also allow direct client icon mapping when the connector exposes it.
    if (user.avatarHash && map[`icon:${user.avatarHash}`]) return map[`icon:${user.avatarHash}`];
    return null;
  }

  /* ---------------- soundboard playback ---------------- */

  let audioCtx = null;
  let outputDevice = '';
  let masterVolume = 0.8;
  const buffers = new Map(); // file -> AudioBuffer

  async function ensureCtx() {
    if (!audioCtx) {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      if (!Ctx) return null;
      audioCtx = new Ctx();
      if (audioCtx.setSinkId && outputDevice) {
        try {
          await audioCtx.setSinkId(outputDevice);
        } catch {
          /* device may be gone */
        }
      }
    }
    if (audioCtx.state === 'suspended') await audioCtx.resume().catch(() => {});
    return audioCtx;
  }

  async function setOutputDevice(id) {
    outputDevice = id || '';
    if (audioCtx && audioCtx.setSinkId) {
      try {
        await audioCtx.setSinkId(outputDevice);
      } catch {
        /* ignore */
      }
    }
    buffers.clear();
  }

  async function playFile(file, volume = 1) {
    try {
      const ctx = await ensureCtx();
      if (!ctx) {
        const el = new Audio(file);
        el.volume = Math.min(1, masterVolume * volume);
        if (el.setSinkId && outputDevice) await el.setSinkId(outputDevice).catch(() => {});
        await el.play().catch(() => {});
        return;
      }
      let buf = buffers.get(file);
      if (!buf) {
        const res = await fetch(file);
        const arr = await res.arrayBuffer();
        buf = await ctx.decodeAudioData(arr);
        buffers.set(file, buf);
      }
      const src = ctx.createBufferSource();
      src.buffer = buf;
      const gain = ctx.createGain();
      gain.gain.value = Math.max(0, Math.min(1, masterVolume * volume));
      src.connect(gain).connect(ctx.destination);
      src.start();
    } catch (err) {
      console.warn('[soundboard] play failed:', err);
    }
  }

  function playById(id) {
    const item = (TSN.config?.sounds?.items || []).find((s) => s.id === id);
    if (item && TSN.config?.sounds?.enabled !== false) void playFile(item.file, item.volume ?? 1);
  }

  /* ---------------- settings helpers ---------------- */

  let cfgTimer = null;
  function putConfig(patch) {
    if (cfgTimer) clearTimeout(cfgTimer);
    cfgTimer = setTimeout(() => {
      fetch('/api/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(patch),
      }).catch(() => {});
    }, 300);
  }

  function classSelectOptions(selected) {
    const lang = window.I18N.lang;
    const opts = [`<option value="">${t('classNone')}</option>`];
    for (const key of window.TSIcons.CLASSES) {
      opts.push(
        `<option value="${key}" ${key === selected ? 'selected' : ''}>${window.TSIcons.classLabel(key, lang)}</option>`,
      );
    }
    return opts.join('');
  }

  /* ---------------- classes tab ---------------- */

  function renderLegend() {
    const box = document.getElementById('classLegend');
    if (!box) return;
    const lang = window.I18N.lang;
    box.innerHTML = window.TSIcons.CLASSES.map(
      (key) =>
        `<div class="legend-item">${window.TSIcons.classGlyph(key, 22)}<span>${window.TSIcons.classLabel(key, lang)}</span></div>`,
    ).join('');
  }

  function renderGroupMap() {
    const list = document.getElementById('groupMapList');
    if (!list) return;
    const cfg = TSN.config;
    const groups = TSN.state?.groups || [];
    const map = cfg?.classes?.map || {};
    if (!groups.length) {
      list.innerHTML = `<div class="hint">${t('noGroupsYet')}</div>`;
    } else {
      list.innerHTML = groups
        .map((g) => {
          const key = g.iconId > 0 ? `icon:${g.iconId}` : `group:${g.id}`;
          const label = g.name || `#${g.id}`;
          return `<div class="map-row" data-key="${key}">
            <span class="map-key">${key}</span>
            <span class="map-name" title="${key}">${label} (id ${g.id}${g.iconId > 0 ? `, icon ${g.iconId}` : ''})</span>
            <select class="map-class">${classSelectOptions(map[key] || '')}</select>
            <button class="icon-btn map-apply" title="${t('addBtn')}">✓</button>
          </div>`;
        })
        .join('');
      list.querySelectorAll('.map-row').forEach((row) => {
        row.querySelector('.map-apply').addEventListener('click', () => {
          const key = row.dataset.key;
          const val = row.querySelector('.map-class').value;
          const next = { ...(TSN.config.classes.map || {}) };
          if (val) next[key] = val;
          else delete next[key];
          TSN.config.classes = { ...TSN.config.classes, map: next };
          putConfig({ classes: { map: next } });
          renderManualMap();
        });
      });
    }
    renderManualMap();
  }

  function renderManualMap() {
    const list = document.getElementById('mapManualList');
    if (!list) return;
    const map = TSN.config?.classes?.map || {};
    const groups = TSN.state?.groups || [];
    const knownKeys = new Set(groups.map((g) => (g.iconId > 0 ? `icon:${g.iconId}` : `group:${g.id}`)));
    const manual = Object.entries(map).filter(([k]) => !knownKeys.has(k));
    list.innerHTML = manual
      .map(
        ([key, val]) => `<div class="map-row" data-key="${key}">
          <span class="map-key">${key}</span>
          <span class="map-name"></span>
          <select class="map-class">${classSelectOptions(val)}</select>
          <button class="icon-btn map-del" title="${t('deleteSound')}">✕</button>
        </div>`,
      )
      .join('');
    list.querySelectorAll('.map-row').forEach((row) => {
      const key = row.dataset.key;
      row.querySelector('.map-class').addEventListener('change', (e) => {
        const next = { ...(TSN.config.classes.map || {}) };
        if (e.target.value) next[key] = e.target.value;
        else delete next[key];
        TSN.config.classes = { ...TSN.config.classes, map: next };
        putConfig({ classes: { map: next } });
      });
      row.querySelector('.map-del').addEventListener('click', () => {
        const next = { ...(TSN.config.classes.map || {}) };
        delete next[key];
        TSN.config.classes = { ...TSN.config.classes, map: next };
        putConfig({ classes: { map: next } });
        renderGroupMap();
      });
    });
  }

  function wireClassesTab() {
    renderLegend();
    const enabled = document.getElementById('classesEnabled');
    if (enabled) {
      enabled.addEventListener('change', () => {
        TSN.config.classes = { ...TSN.config.classes, enabled: enabled.checked };
        putConfig({ classes: { enabled: enabled.checked } });
      });
    }
    const addBtn = document.getElementById('mapAddBtn');
    const keyInput = document.getElementById('mapKeyInput');
    const classSel = document.getElementById('mapClassSelect');
    if (classSel) classSel.innerHTML = classSelectOptions('');
    if (addBtn && keyInput && classSel) {
      addBtn.addEventListener('click', () => {
        const key = keyInput.value.trim();
        const val = classSel.value;
        if (!/^(icon|group):\d+$/.test(key) || !val) return;
        const next = { ...(TSN.config.classes.map || {}), [key]: val };
        TSN.config.classes = { ...TSN.config.classes, map: next };
        putConfig({ classes: { map: next } });
        keyInput.value = '';
        classSel.value = '';
        renderGroupMap();
      });
    }
    renderGroupMap();
    document.addEventListener('tsn:state', () => renderGroupMap());
    document.addEventListener('tsn:config', () => {
      const e = document.getElementById('classesEnabled');
      if (e && TSN.config) e.checked = !!TSN.config.classes?.enabled;
    });
  }

  /* ---------------- sound tab ---------------- */

  function captureHotkey(btn, item) {
    btn.classList.add('capturing');
    btn.textContent = t('hotkeyCapture');
    const onKey = (e) => {
      e.preventDefault();
      e.stopPropagation();
      const parts = [];
      if (e.ctrlKey) parts.push('Ctrl');
      if (e.altKey) parts.push('Alt');
      if (e.shiftKey) parts.push('Shift');
      if (e.metaKey) parts.push('Super');
      let key = e.code || e.key || '';
      key = key
        .replace(/^Key/, '')
        .replace(/^Digit/, '')
        .replace('Numpad', 'Numpad')
        .replace('Escape', 'Escape');
      if (!['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) {
        if (key) parts.push(key.length === 1 ? key.toUpperCase() : key);
      }
      if (parts.length && !['Ctrl', 'Alt', 'Shift', 'Super'].includes(parts[parts.length - 1])) {
        item.hotkey = parts.join('+');
        saveSounds();
      }
      btn.classList.remove('capturing');
      window.removeEventListener('keydown', onKey, true);
      renderSoundItems();
    };
    window.addEventListener('keydown', onKey, true);
  }

  function saveSounds() {
    if (!TSN.config?.sounds) return;
    putConfig({ sounds: { items: TSN.config.sounds.items } });
  }

  function renderSoundItems() {
    const list = document.getElementById('soundItems');
    if (!list) return;
    if (!TSN.config?.sounds) {
      list.innerHTML = '';
      return;
    }
    const items = TSN.config?.sounds?.items || [];
    if (!items.length) {
      list.innerHTML = `<div class="hint">${t('soundsEmpty')}</div>`;
      return;
    }
    list.innerHTML = items
      .map(
        (item) => `<div class="sound-row" data-id="${item.id}">
          <button class="icon-btn sound-play" title="${t('testPlay')}">▶</button>
          <input class="sound-name" value="${String(item.name).replace(/"/g, '&quot;')}" />
          <button class="hotkey-btn" title="${t('hotkeyLabel')}">${item.hotkey || t('hotkeyNone')}</button>
          <input class="vol" type="range" min="0" max="1" step="0.05" value="${item.volume ?? 1}" title="${t('soundsVolume')}" />
          <button class="icon-btn sound-del" title="${t('deleteSound')}">✕</button>
        </div>`,
      )
      .join('');
    list.querySelectorAll('.sound-row').forEach((row) => {
      const item = items.find((x) => x.id === row.dataset.id);
      if (!item) return;
      row.querySelector('.sound-play').addEventListener('click', () => void playFile(item.file, item.volume ?? 1));
      row.querySelector('.sound-name').addEventListener('change', (e) => {
        item.name = e.target.value.trim() || item.name;
        saveSounds();
      });
      const hk = row.querySelector('.hotkey-btn');
      hk.addEventListener('click', () => captureHotkey(hk, item));
      row.querySelector('.vol').addEventListener('input', (e) => {
        item.volume = Number(e.target.value);
        saveSounds();
      });
      row.querySelector('.sound-del').addEventListener('click', () => {
        const file = String(item.file || '').replace(/^\/sounds\//, '');
        if (item.file && item.file.startsWith('/sounds/')) {
          fetch(`/api/sounds/${encodeURIComponent(file)}`, { method: 'DELETE' }).catch(() => {});
        }
        TSN.config.sounds.items = items.filter((x) => x.id !== item.id);
        saveSounds();
        renderSoundItems();
      });
    });
  }

  const PRESETS = [
    { name: 'Монета', file: '/presets/coin.wav', hotkey: 'Ctrl+Alt+1' },
    { name: 'Усиление', file: '/presets/powerup.wav', hotkey: 'Ctrl+Alt+2' },
    { name: 'Барабан', file: '/presets/drum.wav', hotkey: 'Ctrl+Alt+3' },
    { name: 'Тревога', file: '/presets/alert.wav', hotkey: 'Ctrl+Alt+4' },
    { name: 'Клаксон', file: '/presets/airhorn.wav', hotkey: 'Ctrl+Alt+5' },
    { name: 'Искры', file: '/presets/sparkle.wav', hotkey: 'Ctrl+Alt+6' },
    { name: 'Ктош', file: '/presets/whoosh.wav', hotkey: 'Ctrl+Alt+7' },
    { name: 'Победа', file: '/presets/success.wav', hotkey: 'Ctrl+Alt+8' },
  ];

  function addPresets() {
    if (!TSN.config?.sounds) return;
    const items = [...(TSN.config.sounds.items || [])];
    const usedHotkeys = new Set(items.map((i) => i.hotkey));
    for (const p of PRESETS) {
      if (items.some((i) => i.file === p.file)) continue;
      items.push({
        id: `p-${p.file.split('/').pop().replace('.wav', '')}-${Math.random().toString(36).slice(2, 6)}`,
        name: p.name,
        file: p.file,
        hotkey: usedHotkeys.has(p.hotkey) ? '' : p.hotkey,
        volume: 1,
      });
    }
    TSN.config.sounds.items = items;
    saveSounds();
    renderSoundItems();
  }

  async function refreshDevices() {
    const sel = document.getElementById('audioOutput');
    if (!sel) return;
    try {
      const list = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'audiooutput');
      sel.innerHTML =
        `<option value="">${t('outputDeviceDefault')}</option>` +
        list
          .filter((d) => d.deviceId && d.deviceId !== 'default')
          .map((d) => `<option value="${d.deviceId}">${d.label || d.deviceId.slice(0, 12)}</option>`)
          .join('');
      sel.value = TSN.config?.sounds?.outputDevice || '';
    } catch {
      /* no permission / unsupported */
    }
  }

  function wireSoundTab() {
    const enabled = document.getElementById('soundsEnabled');
    const board = document.getElementById('boardVisible');
    const volume = document.getElementById('soundVolume');
    const outSel = document.getElementById('audioOutput');
    const sync = () => {
      const s = TSN.config?.sounds;
      if (!s) return;
      if (enabled) enabled.checked = !!s.enabled;
      if (board) board.checked = s.boardVisible !== false;
      if (volume) volume.value = String(s.volume ?? 0.8);
    };
    if (enabled) {
      enabled.addEventListener('change', () => {
        TSN.config.sounds = { ...TSN.config.sounds, enabled: enabled.checked };
        putConfig({ sounds: { enabled: enabled.checked } });
      });
    }
    if (board) {
      board.addEventListener('change', () => {
        TSN.config.sounds = { ...TSN.config.sounds, boardVisible: board.checked };
        putConfig({ sounds: { boardVisible: board.checked } });
      });
    }
    if (volume) {
      volume.addEventListener('input', () => {
        masterVolume = Number(volume.value);
        TSN.config.sounds = { ...TSN.config.sounds, volume: masterVolume };
        putConfig({ sounds: { volume: masterVolume } });
      });
    }
    if (outSel) {
      outSel.addEventListener('change', () => {
        TSN.config.sounds = { ...TSN.config.sounds, outputDevice: outSel.value };
        putConfig({ sounds: { outputDevice: outSel.value } });
        void setOutputDevice(outSel.value);
      });
    }
    const refresh = document.getElementById('refreshDevices');
    if (refresh) refresh.addEventListener('click', () => void refreshDevices());

    const uploadBtn = document.getElementById('soundUploadBtn');
    const upload = document.getElementById('soundUpload');
    if (uploadBtn && upload) {
      uploadBtn.addEventListener('click', () => upload.click());
      upload.addEventListener('change', async () => {
        const file = upload.files?.[0];
        if (!file || !TSN.config?.sounds) return;
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
              const items = [...(TSN.config.sounds.items || [])];
              items.push({
                id: `u-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`,
                name: out.name || file.name,
                file: out.file,
                hotkey: '',
                volume: 1,
              });
              TSN.config.sounds.items = items;
              saveSounds();
              renderSoundItems();
            }
          } catch (err) {
            console.warn('[soundboard] upload failed:', err);
          }
        };
        reader.readAsDataURL(file);
        upload.value = '';
      });
    }

    const presetsBtn = document.getElementById('addPresetsBtn');
    if (presetsBtn) presetsBtn.addEventListener('click', addPresets);

    sync();
    renderSoundItems();
    void refreshDevices();
    document.addEventListener('tsn:config', () => {
      sync();
      renderSoundItems();
      masterVolume = Number(TSN.config?.sounds?.volume ?? 0.8);
      void setOutputDevice(TSN.config?.sounds?.outputDevice || '');
    });
    window.addEventListener('click', () => void ensureCtx(), { once: true });
    window.addEventListener('keydown', () => void ensureCtx(), { once: true });
  }

  /* ---------------- init ---------------- */

  function initFeatures() {
    wireClassesTab();
    wireSoundTab();
    masterVolume = Number(TSN.config?.sounds?.volume ?? 0.8);
    outputDevice = TSN.config?.sounds?.outputDevice || '';
    document.dispatchEvent(new CustomEvent('tsn:features-ready'));
  }

  TSN.classFor = classFor;
  TSN.Soundboard = { playFile, playById, setOutputDevice, ensureCtx };
  TSN.initFeatures = initFeatures;

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initFeatures);
  } else {
    // Defer so app.js has delivered the first config snapshot via WS when possible.
    setTimeout(initFeatures, 0);
  }
})();
