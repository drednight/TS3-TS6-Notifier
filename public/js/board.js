/* Standalone soundboard panel (RP-Soundboard style): grid of sound buttons.
 *
 * Click → play locally (through the configured output device).
 * Hotkeys (desktop) broadcast {type:'sound'} — the overlay page plays those;
 * this panel just flashes the matching button.
 */
(function () {
  const $ = (id) => document.getElementById(id);
  const t = (key) => window.I18N.t(key);

  let config = null;
  let gridSig = null;

  /* ---------------- audio ---------------- */

  let audioCtx = null;
  const activeSources = new Set();

  async function ensureCtx() {
    if (!audioCtx) {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      if (!Ctx) return null;
      audioCtx = new Ctx();
      const device = config?.sounds?.outputDevice || '';
      if (audioCtx.setSinkId && device) {
        try {
          await audioCtx.setSinkId(device);
        } catch {
          /* device may be gone */
        }
      }
    }
    if (audioCtx.state === 'suspended') await audioCtx.resume().catch(() => {});
    return audioCtx;
  }

  const buffers = new Map();

  async function playFile(file, volume = 1) {
    try {
      const ctx = await ensureCtx();
      const master = Number(config?.sounds?.volume ?? 0.8);
      if (!ctx) {
        const el = new Audio(file);
        el.volume = Math.min(1, master * volume);
        if (el.setSinkId && config?.sounds?.outputDevice) {
          await el.setSinkId(config.sounds.outputDevice).catch(() => {});
        }
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
      gain.gain.value = Math.max(0, Math.min(1, master * volume));
      src.connect(gain).connect(ctx.destination);
      activeSources.add(src);
      src.onended = () => activeSources.delete(src);
      src.start();
    } catch (err) {
      console.warn('[board] play failed:', err);
    }
  }

  function stopAll() {
    for (const src of activeSources) {
      try {
        src.stop();
      } catch {
        /* already stopped */
      }
    }
    activeSources.clear();
    if (audioCtx && audioCtx.state === 'running') audioCtx.suspend().catch(() => {});
    if (audioCtx) audioCtx.resume().catch(() => {});
  }

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
        flash(btn);
        void playFile(item.file, item.volume ?? 1);
      });
      btn.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        stopAll();
      });
    });
  }

  function flash(btn) {
    btn.classList.remove('flash');
    // reflow to restart the animation
    void btn.offsetWidth;
    btn.classList.add('flash');
  }

  /* ---------------- wiring ---------------- */

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
    $('boardStop').innerHTML = window.TSIcons.icon('x', 15);
    $('boardGear').innerHTML = window.TSIcons.icon('gear', 15);
    $('boardStop').addEventListener('click', stopAll);
    $('boardGear').addEventListener('click', () => {
      window.open('/?open=settings&bare=1', '_blank', 'width=420,height=800');
    });
    window.addEventListener('click', () => void ensureCtx(), { once: true });
    window.addEventListener('keydown', () => void ensureCtx(), { once: true });
    renderGrid(); // show the empty state right away
    fetch('/api/config')
      .then((r) => r.json())
      .then(applyConfig)
      .catch(() => applyConfig(null));
    connectWS();
    // language comes from config; re-apply labels after first config
    const obs = () => {
      const lang = config?.appearance?.language;
      if (lang && lang !== 'auto') window.I18N.setLanguage(lang);
    };
    setInterval(obs, 2000);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
