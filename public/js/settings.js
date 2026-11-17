/* Appearance model + settings drawer wiring.
 *
 * Effective value precedence (low → high):
 *   defaults → server config → localStorage → URL params.
 */
(function () {
  const LS_KEY = 'tsnotifier:appearance';

  const DEFAULTS = {
    theme: 'glass',
    accent: '#7c6cff',
    nickColor: '#e8ecff',
    msgColor: '#b6b6bd',
    talkOutline: '#57e389',
    glow: 0.7,
    opacity: 0.72,
    blur: 18,
    radius: 16,
    scale: 1,
    fontSize: 13,
    corner: 'tr',
    marginX: 18,
    marginY: 18,
    language: 'auto',
    showHeader: true,
    showServerName: true,
    showUserCount: true,
    showBanner: true,
    showAvatars: true,
    showBadges: true,
    showSelf: true,
    hideQuery: true,
    animations: true,
    eqStyle: 'bars',
    talkingFirst: true,
    onlyTalking: false,
    compact: false,
    hideEmpty: false,
    showWhispers: true,
    showChat: true,
    chatDelay: 8,
    chatPrivate: true,
    showUnreadBadge: true,
    showEvents: true,
    showEvJoin: true,
    showEvLeave: true,
    showEvMove: true,
    showEvPoke: true,
    showEvChat: true,
    showEvGroup: true,
    showEvSys: true,
    evScopeJoin: 'all',
    evScopeLeave: 'all',
    evScopeMove: 'all',
    evScopeGroup: 'all',
    evScopeSys: 'all',
    evScopeChat: 'all',
    eventsCount: 5,
    eventsDelay: 20,
  };

  /** v1.4.7: appearance presets — one-click bundles (theme + colors + feel). */
  const PRESETS = {
    glass: { theme: 'glass', accent: '#7c6cff', nickColor: '#e8ecff', msgColor: '#b6b6bd', talkOutline: '#57e389', blur: 18, opacity: 0.72, glow: 0.7 },
    midnight: { theme: 'midnight', accent: '#4f7dff', nickColor: '#dfe6ff', msgColor: '#98a2c0', talkOutline: '#4f7dff', blur: 14, opacity: 0.85, glow: 0.4 },
    frost: { theme: 'frost', accent: '#38bdf8', nickColor: '#0b2740', msgColor: '#2b5a7d', talkOutline: '#0ea5e9', blur: 10, opacity: 0.9, glow: 0.3 },
    neon: { theme: 'neon', accent: '#ff2fd6', nickColor: '#39ff9c', msgColor: '#c9b8ff', talkOutline: '#39ff9c', blur: 22, opacity: 0.66, glow: 1 },
    classic: { theme: 'glass', accent: '#6c7cff', nickColor: '#ffffff', msgColor: '#a7b0c8', talkOutline: '#f5c451', blur: 8, opacity: 0.92, glow: 0.2 },
    soft: { theme: 'frost', accent: '#b78cff', nickColor: '#3b2f52', msgColor: '#6b5f88', talkOutline: '#b78cff', blur: 16, opacity: 0.8, glow: 0.5 },
  };

  const THEME_ACCENTS = {
    glass: '#7c6cff',
    midnight: '#4f7dff',
    frost: '#0ea5e9',
    neon: '#ff2d95',
    emerald: '#10b981',
    carmine: '#f43f5e',
    amber: '#f59e0b',
    mono: '#e5e5e5',
  };

  const ENUMS = {
    theme: ['glass', 'midnight', 'frost', 'neon', 'emerald', 'carmine', 'amber', 'mono'],
    corner: ['tl', 'tr', 'bl', 'br'],
    language: ['auto', 'ru', 'en'],
    eqStyle: ['bars', 'ring', 'dots', 'none'],
    evScopeJoin: ['all', 'own'],
    evScopeLeave: ['all', 'own'],
    evScopeMove: ['all', 'own'],
    evScopeGroup: ['all', 'own'],
    evScopeSys: ['all', 'own'],
    evScopeChat: ['all', 'own'],
  };

  const NUM_RANGES = {
    glow: [0, 1],
    opacity: [0.15, 1],
    blur: [0, 40],
    radius: [0, 26],
    scale: [0.75, 1.5],
    fontSize: [11, 18],
    marginX: [0, 20000],
    marginY: [0, 20000],
    chatDelay: [2, 60],
    eventsCount: [1, 10],
    eventsDelay: [5, 60],
  };

  const BOOLS = [
    'showHeader', 'showServerName', 'showUserCount', 'showBanner', 'showAvatars',
    'showBadges', 'showSelf', 'hideQuery', 'animations', 'talkingFirst',
    'onlyTalking', 'compact', 'hideEmpty', 'showWhispers', 'showChat', 'chatPrivate',
    'showUnreadBadge', 'showEvents', 'showEvJoin', 'showEvLeave', 'showEvMove',
    'showEvPoke', 'showEvChat', 'showEvGroup', 'showEvSys',
  ];

  let local = {};
  let server = {};
  let urlOverrides = {};
  let clean = false;
  let appearance = { ...DEFAULTS };
  const saveListeners = [];

  function isPlainObject(v) {
    return v && typeof v === 'object' && !Array.isArray(v);
  }

  function merge(base, patch) {
    const out = { ...base };
    if (!isPlainObject(patch)) return out;
    for (const [k, v] of Object.entries(patch)) {
      if (!(k in DEFAULTS) || v === undefined || v === null) continue;
      out[k] = v;
    }
    return out;
  }

  function sanitize(input) {
    const out = {};
    if (!isPlainObject(input)) return out;
    for (const [k, v] of Object.entries(input)) {
      if (!(k in DEFAULTS)) continue;
      if (k in ENUMS) {
        if (ENUMS[k].includes(v)) out[k] = v;
      } else if (k in NUM_RANGES) {
        const n = Number(v);
        const [min, max] = NUM_RANGES[k];
        if (Number.isFinite(n)) out[k] = Math.min(max, Math.max(min, n));
      } else if (BOOLS.includes(k)) {
        out[k] = !!v;
      } else if (k === 'accent' || k === 'nickColor' || k === 'msgColor' || k === 'talkOutline') {
        if (typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v)) out[k] = v;
      } else if (typeof v === 'string') {
        out[k] = v;
      }
    }
    return out;
  }

  function parseUrl() {
    const q = new URLSearchParams(location.search);
    const map = {
      theme: 'theme', accent: 'accent', glow: 'glow', opacity: 'opacity', blur: 'blur',
      radius: 'radius', scale: 'scale', font: 'fontSize', corner: 'corner',
      marginx: 'marginX', marginy: 'marginY',
      lang: 'language', eq: 'eqStyle',
    };
    const out = {};
    for (const [param, key] of Object.entries(map)) {
      const raw = q.get(param);
      if (raw === null) continue;
      let value = raw;
      if (key in NUM_RANGES) value = Number(raw);
      if (BOOLS.includes(key)) value = raw === '1' || raw === 'true';
      const s = sanitize({ [key]: value });
      if (key in s) out[key] = s[key];
    }
    // Legacy ?margin=N sets both axes.
    const legacy = q.get('margin');
    if (legacy !== null) {
      const s = sanitize({ marginX: Number(legacy), marginY: Number(legacy) });
      if ('marginX' in s) out.marginX = s.marginX;
      if ('marginY' in s) out.marginY = s.marginY;
    }
    if (q.get('compact') !== null) out.compact = q.get('compact') === '1';
    if (q.get('anim') !== null) out.animations = q.get('anim') !== '0';
    if (q.get('clean') === '1') clean = true;
    if (q.get('bare') === '1') {
      // Desktop settings window: settings UI only, no overlay page behind it.
      document.body.classList.add('bare');
      const stage = document.getElementById('stage');
      if (stage) stage.style.display = 'none';
    }
    return out;
  }

  function recompute() {
    appearance = merge(merge(merge(DEFAULTS, sanitize(server)), sanitize(local)), urlOverrides);
    return appearance;
  }

  /* ---------------- CSS application ---------------- */

  function hexToRgb(hex) {
    const m = /^#?([0-9a-fA-F]{6})$/.exec(hex || '');
    if (!m) return '124, 108, 255';
    const n = parseInt(m[1], 16);
    return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
  }

  function apply() {
    const a = appearance;
    const root = document.documentElement.style;
    root.setProperty('--accent', a.accent);
    root.setProperty('--accent-rgb', hexToRgb(a.accent));
    root.setProperty('--nick-color', a.nickColor);
    root.setProperty('--msg-color', a.msgColor);
    root.setProperty('--talk-outline', a.talkOutline);
    root.setProperty('--glow', a.glow);
    root.setProperty('--panel-opacity', a.opacity);
    root.setProperty('--blur', `${a.blur}px`);
    root.setProperty('--radius', `${a.radius}px`);
    root.setProperty('--scale', a.scale);
    root.setProperty('--font-size', `${a.fontSize}px`);
    const clamp = panelClamp();
    const mx = Math.min(Number(a.marginX) || 0, clamp.maxX);
    const my = Math.min(Number(a.marginY) || 0, clamp.maxY);
    root.setProperty('--margin', `${mx}px`);
    root.setProperty('--margin-x', `${mx}px`);
    root.setProperty('--margin-y', `${my}px`);

    document.body.dataset.theme = a.theme;
    document.body.dataset.corner = a.corner;
    document.body.classList.toggle('compact', a.compact);
    document.body.classList.toggle('no-anim', !a.animations);
    document.body.classList.toggle('no-avatars', !a.showAvatars);
    document.body.classList.toggle('no-badges', !a.showBadges);
    document.body.classList.toggle('eq-none', a.eqStyle === 'none');
  }

  /* ---------------- persistence ---------------- */

  function persistLocal() {
    try {
      localStorage.setItem(LS_KEY, JSON.stringify(sanitize(local)));
    } catch {
      /* private mode */
    }
  }

  let saveTimer = null;
  function pushConfig() {
    fetch('/api/config', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ appearance }),
    }).catch(() => {});
  }

  /** immediate: toggles/buttons save at once (no debounce flicker); sliders
   *  stay debounced while dragging. */
  function notifyChange(immediate) {
    recompute();
    apply();
    persistLocal();
    // The in-memory server mirror follows the effective appearance so a later
    // broadcast echo can never roll back what the user just did.
    server = sanitize(appearance);
    for (const cb of saveListeners) cb(appearance);
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    if (immediate) {
      pushConfig();
    } else {
      saveTimer = setTimeout(() => {
        saveTimer = null;
        pushConfig();
      }, 250);
    }
  }

  /* ---------------- drawer UI ---------------- */

  function fmt(key, value) {
    if (key === 'opacity' || key === 'glow' || key === 'scale') return `${Math.round(value * 100)}%`;
    if (key === 'chatDelay' || key === 'eventsDelay') return `${value} s`;
    if (key === 'eventsCount') return `${value}`;
    return `${value}px`;
  }

  // Margin sliders follow the user's screen size: the overlay may be dragged
  // anywhere on the display, so the offsets can reach every pixel of it.
  let boundsCache = null;
  function overlayBounds() {
    if (boundsCache) return boundsCache;
    const sw = Number(window.screen?.availWidth) || 1920;
    const sh = Number(window.screen?.availHeight) || 1080;
    boundsCache = {
      maxX: Math.max(0, Math.round(sw) - (panelW || 292)),
      maxY: Math.max(0, Math.round(sh) - (panelH || 140)),
    };
    try {
      fetch('/api/displays')
        .then((r) => r.json())
        .then((body) => {
          const list = body?.displays || [];
          const d = list.find((x) => x.primary) || list[0];
          if (d?.width && d?.height) {
            const pw = body?.panelSize?.w || panelW || 292;
            const ph = body?.panelSize?.h || panelH || 140;
            boundsCache = {
              maxX: Math.max(0, Math.round(d.width) - pw),
              maxY: Math.max(0, Math.round(d.height) - ph),
            };
            paintControls();
          }
        })
        .catch(() => {});
    } catch {
      /* plain web — screen fallback stays */
    }
    return boundsCache;
  }

  /* Panel metrics are cached: reading offsetWidth/offsetHeight on every apply
   * forced a synchronous layout each slider tick (visible lag in v1.1.14). */
  let panelW = 0;
  let panelH = 0;
  function measurePanel() {
    const panel = document.getElementById('panel');
    if (panel && panel.offsetWidth) panelW = panel.offsetWidth;
    if (panel && panel.offsetHeight) panelH = panel.offsetHeight;
    boundsCache = null;
  }

  /** Viewport minus the panel size — the panel must never leave the surface
   *  it lives in. window.innerWidth/innerHeight match the drag handler's
   *  bounds exactly (the overlay window on desktop, the browser source in
   *  OBS), so a dropped panel never snaps elsewhere on apply. */
  function panelClamp() {
    const vw = Number(window.innerWidth) || Number(window.screen?.availWidth) || 1920;
    const vh = Number(window.innerHeight) || Number(window.screen?.availHeight) || 1080;
    return {
      maxX: Math.max(0, vw - (panelW || 292)),
      maxY: Math.max(0, vh - (panelH || 140)),
    };
  }

  function paintControls() {
    const a = appearance;

    document.querySelectorAll('[data-slider]').forEach((field) => {
      const key = field.getAttribute('data-slider');
      const input = field.querySelector('input[type="range"]');
      const output = field.querySelector('output');
      input.min = field.getAttribute('data-min');
      input.max = field.getAttribute('data-max');
      input.step = field.getAttribute('data-step');
      input.value = a[key];
      const pct = ((a[key] - Number(input.min)) / (Number(input.max) - Number(input.min))) * 100;
      input.style.setProperty('--fill', `${pct}%`);
      output.textContent = fmt(key, a[key]);
    });

    document.querySelectorAll('[data-toggle]').forEach((input) => {
      input.checked = !!a[input.getAttribute('data-toggle')];
    });

    const bounds = overlayBounds();
    for (const [key, max] of [['marginX', bounds.maxX], ['marginY', bounds.maxY]]) {
      const field = document.querySelector(`[data-slider="${key}"]`);
      const input = field?.querySelector('input[type="range"]');
      if (!input) continue;
      input.min = '0';
      input.max = String(max);
      input.step = '1';
      input.value = String(Math.min(Number(max), Number(a[key]) || 0));
      const pct = (Number(input.value) / Number(max)) * 100;
      input.style.setProperty('--fill', `${Math.max(0, Math.min(100, pct))}%`);
      const output = field.querySelector('output');
      if (output) output.textContent = fmt(key, Number(input.value));
    }

    document.querySelectorAll('#themeSeg button').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.value === a.theme);
    });

    document.querySelectorAll('[data-color]').forEach((inp) => {
      const key = inp.getAttribute('data-color');
      if (key in a) inp.value = a[key];
    });

    document.querySelectorAll('#accentSwatches .swatch').forEach((sw) => {
      sw.classList.toggle('active', sw.dataset.value.toLowerCase() === String(a.accent).toLowerCase());
    });
    const custom = document.getElementById('accentCustom');
    if (custom) custom.value = a.accent;

    document.querySelectorAll('#cornerGrid button').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.value === a.corner);
    });

    document.querySelectorAll('[data-enum]').forEach((sel) => {
      const key = sel.getAttribute('data-enum');
      if (key in a) sel.value = a[key];
    });

    const langSel = document.getElementById('languageSelect');
    if (langSel) langSel.value = a.language;
    const eqSel = document.getElementById('eqStyleSelect');
    if (eqSel) eqSel.value = a.eqStyle;
  }

  /** Apply a preset bundle in one save (v1.4.7). */
  function setMany(patchObj) {
    const s = sanitize(patchObj);
    for (const [k, v] of Object.entries(s)) local[k] = v;
    if ('theme' in s && !('accent' in s) && THEME_ACCENTS[s.theme]) local.accent = THEME_ACCENTS[s.theme];
    notifyChange(true);
    paintControls();
    if ('language' in s) I18N.setLanguage(appearance.language);
  }

  function set(key, value, immediate) {
    const s = sanitize({ [key]: value });
    if (!(key in s)) return;
    local[key] = s[key];
    if (key === 'theme' && THEME_ACCENTS[s[key]]) {
      // Themes carry their own signature accent — switch along with the theme.
      local.accent = THEME_ACCENTS[s[key]];
    }
    notifyChange(immediate !== false);
    paintControls();
    if (key === 'language') I18N.setLanguage(appearance.language);
  }

  function wireDrawer() {
    document.querySelectorAll('[data-slider]').forEach((field) => {
      const key = field.getAttribute('data-slider');
      const input = field.querySelector('input[type="range"]');
      input.addEventListener('input', () => set(key, Number(input.value), false));
    });

    document.querySelectorAll('[data-toggle]').forEach((input) => {
      input.addEventListener('change', () => set(input.getAttribute('data-toggle'), input.checked));
    });

    document.querySelectorAll('#themeSeg button').forEach((btn) => {
      btn.addEventListener('click', () => set('theme', btn.dataset.value));
    });

    document.querySelectorAll('#presetSeg button').forEach((btn) => {
      btn.addEventListener('click', () => {
        const p = PRESETS[btn.dataset.value];
        if (p) setMany({ ...p });
      });
    });

    document.querySelectorAll('[data-color]').forEach((inp) => {
      inp.addEventListener('input', () => set(inp.getAttribute('data-color'), inp.value));
    });

    document.querySelectorAll('#accentSwatches .swatch').forEach((sw) => {
      sw.addEventListener('click', () => set('accent', sw.dataset.value));
    });

    const custom = document.getElementById('accentCustom');
    if (custom) custom.addEventListener('input', () => set('accent', custom.value));

    document.querySelectorAll('#cornerGrid button').forEach((btn) => {
      btn.addEventListener('click', () => set('corner', btn.dataset.value));
    });

    document.querySelectorAll('[data-enum]').forEach((sel) => {
      sel.addEventListener('change', () => set(sel.getAttribute('data-enum'), sel.value));
    });

    const langSel = document.getElementById('languageSelect');
    if (langSel) langSel.addEventListener('change', () => set('language', langSel.value));

    const eqSel = document.getElementById('eqStyleSelect');
    if (eqSel) eqSel.addEventListener('change', () => set('eqStyle', eqSel.value));

    // tabs
    document.querySelectorAll('#tabs .tab').forEach((tab) => {
      tab.addEventListener('click', () => {
        document.querySelectorAll('#tabs .tab').forEach((t) => t.classList.toggle('active', t === tab));
        document.querySelectorAll('.tab-panel').forEach((p) => {
          p.classList.toggle('active', p.dataset.panel === tab.dataset.tab);
        });
      });
    });

    // open / close
    const drawer = document.getElementById('settingsDrawer');
    const backdrop = document.getElementById('drawerBackdrop');
    const open = () => {
      drawer.hidden = false;
      backdrop.hidden = false;
      paintControls();
    };
    const close = () => {
      drawer.hidden = true;
      backdrop.hidden = true;
    };
    document.getElementById('settingsBtn').addEventListener('click', open);
    document.getElementById('closeDrawer')?.addEventListener('click', close);
    backdrop.addEventListener('click', close);
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') close();
    });

    // profile
    document.getElementById('exportBtn').addEventListener('click', () => {
      const blob = new Blob([JSON.stringify({ tsNotifier: 1, appearance }, null, 2)], { type: 'application/json' });
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = 'ts-notifier-profile.json';
      a.click();
      URL.revokeObjectURL(a.href);
    });

    document.getElementById('importBtn').addEventListener('click', () => document.getElementById('importFile').click());
    document.getElementById('importFile').addEventListener('change', (e) => {
      const file = e.target.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = () => {
        try {
          const parsed = JSON.parse(String(reader.result));
          const patch = sanitize(parsed.appearance ?? parsed);
          local = { ...local, ...patch };
          notifyChange();
          paintControls();
          I18N.setLanguage(appearance.language);
        } catch {
          /* ignore bad file */
        }
      };
      reader.readAsText(file);
      e.target.value = '';
    });

    document.getElementById('resetBtn').addEventListener('click', () => {
      local = {};
      server = { ...DEFAULTS };
      try {
        localStorage.removeItem(LS_KEY);
      } catch {
        /* ignore */
      }
      notifyChange(true);
      paintControls();
      I18N.setLanguage(appearance.language);
    });
  }

  /* ---------------- public API ---------------- */

  window.Settings = {
    DEFAULTS,
    get appearance() {
      return appearance;
    },
    get clean() {
      return clean;
    },
    init() {
      try {
        local = sanitize(JSON.parse(localStorage.getItem(LS_KEY) || '{}'));
      } catch {
        local = {};
      }
      urlOverrides = parseUrl();
      measurePanel();
      recompute();
      apply();
      wireDrawer();
      paintControls();
      I18N.setLanguage(appearance.language);
      window.addEventListener('resize', () => {
        measurePanel();
        paintControls();
      });
      // Another window (overlay ↔ settings) changed the appearance: apply it
      // immediately — no waiting for the server echo.
      window.addEventListener('storage', (e) => {
        if (e.key !== LS_KEY) return;
        try {
          local = { ...local, ...sanitize(JSON.parse(localStorage.getItem(LS_KEY) || '{}')) };
        } catch {
          /* ignore */
        }
        recompute();
        apply();
        paintControls();
      });
    },
    /** Server pushed its config (hello / config message). */
    syncServer(cfg) {
      const incoming = sanitize(cfg?.appearance);
      // Server values win over the local cache: what was saved is what shows.
      local = { ...local, ...incoming };
      server = incoming;
      recompute();
      apply();
      paintControls();
      persistLocal();
    },
    onChange(cb) {
      saveListeners.push(cb);
    },
    openDrawer() {
      const drawer = document.getElementById('settingsDrawer');
      const backdrop = document.getElementById('drawerBackdrop');
      if (drawer && backdrop) {
        drawer.hidden = false;
        backdrop.hidden = false;
        paintControls();
      }
    },
  };
})();
