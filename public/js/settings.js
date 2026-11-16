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
  };

  const ENUMS = {
    theme: ['glass', 'midnight', 'frost', 'neon'],
    corner: ['tl', 'tr', 'bl', 'br'],
    language: ['auto', 'ru', 'en'],
    eqStyle: ['bars', 'ring', 'dots', 'none'],
  };

  const NUM_RANGES = {
    glow: [0, 1],
    opacity: [0.15, 1],
    blur: [0, 40],
    radius: [0, 26],
    scale: [0.75, 1.5],
    fontSize: [11, 18],
    marginX: [0, 240],
    marginY: [0, 240],
    chatDelay: [2, 60],
  };

  const BOOLS = [
    'showHeader', 'showServerName', 'showUserCount', 'showBanner', 'showAvatars',
    'showBadges', 'showSelf', 'hideQuery', 'animations', 'talkingFirst',
    'onlyTalking', 'compact', 'hideEmpty', 'showWhispers', 'showChat', 'chatPrivate',
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
      } else if (k === 'accent') {
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
    root.setProperty('--glow', a.glow);
    root.setProperty('--panel-opacity', a.opacity);
    root.setProperty('--blur', `${a.blur}px`);
    root.setProperty('--radius', `${a.radius}px`);
    root.setProperty('--scale', a.scale);
    root.setProperty('--font-size', `${a.fontSize}px`);
    root.setProperty('--margin', `${a.marginX}px`);
    root.setProperty('--margin-x', `${a.marginX}px`);
    root.setProperty('--margin-y', `${a.marginY}px`);

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
  function notifyChange() {
    recompute();
    apply();
    persistLocal();
    for (const cb of saveListeners) cb(appearance);
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      fetch('/api/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ appearance }),
      }).catch(() => {});
    }, 450);
  }

  /* ---------------- drawer UI ---------------- */

  function fmt(key, value) {
    if (key === 'opacity' || key === 'glow' || key === 'scale') return `${Math.round(value * 100)}%`;
    if (key === 'chatDelay') return `${value} s`;
    return `${value}px`;
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

    document.querySelectorAll('#themeSeg button').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.value === a.theme);
    });

    document.querySelectorAll('#accentSwatches .swatch').forEach((sw) => {
      sw.classList.toggle('active', sw.dataset.value.toLowerCase() === String(a.accent).toLowerCase());
    });
    const custom = document.getElementById('accentCustom');
    if (custom) custom.value = a.accent;

    document.querySelectorAll('#cornerGrid button').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.value === a.corner);
    });

    const langSel = document.getElementById('languageSelect');
    if (langSel) langSel.value = a.language;
    const eqSel = document.getElementById('eqStyleSelect');
    if (eqSel) eqSel.value = a.eqStyle;
  }

  function set(key, value) {
    const s = sanitize({ [key]: value });
    if (!(key in s)) return;
    local[key] = s[key];
    notifyChange();
    paintControls();
    if (key === 'language') I18N.setLanguage(appearance.language);
  }

  function wireDrawer() {
    document.querySelectorAll('[data-slider]').forEach((field) => {
      const key = field.getAttribute('data-slider');
      const input = field.querySelector('input[type="range"]');
      input.addEventListener('input', () => set(key, Number(input.value)));
    });

    document.querySelectorAll('[data-toggle]').forEach((input) => {
      input.addEventListener('change', () => set(input.getAttribute('data-toggle'), input.checked));
    });

    document.querySelectorAll('#themeSeg button').forEach((btn) => {
      btn.addEventListener('click', () => set('theme', btn.dataset.value));
    });

    document.querySelectorAll('#accentSwatches .swatch').forEach((sw) => {
      sw.addEventListener('click', () => set('accent', sw.dataset.value));
    });

    const custom = document.getElementById('accentCustom');
    if (custom) custom.addEventListener('input', () => set('accent', custom.value));

    document.querySelectorAll('#cornerGrid button').forEach((btn) => {
      btn.addEventListener('click', () => set('corner', btn.dataset.value));
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
    document.getElementById('closeDrawer').addEventListener('click', close);
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
      try {
        localStorage.removeItem(LS_KEY);
      } catch {
        /* ignore */
      }
      notifyChange();
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
      recompute();
      apply();
      wireDrawer();
      paintControls();
      I18N.setLanguage(appearance.language);
    },
    /** Server pushed its config (hello / config message). */
    syncServer(cfg) {
      server = sanitize(cfg?.appearance);
      recompute();
      apply();
      paintControls();
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
