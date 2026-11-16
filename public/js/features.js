/* Soundboard + group icons + display target (shared by overlay and settings). */
(function () {
  const t = (key) => window.I18N.t(key);

  /* ---------------- shared state refs (filled by app.js) ---------------- */

  const TSN = (window.TSN = window.TSN || {});
  TSN.config = TSN.config || null;
  TSN.state = TSN.state || null;

  /* ---------------- group icon (replaces avatar art) ---------------- */

  /**
   * URL of the user's server-group icon (TS3 reads it from the client cache),
   * or null when no group icon is available.
   */
  function groupIconUrl(user) {
    if (!user) return null;
    const ids = String(user.serverGroups || '')
      .split(',')
      .map((x) => parseInt(x.trim(), 10))
      .filter((n) => Number.isFinite(n) && n > 0);
    const groups = TSN.state?.groups || [];
    for (const gid of ids) {
      const g = groups.find((x) => x.id === gid);
      if (g && g.iconId > 0) return `/api/tsicon/${g.iconId}`;
    }
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
      key = key.replace(/^Key/, '').replace(/^Digit/, '');
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
    await detectLoopback();
  }

  /** Look for a loopback input (Stereo Mix / What U Hear) — soundboard without a virtual cable. */
  async function detectLoopback() {
    const el = document.getElementById('loopbackStatus');
    if (!el) return;
    el.className = 'guide-status searching';
    el.textContent = t('loopbackSearching');
    try {
      const devices = await navigator.mediaDevices.enumerateDevices();
      const inputs = devices.filter((d) => d.kind === 'audioinput' && d.label);
      const loop = inputs.find((d) =>
        /stereo\s*mix|stereo\s*микшер|стерео\s*микшер|стереомикшер|what\s*u\s*hear|wave\s*out|loopback|mono\s*mix|петлев|смеш/i.test(d.label),
      );
      if (loop) {
        el.className = 'guide-status found';
        el.textContent = t('loopbackFound').replace('{name}', loop.label);
      } else {
        el.className = 'guide-status missing';
        el.textContent = t('loopbackMissing');
      }
    } catch {
      el.className = 'guide-status missing';
      el.textContent = t('loopbackMissing');
    }
  }

  function wireSoundTab() {
    const enabled = document.getElementById('soundsEnabled');
    const volume = document.getElementById('soundVolume');
    const outSel = document.getElementById('audioOutput');
    const sync = () => {
      const s = TSN.config?.sounds;
      if (!s) return;
      if (enabled) enabled.checked = !!s.enabled;
      if (volume) volume.value = String(s.volume ?? 0.8);
    };
    if (enabled) {
      enabled.addEventListener('change', () => {
        TSN.config.sounds = { ...TSN.config.sounds, enabled: enabled.checked };
        putConfig({ sounds: { enabled: enabled.checked } });
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
    void detectLoopback();

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

    const openBoard = document.getElementById('openBoardBtn');
    if (openBoard) {
      openBoard.addEventListener('click', () => {
        window.open('/board.html', '_blank', 'width=520,height=440');
      });
    }

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

  /* ---------------- display target (desktop, multi-monitor) ---------------- */

  async function wireDisplaySelect() {
    const field = document.getElementById('displayField');
    const sel = document.getElementById('displaySelect');
    if (!field || !sel) return;
    try {
      const res = await fetch('/api/displays');
      const out = await res.json();
      const displays = out.displays || [];
      if (!displays.length) return; // web mode — hide the field
      field.hidden = false;
      sel.innerHTML = displays
        .map(
          (d) =>
            `<option value="${d.id}">${d.primary ? `${t('displayPrimary')} — ` : ''}${String(d.label).replace(/</g, '&lt;')}</option>`,
        )
        .join('');
      const primary = displays.find((d) => d.primary);
      if (primary) {
        const opt = sel.querySelector(`option[value="${primary.id}"]`);
        if (opt) opt.value = 'primary';
      }
      sel.value = TSN.config?.desktop?.display || 'primary';
      if (!sel.value) sel.value = 'primary';
      sel.addEventListener('change', () => {
        if (!TSN.config) return;
        TSN.config.desktop = { ...TSN.config.desktop, display: sel.value };
        putConfig({ desktop: { display: sel.value } });
      });
    } catch {
      /* web mode */
    }
  }

  /* ---------------- init ---------------- */

  function initFeatures() {
    wireSoundTab();
    void wireDisplaySelect();
    masterVolume = Number(TSN.config?.sounds?.volume ?? 0.8);
    outputDevice = TSN.config?.sounds?.outputDevice || '';
    document.dispatchEvent(new CustomEvent('tsn:features-ready'));
  }

  TSN.groupIconUrl = groupIconUrl;
  TSN.Soundboard = { playFile, playById, setOutputDevice, ensureCtx };
  TSN.initFeatures = initFeatures;

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initFeatures);
  } else {
    // Defer so app.js has delivered the first config snapshot via WS when possible.
    setTimeout(initFeatures, 0);
  }
})();
