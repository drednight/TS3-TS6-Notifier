/* Soundboard (dual output), group icons, display target, range painting. */
(function () {
  const t = (key) => window.I18N.t(key);

  /* ---------------- shared state refs (filled by app.js) ---------------- */

  const TSN = (window.TSN = window.TSN || {});
  TSN.config = TSN.config || null;
  TSN.state = TSN.state || null;

  /* ---------------- class group / group icon ---------------- */

  /**
   * Class set: server-group-name label → bundled icon (the very icons the
   * server uses). Matched against group names, e.g. «Лучники Ордена» → Лучник.
   * No «Братья и сёстры» by request.
   */
  const CLASS_META = [
    { key: 'kanglong', ru: 'Канглонг', en: 'Kanglong', icon: '/img/classes/canglong.webp' },
    { key: 'blood', ru: 'Дух Крови', en: 'Blood Spirit', icon: '/img/classes/dk.webp' },
    { key: 'werewolf', ru: 'Оборотень', en: 'Werewolf', icon: '/img/classes/tank.webp' },
    { key: 'warrior', ru: 'Воин', en: 'Warrior', icon: '/img/classes/var.webp' },
    { key: 'assassin', ru: 'Убийца', en: 'Assassin', icon: '/img/classes/sin.webp' },
    { key: 'gunslinger', ru: 'Стрелок', en: 'Gunner', icon: '/img/classes/gan.webp' },
    { key: 'guardian', ru: 'Страж', en: 'Guardian', icon: '/img/classes/sik.webp' },
    { key: 'priest', ru: 'Прист', en: 'Priest', icon: '/img/classes/prist.webp' },
    { key: 'archer', ru: 'Лучник', en: 'Archer', icon: '/img/classes/luk.webp' },
    { key: 'paladin', ru: 'Паладин', en: 'Paladin', icon: '/img/classes/pal.webp' },
    { key: 'shaman', ru: 'Шаман', en: 'Shaman', icon: '/img/classes/sham.webp' },
    { key: 'bard', ru: 'Бард', en: 'Bard', icon: '/img/classes/bard.webp' },
    { key: 'mystic', ru: 'Мистик', en: 'Mystic', icon: '/img/classes/mist.webp' },
    { key: 'druid', ru: 'Друид', en: 'Druid', icon: '/img/classes/dru.webp' },
    { key: 'monkey', ru: 'Обезьяна', en: 'Monkey', icon: '/img/classes/mk.webp' },
    { key: 'mage', ru: 'Маг', en: 'Mage', icon: '/img/classes/mag.webp' },
    { key: 'reaper', ru: 'Жнец', en: 'Reaper', icon: '/img/classes/kosa.webp' },
    { key: 'ghost', ru: 'Призрак', en: 'Ghost', icon: '/img/classes/gost.webp' },
  ];
  TSN.CLASS_META = CLASS_META;

  function normName(s) {
    return String(s || '')
      .toLowerCase()
      .replace(/ё/g, 'е')
      .replace(/[^a-zа-я0-9 ]/gi, ' ')
      .replace(/\s+/g, ' ')
      .trim();
  }

  /** Word-prefix name match: «Духи Крови» matches «Дух Крови», «Лучники» → «Лучник». */
  function nameMatches(name, label) {
    const nWords = normName(name).split(' ').filter(Boolean);
    const lWords = normName(label).split(' ').filter(Boolean);
    if (!lWords.length || !nWords.length) return false;
    return lWords.every((lw) => nWords.some((nw) => nw.startsWith(lw) || lw.startsWith(nw)));
  }

  function userGroupIds(user) {
    return String(user.serverGroups || '')
      .split(',')
      .map((x) => parseInt(x.trim(), 10))
      .filter((n) => Number.isFinite(n) && n > 0);
  }

  /** Find the user's class group: a group whose NAME matches one of the classes. */
  function classGroupFor(user) {
    if (!user) return null;
    const groups = TSN.state?.groups || [];
    const ids = userGroupIds(user);
    // labels sorted longest-first so «Дух Крови» wins over shorter names
    const labels = [];
    for (const meta of CLASS_META) {
      labels.push([meta, meta.ru]);
      labels.push([meta, meta.en]);
    }
    labels.sort((a, b) => b[1].length - a[1].length);
    const map = TSN.config?.classes?.map || {};
    for (const gid of ids) {
      const g = groups.find((x) => x.id === gid);
      if (!g) continue;
      // 1) manual mapping (group id → class) — needed for TS6 (no names there)
      const mapped = map[String(g.id)];
      if (mapped && mapped !== 'none') {
        const meta = CLASS_META.find((m) => m.key === mapped);
        if (meta) return { group: g, meta };
      }
      // 2) name matching (TS3 groups carry names)
      if (g.name) {
        const hit = labels.find(([, label]) => nameMatches(g.name, label));
        if (hit) return { group: g, meta: hit[0] };
      }
    }
    // Fallback: first group carrying an icon.
    for (const gid of ids) {
      const g = groups.find((x) => x.id === gid);
      if (g && g.iconId > 0) return { group: g, meta: null };
    }
    return null;
  }

  /** Bundled class icon (the server's own icon art) or null. */
  function classIconFor(user) {
    return classGroupFor(user)?.meta?.icon ?? null;
  }

  /** Avatar icon URL: class icon first, then the group's server icon from the client cache. */
  function groupIconUrl(user) {
    const hit = classGroupFor(user);
    if (hit?.meta) return hit.meta.icon;
    return hit && hit.group.iconId > 0 ? `/api/tsicon/${hit.group.iconId}` : null;
  }

  /** Class key for the SVG glyph fallback (Лучник, Прист, …). */
  function classKeyFor(user) {
    return classGroupFor(user)?.meta?.key ?? null;
  }

  /* ---------------- dual audio playback (local + channel) ---------------- */

  const MODIFIERS = ['Control', 'Alt', 'Shift', 'Meta', 'OS', 'AltGraph', 'CapsLock'];

  function makePlayer() {
    return { ctx: null, sources: new Set(), buffers: new Map() };
  }
  const players = { local: makePlayer(), channel: makePlayer() };

  async function playerCtx(kind) {
    const p = players[kind];
    if (!p.ctx) {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      if (!Ctx) return null;
      p.ctx = new Ctx();
      const device = deviceFor(kind);
      if (p.ctx.setSinkId && device) {
        try {
          await p.ctx.setSinkId(device);
        } catch {
          /* device may be gone */
        }
      }
    }
    if (p.ctx.state === 'suspended') await p.ctx.resume().catch(() => {});
    return p.ctx;
  }

  function deviceFor(kind) {
    const s = TSN.config?.sounds ?? {};
    return kind === 'local' ? s.localDevice || '' : s.outputDevice || '';
  }

  function volumeFor(kind) {
    const s = TSN.config?.sounds ?? {};
    return Number(kind === 'local' ? (s.localVolume ?? 0.8) : (s.volume ?? 0.8));
  }

  function enabledFor(kind) {
    const s = TSN.config?.sounds ?? {};
    return kind === 'local' ? s.localEnabled !== false : s.enabled !== false;
  }

  async function setOutputDevice(kind, id) {
    const p = players[kind];
    if (p.ctx && p.ctx.setSinkId) {
      try {
        await p.ctx.setSinkId(id || '');
      } catch {
        /* ignore */
      }
    }
    p.buffers.clear();
  }

  async function playOn(kind, file, itemVolume = 1, itemId = null, trim = null) {
    try {
      const ctx = await playerCtx(kind);
      const p = players[kind];
      const vol = Math.max(0, Math.min(1, volumeFor(kind) * itemVolume));
      if (!ctx) {
        const el = new Audio(file);
        el.volume = vol;
        el.__item = itemId;
        const device = deviceFor(kind);
        if (el.setSinkId && device) await el.setSinkId(device).catch(() => {});
        const startAt = Math.max(0, Number(trim?.start) || 0);
        if (startAt) el.currentTime = startAt;
        const endAt = Number(trim?.end) || 0;
        if (endAt > startAt) {
          el.addEventListener('timeupdate', () => {
            if (el.currentTime >= endAt) el.pause();
          });
        }
        await el.play().catch(() => {});
        return;
      }
      let buf = p.buffers.get(file);
      if (!buf) {
        const res = await fetch(file);
        const arr = await res.arrayBuffer();
        buf = await ctx.decodeAudioData(arr);
        p.buffers.set(file, buf);
      }
      const src = ctx.createBufferSource();
      src.buffer = buf;
      src.__item = itemId;
      const gain = ctx.createGain();
      gain.gain.value = vol;
      src.connect(gain).connect(ctx.destination);
      p.sources.add(src);
      src.onended = () => p.sources.delete(src);
      const startAt = Math.max(0, Math.min(Number(trim?.start) || 0, Math.max(0, buf.duration - 0.02)));
      const endAt = Number(trim?.end) || 0;
      const duration = endAt > startAt ? endAt - startAt : undefined;
      src.start(0, startAt, duration);
    } catch (err) {
      console.warn(`[soundboard] ${kind} play failed:`, err);
    }
  }

  /** Play to every enabled target ("мне" and "в канал"). Re-press restarts — never stacks. */
  async function playFile(file, itemVolume = 1, itemId = null, trim = null) {
    if (itemId) stopItem(itemId); // one sound = one track: restart instead of stacking
    const jobs = [];
    if (enabledFor('local')) jobs.push(playOn('local', file, itemVolume, itemId, trim));
    if (enabledFor('channel')) jobs.push(playOn('channel', file, itemVolume, itemId, trim));
    await Promise.all(jobs);
    updateStopButtons();
  }

  function isPlaying(id) {
    for (const kind of ['local', 'channel']) {
      for (const src of players[kind].sources) {
        if (src.__item === id) return true;
      }
    }
    return false;
  }

  function stopAll() {
    for (const kind of ['local', 'channel']) {
      const p = players[kind];
      for (const src of p.sources) {
        try {
          src.stop();
        } catch {
          /* already stopped */
        }
      }
      p.sources.clear();
    }
    updateStopButtons();
  }

  function stopItem(id) {
    for (const kind of ['local', 'channel']) {
      const p = players[kind];
      for (const src of [...p.sources]) {
        if (src.__item === id) {
          try {
            src.stop();
          } catch {
            /* ignore */
          }
          p.sources.delete(src);
        }
      }
    }
    updateStopButtons();
  }

  function playById(id) {
    const item = (TSN.config?.sounds?.items || []).find((s) => s.id === id);
    if (!item) return;
    void playFile(item.file, item.volume ?? 1, item.id, { start: item.start, end: item.end });
  }

  /* ---------------- range fill painting ---------------- */

  function paintRange(input) {
    const min = Number(input.min || 0);
    const max = Number(input.max || 100);
    const val = Number(input.value);
    const pct = max > min ? ((val - min) / (max - min)) * 100 : 0;
    input.style.setProperty('--fill', `${pct}%`);
  }

  function wireRanges(root) {
    (root || document).querySelectorAll('input[type="range"]').forEach((input) => {
      paintRange(input);
      input.addEventListener('input', () => paintRange(input));
    });
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

  function captureHotkey(btn, item, cap) {
    btn.classList.add('capturing');
    btn.textContent = '●';
    const row = btn.closest('.sound-row');
    if (row) row.classList.add('capturing');
    if (cap) {
      cap.classList.add('capturing');
      cap.textContent = t('hotkeyCapture');
    }
    // Big obvious banner — the recording state must be unmissable.
    const banner = document.createElement('div');
    banner.className = 'capture-banner';
    banner.innerHTML = `<span class="capture-dot"></span><span>${t('hotkeyRecording')}</span>`;
    document.body.appendChild(banner);

    const cleanup = () => {
      btn.classList.remove('capturing');
      if (row) row.classList.remove('capturing');
      if (cap) cap.classList.remove('capturing');
      banner.remove();
      window.removeEventListener('keydown', onKey, true);
    };
    const onKey = (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (MODIFIERS.includes(e.key)) {
        // live preview of the pending combo, e.g. "Ctrl+Shift+…"
        if (cap) {
          const pending = [];
          if (e.ctrlKey) pending.push('Ctrl');
          if (e.altKey) pending.push('Alt');
          if (e.shiftKey) pending.push('Shift');
          if (e.metaKey) pending.push('Super');
          cap.textContent = pending.length ? `${pending.join('+')}+…` : t('hotkeyCapture');
        }
        return; // keep waiting for the real key
      }
      if (e.key === 'Escape') {
        cleanup();
        renderSoundItems();
        return;
      }
      const parts = [];
      if (e.ctrlKey) parts.push('Ctrl');
      if (e.altKey) parts.push('Alt');
      if (e.shiftKey) parts.push('Shift');
      if (e.metaKey) parts.push('Super');
      let key = (e.code || e.key || '').replace(/^Key/, '').replace(/^Digit/, '');
      if (key) parts.push(key.length === 1 ? key.toUpperCase() : key);
      if (parts.length) {
        item.hotkey = parts.join('+');
        saveSounds();
      }
      cleanup();
      renderSoundItems();
    };
    window.addEventListener('keydown', onKey, true);
  }

  function saveSounds() {
    if (!TSN.config?.sounds) return;
    putConfig({ sounds: { items: TSN.config.sounds.items } });
  }

  function updateStopButtons() {
    document.querySelectorAll('.sound-item').forEach((wrap) => {
      const btn = wrap.querySelector('.sound-play');
      if (!btn) return;
      const active = isPlaying(wrap.dataset.id);
      btn.textContent = active ? '■' : '▶';
      btn.classList.toggle('playing', active);
    });
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
    const groupNames = [...new Set(items.map((i) => i.group).filter(Boolean))];
    list.innerHTML = items
      .map(
        (item) => `<div class="sound-item" data-id="${item.id}">
        <div class="sound-row">
          <button class="icon-btn sound-play" title="${t('testPlay')}">▶</button>
          <input class="sound-name" value="${String(item.name).replace(/"/g, '&quot;')}" />
          ${item.group ? `<span class="group-chip">${String(item.group).replace(/</g, '&lt;')}</span>` : ''}
          <span class="hotkey-cap" title="${t('hotkeyLabel')}">${item.hotkey || t('hotkeyNone')}</span>
          <button class="icon-btn hotkey-edit" title="${t('hotkeyLabel')}">⌨</button>
          <input class="vol" type="range" min="0" max="1" step="0.05" value="${item.volume ?? 1}" title="${t('soundsVolume')}" />
          <button class="icon-btn sound-edit" title="${t('editSound')}">✎</button>
          <button class="icon-btn sound-del" title="${t('deleteSound')}">✕</button>
        </div>
        <div class="sound-details" hidden>
          <label>${t('soundGroup')}</label>
          <input class="d-group" list="soundGroupList" value="${String(item.group || '').replace(/"/g, '&quot;')}" />
          <label>${t('soundStart')}</label>
          <input class="d-start" type="number" min="0" step="0.1" value="${item.start ?? 0}" />
          <label>${t('soundEnd')}</label>
          <input class="d-end" type="number" min="0" step="0.1" value="${item.end ?? 0}" />
          <label>${t('soundsVolume')}</label>
          <input class="d-vol" type="number" min="0" max="1" step="0.05" value="${item.volume ?? 1}" />
          <datalist id="soundGroupList">${groupNames.map((g) => `<option value="${String(g).replace(/"/g, '&quot;')}"></option>`).join('')}</datalist>
        </div>
      </div>`,
      )
      .join('');
    list.querySelectorAll('.sound-item').forEach((wrap) => {
      const row = wrap.querySelector('.sound-row');
      const details = wrap.querySelector('.sound-details');
      const item = items.find((x) => x.id === wrap.dataset.id);
      if (!item || !row) return;
      row.querySelector('.sound-play').addEventListener('click', () => {
        if (isPlaying(item.id)) {
          stopItem(item.id);
        } else {
          void playFile(item.file, item.volume ?? 1, item.id, { start: item.start, end: item.end });
        }
      });
      const editBtn = row.querySelector('.sound-edit');
      editBtn.addEventListener('click', () => {
        details.hidden = !details.hidden;
        editBtn.classList.toggle('active', !details.hidden);
      });
      const num = (cls, apply) => {
        const el = details.querySelector(cls);
        if (!el) return;
        el.addEventListener('change', () => {
          apply(Math.max(0, Number(el.value) || 0));
          saveSounds();
          renderSoundItems();
        });
      };
      const groupEl = details.querySelector('.d-group');
      if (groupEl) {
        groupEl.addEventListener('change', () => {
          item.group = groupEl.value.trim();
          saveSounds();
          renderSoundItems();
        });
      }
      num('.d-start', (v) => (item.start = v));
      num('.d-end', (v) => (item.end = v));
      num('.d-vol', (v) => (item.volume = Math.min(1, v)));
      row.querySelector('.sound-name').addEventListener('change', (e) => {
        item.name = e.target.value.trim() || item.name;
        saveSounds();
      });
      const edit = row.querySelector('.hotkey-edit');
      const cap = row.querySelector('.hotkey-cap');
      edit.addEventListener('click', () => captureHotkey(edit, item, cap));
      row.querySelector('.vol').addEventListener('input', (e) => {
        item.volume = Number(e.target.value);
        saveSounds();
      });
      row.querySelector('.sound-del').addEventListener('click', () => {
        stopItem(item.id);
        const file = String(item.file || '').replace(/^\/sounds\//, '');
        if (item.file && item.file.startsWith('/sounds/')) {
          fetch(`/api/sounds/${encodeURIComponent(file)}`, { method: 'DELETE' }).catch(() => {});
        }
        TSN.config.sounds.items = items.filter((x) => x.id !== item.id);
        saveSounds();
        renderSoundItems();
      });
    });
    wireRanges(list);
    updateStopButtons();
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
    const selects = [
      document.getElementById('audioOutput'),
      document.getElementById('audioOutputLocal'),
    ].filter(Boolean);
    if (!selects.length) return;
    try {
      const list = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'audiooutput');
      for (const sel of selects) {
        sel.innerHTML =
          `<option value="">${t('outputDeviceDefault')}</option>` +
          list
            .filter((d) => d.deviceId && d.deviceId !== 'default')
            .map((d) => `<option value="${d.deviceId}">${d.label || d.deviceId.slice(0, 12)}</option>`)
            .join('');
      }
      const out = document.getElementById('audioOutput');
      const loc = document.getElementById('audioOutputLocal');
      if (out) out.value = TSN.config?.sounds?.outputDevice || '';
      if (loc) loc.value = TSN.config?.sounds?.localDevice || '';
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
    const localEnabled = document.getElementById('localEnabled');
    const volume = document.getElementById('soundVolume');
    const localVolume = document.getElementById('localVolume');
    const outSel = document.getElementById('audioOutput');
    const locSel = document.getElementById('audioOutputLocal');
    const hotkeysToggle = document.getElementById('hotkeysEnabled');

    const sync = () => {
      const s = TSN.config?.sounds;
      if (!s) return;
      if (enabled) enabled.checked = s.enabled !== false;
      if (localEnabled) localEnabled.checked = s.localEnabled !== false;
      if (volume) volume.value = String(s.volume ?? 0.8);
      if (localVolume) localVolume.value = String(s.localVolume ?? 0.8);
      if (hotkeysToggle) hotkeysToggle.checked = s.hotkeysEnabled !== false;
      document.querySelectorAll('[data-sound-range]').forEach(paintRange);
    };

    const patch = (fields) => {
      if (!TSN.config?.sounds) return;
      TSN.config.sounds = { ...TSN.config.sounds, ...fields };
      putConfig({ sounds: fields });
      sync();
    };

    if (enabled) enabled.addEventListener('change', () => patch({ enabled: enabled.checked }));
    if (localEnabled) localEnabled.addEventListener('change', () => patch({ localEnabled: localEnabled.checked }));
    if (volume) volume.addEventListener('input', () => patch({ volume: Number(volume.value) }));
    if (localVolume) localVolume.addEventListener('input', () => patch({ localVolume: Number(localVolume.value) }));
    if (hotkeysToggle) hotkeysToggle.addEventListener('change', () => patch({ hotkeysEnabled: hotkeysToggle.checked }));
    if (outSel) {
      outSel.addEventListener('change', () => {
        patch({ outputDevice: outSel.value });
        void setOutputDevice('channel', outSel.value);
      });
    }
    if (locSel) {
      locSel.addEventListener('change', () => {
        patch({ localDevice: locSel.value });
        void setOutputDevice('local', locSel.value);
      });
    }

    const refresh = document.getElementById('refreshDevices');
    if (refresh) refresh.addEventListener('click', () => void refreshDevices());
    void detectLoopback();

    const stopAllBtn = document.getElementById('stopAllBtn');
    if (stopAllBtn) stopAllBtn.addEventListener('click', stopAll);

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
        window.open('/board.html', 'tsn-board', 'width=520,height=440');
      });
    }

    sync();
    renderSoundItems();
    void refreshDevices();
    document.addEventListener('tsn:config', () => {
      sync();
      renderSoundItems();
      void setOutputDevice('local', TSN.config?.sounds?.localDevice || '');
      void setOutputDevice('channel', TSN.config?.sounds?.outputDevice || '');
    });
    window.addEventListener('click', () => void playerCtx('local'), { once: true });
    window.addEventListener('keydown', () => void playerCtx('local'), { once: true });
  }

  /* ---------------- TS6 group → class map ---------------- */

  function renderTs6GroupMap() {
    const host = document.getElementById('ts6GroupMap');
    if (!host) return;
    const groups = TSN.state?.groups || [];
    const map = TSN.config?.classes?.map || {};
    if (!groups.length) {
      host.innerHTML = `<div class="hint">${t('ts6GroupsWaiting')}</div>`;
      return;
    }
    host.innerHTML = groups
      .map((g) => {
        const cur = map[String(g.id)] || '';
        return `<div class="group-map-row" data-gid="${g.id}">
          <span class="group-map-id">#${g.id}</span>
          <span class="group-map-name">${String(g.name || '—').replace(/</g, '&lt;')}</span>
          <select>
            <option value="">${t('ts6GroupsNone')}</option>
            ${CLASS_META.map((m) => `<option value="${m.key}" ${cur === m.key ? 'selected' : ''}>${m.ru}</option>`).join('')}
          </select>
        </div>`;
      })
      .join('');
    host.querySelectorAll('.group-map-row').forEach((row) => {
      row.querySelector('select').addEventListener('change', (e) => {
        const gid = row.dataset.gid;
        putConfig({ classes: { map: { [gid]: e.target.value || 'none' } } });
      });
    });
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
    wireRanges(document);
    renderTs6GroupMap();
    document.addEventListener('tsn:state', renderTs6GroupMap);
    document.addEventListener('tsn:config', renderTs6GroupMap);
    document.dispatchEvent(new CustomEvent('tsn:features-ready'));
  }

  TSN.groupIconUrl = groupIconUrl;
  TSN.classIconFor = classIconFor;
  TSN.classKeyFor = classKeyFor;
  TSN.Soundboard = {
    playFile,
    playById,
    stopAll,
    setOutputDevice: (id) => setOutputDevice('channel', id),
    setLocalDevice: (id) => setOutputDevice('local', id),
    ensureCtx: () => playerCtx('local'),
  };
  TSN.initFeatures = initFeatures;

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initFeatures);
  } else {
    setTimeout(initFeatures, 0);
  }
})();
