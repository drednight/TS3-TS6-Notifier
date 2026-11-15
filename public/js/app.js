/* Overlay renderer + live state + connection settings. */
(function () {
  const $ = (id) => document.getElementById(id);
  const { icon, avatarGradient, initials } = window.TSIcons;
  const t = (key) => window.I18N.t(key);

  let state = null;
  let config = null;
  const rows = new Map(); // userId -> { el, avatar, ini, pip, nick, sub, badges, eq, sig }
  const leavingTimers = new Map();

  /* ---------------- static icon injection ---------------- */

  function injectIcons() {
    $('settingsBtn').innerHTML = icon('gear', 16.5);
    $('closeDrawer').innerHTML = icon('x', 16);
    document.querySelector('.drawer-logo').innerHTML = icon('logo', 20, 0);
    document.querySelector('.channel-name .wave-ico').innerHTML = icon('wave', 14, 2.2);
    document.querySelector('.empty-icon').innerHTML = icon('users', 20);
    document.querySelectorAll('[data-ico]').forEach((el) => {
      el.innerHTML = icon(el.getAttribute('data-ico'), 14);
    });
  }

  /* ---------------- state helpers ---------------- */

  function visibleUsers() {
    if (!state) return [];
    const a = Settings.appearance;
    let users = state.users.slice();
    if (a.hideQuery) users = users.filter((u) => !u.isQuery);
    if (!a.showSelf) users = users.filter((u) => !u.isSelf);
    if (a.onlyTalking) users = users.filter((u) => u.talking || u.isSelf);
    users.sort((x, y) => {
      if (a.talkingFirst && x.talking !== y.talking) return y.talking - x.talking;
      if (x.isSelf !== y.isSelf) return x.isSelf ? -1 : 1;
      return x.nickname.localeCompare(y.nickname, undefined, { sensitivity: 'base' });
    });
    return users;
  }

  function statusKey(status) {
    return (
      {
        connecting: 'statusConnecting',
        live: 'statusLive',
        'waiting-auth': 'statusWaitingAuth',
        offline: 'statusOffline',
        error: 'statusError',
      }[status] || 'statusOffline'
    );
  }

  function sourceKey(source) {
    return { mock: 'sourceMock', ts3: 'sourceTs3', ts6: 'sourceTs6' }[source] || 'sourceMock';
  }

  function statusHint() {
    if (state?.detail) return state.detail;
    if (!state) return '';
    return { mock: 'hintMock', ts3: 'hintTs3', ts6: 'hintTs6' }[state.source] || '';
  }

  /* ---------------- row rendering ---------------- */

  function badgeHtml(u) {
    const a = Settings.appearance;
    if (!a.showBadges) return '';
    const parts = [];
    if (u.inputMuted) parts.push(`<span class="badge mic-off">${icon('micOff', 13, 2.1)}</span>`);
    if (u.outputMuted) parts.push(`<span class="badge deaf">${icon('headphonesOff', 13, 2.1)}</span>`);
    if (u.away) parts.push(`<span class="badge away">${icon('moon', 12, 2.1)}</span>`);
    if (u.channelCommander) parts.push(`<span class="badge commander">${icon('star', 12, 2.1)}</span>`);
    if (u.prioritySpeaker) parts.push(`<span class="badge priority">${icon('shield', 12, 2.1)}</span>`);
    return parts.join('');
  }

  function pipColor(u) {
    if (u.talking) return 'var(--accent)';
    if (u.inputMuted) return 'var(--danger)';
    if (u.away) return 'var(--warn)';
    return 'var(--ok)';
  }

  function buildRow(u) {
    const el = document.createElement('div');
    el.className = 'row entering';
    el.innerHTML = `
      <div class="avatar"><span class="ini"></span><span class="status-pip"></span></div>
      <div class="meta">
        <div class="nick"></div>
        <div class="sub-line" hidden></div>
      </div>
      <div class="badges"></div>
      <div class="eq"><span class="bar"></span><span class="bar"></span><span class="bar"></span></div>`;
    const pack = {
      el,
      avatar: el.querySelector('.avatar'),
      ini: el.querySelector('.ini'),
      pip: el.querySelector('.status-pip'),
      nick: el.querySelector('.nick'),
      sub: el.querySelector('.sub-line'),
      badges: el.querySelector('.badges'),
      eq: el.querySelector('.eq'),
      sig: '',
    };
    setTimeout(() => el.classList.remove('entering'), 320);
    return pack;
  }

  function updateRow(pack, u) {
    const a = Settings.appearance;
    const sig = [
      u.talking, u.inputMuted, u.outputMuted, u.away, u.awayMessage,
      u.channelCommander, u.prioritySpeaker, u.isSelf, u.nickname,
      u.avatarHash, a.eqStyle, a.showBadges, a.compact, window.I18N.lang,
    ].join('|');
    if (sig === pack.sig) return;
    pack.sig = sig;

    const el = pack.el;
    el.classList.toggle('talking', u.talking);
    el.classList.toggle('self', u.isSelf);
    el.classList.toggle('muted', u.inputMuted);
    el.classList.toggle('away', u.away);

    pack.avatar.style.background = avatarGradient(u.avatarHash || u.id, u.nickname);
    pack.ini.textContent = initials(u.nickname);
    pack.pip.style.background = pipColor(u);

    pack.nick.textContent = u.nickname;
    pack.nick.dataset.you = t('you');

    const sub = u.away && u.awayMessage && !a.compact ? u.awayMessage : '';
    pack.sub.hidden = !sub;
    pack.sub.textContent = sub;

    pack.badges.innerHTML = badgeHtml(u);
    pack.eq.className = `eq ${a.eqStyle === 'none' ? '' : a.eqStyle}`;
  }

  /* ---------------- main render ---------------- */

  function render() {
    const a = Settings.appearance;
    if (!state) return;

    // header
    $('channelNameText').textContent = state.channelName || t('noChannel');
    $('panelHeader').hidden = !a.showHeader;

    const serverName = $('serverName');
    serverName.hidden = !a.showServerName || !state.serverName;
    serverName.textContent = state.serverName || '';

    const users = visibleUsers();
    const count = $('userCount');
    count.hidden = !a.showUserCount;
    count.textContent = users.length;

    const dot = $('connDot');
    dot.className = `conn-dot ${state.status}`;
    dot.title = `${t(sourceKey(state.source))} · ${t(statusKey(state.status))}`;

    // banner
    const showBanner = a.showBanner && !Settings.clean && state.status !== 'live';
    $('banner').hidden = !showBanner;
    if (showBanner) {
      $('banner').querySelector('.banner-icon').innerHTML = icon('info', 15, 2);
      $('bannerTitle').textContent = t(statusKey(state.status));
      $('bannerDetail').textContent = statusHint();
    }

    // rows
    const list = $('userList');
    const seen = new Set();
    for (const u of users) {
      seen.add(u.id);
      let pack = rows.get(u.id);
      if (!pack) {
        pack = buildRow(u);
        rows.set(u.id, pack);
        list.appendChild(pack.el);
      }
      if (leavingTimers.has(u.id)) {
        clearTimeout(leavingTimers.get(u.id));
        leavingTimers.delete(u.id);
        pack.el.classList.remove('leaving');
      }
      updateRow(pack, u);
    }

    // Keep DOM order in sync with the sorted list (talking-first etc.).
    for (const u of users) {
      const pack = rows.get(u.id);
      if (pack) list.appendChild(pack.el);
    }

    for (const [id, pack] of rows) {
      if (seen.has(id) || leavingTimers.has(id)) continue;
      pack.el.classList.add('leaving');
      leavingTimers.set(
        id,
        setTimeout(() => {
          pack.el.remove();
          rows.delete(id);
          leavingTimers.delete(id);
        }, 300),
      );
    }

    // empty / panel visibility
    const empty = users.length === 0;
    $('emptyState').hidden = !empty || state.status !== 'live';
    list.hidden = empty;
    $('panel').style.display = empty && a.hideEmpty && state.status === 'live' ? 'none' : '';

    // connection tab
    const pill = $('statusPill');
    pill.className = `status-pill ${state.status}`;
    pill.textContent = t(statusKey(state.status));
    $('statusDetail').textContent = statusHint();
  }

  /* ---------------- connection tab ---------------- */

  let connTimer = null;
  function putConfig(patch) {
    if (connTimer) clearTimeout(connTimer);
    connTimer = setTimeout(() => {
      fetch('/api/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(patch),
      }).catch(() => {});
    }, 400);
  }

  function fillConnectionTab() {
    if (!config) return;
    $('sourceSelect').value = config.source;
    $('ts3Host').value = config.ts3.host;
    $('ts3Port').value = config.ts3.port;
    $('ts3ApiKey').value = config.ts3.apiKey;
    $('ts6Host').value = config.ts6.host;
    $('ts6Port').value = config.ts6.port;
    $('ts6ApiKey').value = config.ts6.apiKey;
    $('ts3Fields').classList.toggle('visible', config.source === 'ts3');
    $('ts6Fields').classList.toggle('visible', config.source === 'ts6');
  }

  function wireConnectionTab() {
    $('sourceSelect').addEventListener('change', () => {
      if (!config) return;
      config.source = $('sourceSelect').value;
      fillConnectionTab();
      putConfig({ source: config.source });
    });

    const ts3Patch = () => ({
      ts3: {
        host: $('ts3Host').value.trim() || '127.0.0.1',
        port: Number($('ts3Port').value) || 25639,
        apiKey: $('ts3ApiKey').value.trim(),
      },
    });
    const ts6Patch = () => ({
      ts6: {
        host: $('ts6Host').value.trim() || '127.0.0.1',
        port: Number($('ts6Port').value) || 5899,
        apiKey: $('ts6ApiKey').value.trim(),
      },
    });

    ['ts3Host', 'ts3Port', 'ts3ApiKey'].forEach((id) => {
      $(id).addEventListener('change', () => putConfig(ts3Patch()));
    });
    ['ts6Host', 'ts6Port', 'ts6ApiKey'].forEach((id) => {
      $(id).addEventListener('change', () => putConfig(ts6Patch()));
    });

    $('reconnectBtn').addEventListener('click', () => {
      fetch('/api/reconnect', { method: 'POST' }).catch(() => {});
    });
  }

  /* ---------------- websocket ---------------- */

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
      if (msg.type === 'hello') {
        config = msg.config;
        Settings.syncServer(config);
        fillConnectionTab();
        if (msg.state) {
          state = msg.state;
          render();
        }
      } else if (msg.type === 'state') {
        state = msg.state;
        render();
      } else if (msg.type === 'config') {
        config = msg.config;
        Settings.syncServer(config);
        fillConnectionTab();
      }
    };

    ws.onclose = () => setTimeout(connectWS, 1600);
    ws.onerror = () => ws.close();
  }

  /* ---------------- boot ---------------- */

  injectIcons();
  Settings.init();
  wireConnectionTab();

  if (Settings.clean) {
    $('settingsBtn').style.display = 'none';
  }

  Settings.onChange(() => render());
  window.addEventListener('tsn:lang', () => render());
  window.addEventListener('hashchange', () => location.reload());

  connectWS();
})();
