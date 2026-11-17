/* Group icons, display target, range painting. */
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
      // The group list may be empty (TS6 never sends names; TS3 may deny
      // servergrouplist) — synthesise a stub so the id → class map still works.
      const g = groups.find((x) => x.id === gid) ?? { id: gid, name: '', iconId: 0 };
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

  /* ---------------- TS6 group → class map ---------------- */

  let ts6GroupMapSig = '';
  function renderTs6GroupMap() {
    const host = document.getElementById('ts6GroupMap');
    if (!host) return;
    const groups = TSN.state?.groups || [];
    const map = TSN.config?.classes?.map || {};
    // Runs on every state event — only touch the DOM when something changed.
    const sig = groups.map((g) => `${g.id}:${g.name}:${map[String(g.id)] || ''}`).join('|');
    if (sig === ts6GroupMapSig) return;
    ts6GroupMapSig = sig;
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
  TSN.initFeatures = initFeatures;

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initFeatures);
  } else {
    setTimeout(initFeatures, 0);
  }
})();
