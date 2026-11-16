/* Standalone soundboard panel (RP-Soundboard style): grid of sound buttons.
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
        if (window.TSN?.Soundboard) void window.TSN.Soundboard.playFile(item.file, item.volume ?? 1);
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

  /* ---------------- mix footer ---------------- */

  function syncMix() {
    const s = config?.sounds || {};
    const lv = $('boardLocalVol');
    const cv = $('boardChannelVol');
    if (lv) {
      lv.value = String(s.localVolume ?? 0.8);
      lv.disabled = s.localEnabled === false;
    }
    if (cv) {
      cv.value = String(s.volume ?? 0.8);
      cv.disabled = s.enabled === false;
    }
    const hk = $('boardHotkeys');
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
    syncMix();
  }

  function wireMix() {
    const lv = $('boardLocalVol');
    const cv = $('boardChannelVol');
    const hk = $('boardHotkeys');
    if (lv) lv.addEventListener('input', () => putSounds({ localVolume: Number(lv.value) }));
    if (cv) cv.addEventListener('input', () => putSounds({ volume: Number(cv.value) }));
    if (hk) {
      hk.addEventListener('click', () => {
        const on = config?.sounds?.hotkeysEnabled !== false;
        putSounds({ hotkeysEnabled: !on });
      });
    }
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
    syncMix();
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
    $('boardStop').addEventListener('click', () => window.TSN?.Soundboard?.stopAll());
    $('boardGear').addEventListener('click', () => {
      window.open('/?open=settings&bare=1', '_blank', 'width=420,height=800');
    });
    wireMix();
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
