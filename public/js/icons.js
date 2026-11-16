/* SVG icon set (feather-style, 24×24 stroke) + deterministic avatar art. */
(function () {
  const P = {
    mic: '<path d="M12 2a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z"/><path d="M19 10v1a7 7 0 0 1-14 0v-1"/><line x1="12" y1="18" x2="12" y2="22"/>',
    micOff:
      '<path d="M12 2a3 3 0 0 0-3 3v4"/><path d="M15 9V5a3 3 0 0 0-5.12-2.12"/><path d="M19 10v1a7 7 0 0 1-2.22 5.1"/><path d="M5 10v1a7 7 0 0 0 11.26 5.86"/><line x1="12" y1="18" x2="12" y2="22"/><line x1="3" y1="3" x2="21" y2="21"/>',
    headphones:
      '<path d="M3 18v-6a9 9 0 0 1 18 0v6"/><path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3zM3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z"/>',
    headphonesOff:
      '<path d="M3 14v4"/><path d="M3 18v-6a9 9 0 0 1 13.5-7.8"/><path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3zM7 21a2 2 0 0 0 2-2v-1"/><line x1="3" y1="3" x2="21" y2="21"/>',
    moon: '<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>',
    shield: '<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>',
    star: '<polygon points="12 2.5 15.09 8.5 21.7 9.3 16.9 14.1 18.2 20.8 12 17.5 5.8 20.8 7.1 14.1 2.3 9.3 8.91 8.5"/>',
    gear:
      '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>',
    x: '<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>',
    wave:
      '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/>',
    users:
      '<path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>',
    download:
      '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>',
    upload:
      '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>',
    alert:
      '<path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>',
    info: '<circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/>',
    logo:
      '<rect x="3" y="3" width="18" height="18" rx="5" stroke="none" fill="rgba(255,255,255,0.28)"/><path d="M8 10v4"/><path d="M12 7v10"/><path d="M16 10v4"/>',
  };

  /** Wrap raw paths into a standard stroked SVG. */
  function icon(name, size = 16, strokeWidth = 2) {
    const body = P[name] || P.info;
    return (
      `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" ` +
      `stroke-width="${strokeWidth}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`
    );
  }

  /* ---------------- avatars ---------------- */

  function hashHue(str) {
    let h = 0;
    for (let i = 0; i < str.length; i++) h = (Math.imul(h, 31) + str.charCodeAt(i)) | 0;
    return Math.abs(h) % 360;
  }

  function avatarGradient(id, name) {
    const hue = hashHue(`${id}|${name}`);
    const hue2 = (hue + 42) % 360;
    return `linear-gradient(135deg, hsl(${hue} 72% 56%), hsl(${hue2} 70% 41%))`;
  }

  function initials(name) {
    const parts = String(name || '')
      .trim()
      .split(/\s+/)
      .filter(Boolean);
    if (parts.length === 0) return '?';
    const first = [...parts[0]][0] || '?';
    if (parts.length === 1) return first.toUpperCase();
    const last = [...parts[parts.length - 1]][0] || '';
    return (first + last).toUpperCase();
  }

  window.TSIcons = { icon, avatarGradient, initials };
})();

/* 17 game class glyphs (stroke) + gold commander crown. */
(function () {
  const C = {
    archer: 'M6 4a12 12 0 0 1 0 16M12 12h9M18 9l3 3-3 3',
    priest: 'M12 2v20M6 7h12M8 13h8',
    warrior: 'M4 20 18 6M15 5l3 3M20 20 6 6M3 5l3 3',
    mage: 'M6 15 12 3l6 12M4 15h16M12 17v3',
    werewolf: 'M4 5l4 3M20 5l-4 3M6 9v3a6 6 0 0 0 12 0V9M9 12h.01M15 12h.01M10 16h4',
    druid: 'M20 4C10 4 4 10 4 20c10 0 16-6 16-16zM8 16c2-4 5-7 8-9',
    assassin: 'M12 2v12M8 6h8M9 14h6l-1 6h-4l-1-6z',
    shaman: 'M6 8 4 3l3 2M18 8l2-5-3 2M6 8v4a6 6 0 0 0 12 0V8M9 11h.01M15 11h.01M9 15c2 2 4 2 6 0',
    ghost: 'M5 22V10a7 7 0 0 1 14 0v12l-2.3-2-2.4 2-2.3-2-2.4 2L5 22zM9.5 10h.01M14.5 10h.01',
    reaper: 'M4 21 17 8M7 3c8-1 13 4 13 10-4-4-9-5-14-3',
    paladin: 'M12 2 4 6v6c0 5 4 8 8 10 4-2 8-5 8-10V6l-8-4zM12 7v8M9 10h6',
    gunslinger: 'M3 7h13l2 3h4v2h-4l-1 3h-4l-1 5H8l1-7H3z',
    guardian: 'M5 21V8l7-5 7 5v13M5 21h14M10 21v-6h4v6M9 11h2M13 11h2',
    mystic: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
    bard: 'M9 18V6l10-2v12M9 18a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0zM19 16a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0z',
    blood: 'M12 2c4 6 7 9 7 13a7 7 0 0 1-14 0c0-4 3-7 7-13zM9 15c1-2 2-3.5 3-5 1 1.8 2 3 3 5',
    monkey: 'M12 4a6 6 0 0 0-6 6v2a6 6 0 0 0 12 0v-2a6 6 0 0 0-6-6zM6 9a2.5 2.5 0 1 0 0 5M18 9a2.5 2.5 0 1 1 0 5M9 11h.01M15 11h.01M10 14.5c1 1 1 2 2 2s1-1 2-2',
  };
  const LABELS = {
    archer: ['Лучник', 'Archer'], priest: ['Прист', 'Priest'], warrior: ['Воин', 'Warrior'],
    mage: ['Маг', 'Mage'], werewolf: ['Оборотень', 'Werewolf'], druid: ['Друид', 'Druid'],
    assassin: ['Убийца', 'Assassin'], shaman: ['Шаман', 'Shaman'], ghost: ['Призрак', 'Ghost'],
    reaper: ['Жнец', 'Reaper'], paladin: ['Паладин', 'Paladin'], gunslinger: ['Стрелок', 'Gunslinger'],
    guardian: ['Страж', 'Guardian'], mystic: ['Мистик', 'Mystic'], bard: ['Бард', 'Bard'],
    blood: ['Дух крови', 'Blood Spirit'], monkey: ['Обезьяна', 'Monkey'],
  };
  function classGlyph(key, size) {
    const d = C[key];
    if (!d) return '';
    const s = size || 18;
    return (
      '<svg class="class-glyph cls-' + key + '" width="' + s + '" height="' + s + '" viewBox="0 0 24 24" ' +
      'fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
      '<path d="' + d + '"/></svg>'
    );
  }
  function crown(size) {
    const s = size || 13;
    return (
      '<svg class="crown" width="' + s + '" height="' + s + '" viewBox="0 0 24 24" aria-hidden="true">' +
      '<path fill="#f6c453" stroke="none" d="M3 18 4.5 6.5l4.6 2.8L12 3.5l2.9 5.8 4.6-2.8L21 18H3zM4.8 19.6h14.4v1.6H4.8z"/>' +
      '</svg>'
    );
  }
  const T = window.TSIcons;
  T.CLASSES = Object.keys(C);
  T.classGlyph = classGlyph;
  T.classLabel = function (key, lang) {
    const l = LABELS[key];
    return l ? (lang === 'en' ? l[1] : l[0]) : key;
  };
  T.crown = crown;
})();
