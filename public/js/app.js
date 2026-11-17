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
    if (u.channelCommander) parts.push(`<span class="badge commander" title="Channel Commander">${window.TSIcons.crown(14)}</span>`);
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
    const iconUrl = window.TSN && window.TSN.groupIconUrl ? window.TSN.groupIconUrl(u) : null;
    const classKey = !iconUrl && window.TSN && window.TSN.classKeyFor ? window.TSN.classKeyFor(u) : null;
    const sig = [
      u.talking, u.whispering, u.inputMuted, u.outputMuted, u.away, u.awayMessage,
      u.channelCommander, u.prioritySpeaker, u.isSelf, u.nickname,
      u.avatarHash, u.serverGroups, iconUrl, classKey, a.eqStyle, a.showBadges, a.compact, window.I18N.lang,
    ].join('|');
    if (sig === pack.sig) return;
    pack.sig = sig;

    const el = pack.el;
    el.classList.toggle('talking', u.talking);
    el.classList.toggle('whisper', !!u.whispering);
    el.classList.toggle('self', u.isSelf);
    el.classList.toggle('muted', u.inputMuted);
    el.classList.toggle('away', u.away);

    if (iconUrl) {
      pack.avatar.style.background = 'rgba(255,255,255,0.06)';
      pack.ini.innerHTML = '';
      const img = document.createElement('img');
      img.className = 'group-ico';
      img.alt = '';
      img.src = iconUrl;
      img.onerror = () => {
        img.remove();
        pack.ini.textContent = initials(u.nickname);
        pack.avatar.style.background = avatarGradient(u.avatarHash || u.id, u.nickname);
      };
      pack.ini.appendChild(img);
    } else if (classKey) {
      pack.avatar.style.background = 'rgba(255,255,255,0.06)';
      pack.ini.innerHTML = window.TSIcons.classGlyph(classKey, a.compact ? 15 : 19);
    } else {
      pack.avatar.style.background = avatarGradient(u.avatarHash || u.id, u.nickname);
      pack.ini.textContent = initials(u.nickname);
    }
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

  // State events can arrive in bursts (talk flaps, big servers) — coalesce
  // renders to at most one per animation frame so the DOM is not rebuilt
  // dozens of times per second (CPU + garbage churn).
  let renderQueued = false;
  function scheduleRender() {
    if (renderQueued) return;
    renderQueued = true;
    requestAnimationFrame(() => {
      renderQueued = false;
      render();
    });
  }

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

    // rows — whisperers go to their own fading section
    const list = $('userList');
    const whisperList = $('whisperList');
    const whisperBlock = $('whisperBlock');
    const whisperActive = a.showWhispers && users.some((u) => u.whispering);
    whisperBlock.hidden = !whisperActive;
    const containerFor = (u) => (whisperActive && u.whispering ? whisperList : list);
    const seen = new Set();
    for (const u of users) {
      seen.add(u.id);
      let pack = rows.get(u.id);
      if (!pack) {
        pack = buildRow(u);
        rows.set(u.id, pack);
        containerFor(u).appendChild(pack.el);
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
      if (pack) containerFor(u).appendChild(pack.el);
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

    renderChat();

    // connection tab
    const pill = $('statusPill');
    pill.className = `status-pill ${state.status}`;
    pill.textContent = t(statusKey(state.status));
    $('statusDetail').textContent = statusHint();
  }

  /* ---------------- chat toasts ---------------- */

  const chatRows = new Map(); // id -> { el, timer }

  /* ---------------- unread-message badge ---------------- */

  const seenChatIds = new Set();
  let unreadCount = 0;
  let badgeHovering = false;

  function clickThroughWanted() {
    return config?.desktop?.clickThrough !== false;
  }

  function setBadgeHover(on) {
    // The overlay is click-through so games keep the mouse — take real input
    // only while the cursor is on the badge, which makes the "mark as read"
    // click possible; released again on leave.
    if (badgeHovering === on) return;
    badgeHovering = on;
    try {
      fetch('/api/overlay/clickthrough', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ignore: on ? false : clickThroughWanted() }),
      }).catch(() => {});
    } catch {
      /* plain web / no fetch available — the badge stays informational */
    }
  }

  function clearUnread() {
    // "Read" is explicit only: the badge never fades on its own, it clears
    // when the user clicks it.
    unreadCount = 0;
    const el = $('chatUnread');
    if (el) el.hidden = true;
    setBadgeHover(false);
  }

  function trackUnread() {
    const el = $('chatUnread');
    if (!el) return;
    if (seenChatIds.size > 500) {
      let n = 0;
      for (const id of seenChatIds) {
        if (n++ < 250) seenChatIds.delete(id);
        else break;
      }
    }
    for (const m of state.chat || []) {
      if (!m.id || seenChatIds.has(m.id)) continue;
      seenChatIds.add(m.id);
      const isSelf = state.selfId != null && String(m.userId) === String(state.selfId);
      if (isSelf) continue;
      if (m.kind === 'system') continue;
      if (m.kind === 'private' && !Settings.appearance.chatPrivate) continue;
      unreadCount += 1;
    }
    if (unreadCount <= 0) return;
    el.hidden = false;
    el.title = t('unreadTitle');
    $('chatUnreadCount').textContent = unreadCount > 99 ? '99+' : String(unreadCount);
    el.classList.remove('bump');
    void el.offsetWidth; // restart the pulse animation
    el.classList.add('bump');
  }

  // strip zero-width / bidi control chars — they render as garbage glyphs
  const cleanChat = (s) => String(s || '').replace(/[\u200B-\u200F\u202A-\u202E\u2060-\u2064\uFEFF]/g, '');

  function renderChat() {
    const a = Settings.appearance;
    trackUnread();
    const feed = $('chatFeed');
    if (!feed) return;
    const show = a.showChat;
    feed.hidden = !show;
    if (!show) {
      for (const [, rec] of chatRows) {
        clearTimeout(rec.timer);
        rec.el.remove();
      }
      chatRows.clear();
      return;
    }
    const delayMs = Math.max(2, Number(a.chatDelay) || 8) * 1000;
    const now = Date.now();
    const msgs = (state.chat || []).slice(-8).filter((m) => now - m.at < delayMs + 1500);

    for (const [id, rec] of chatRows) {
      if (!msgs.some((m) => m.id === id)) {
        clearTimeout(rec.timer);
        rec.el.remove();
        chatRows.delete(id);
      }
    }
    const clean = cleanChat;
    for (const m of msgs) {
      if (m.kind === 'private' && !a.chatPrivate) continue;
      if (chatRows.has(m.id)) continue;
      const el = document.createElement('div');
      el.className = `chat-toast kind-${m.kind}`;
      const badge =
        m.kind === 'private' ? `<span class="chat-kind">${t('chatPrivateBadge')}</span>` :
        m.kind === 'server' ? `<span class="chat-kind">${t('chatServerBadge')}</span>` :
        m.kind === 'poke' ? `<span class="chat-kind">${t('pokeBadge')}</span>` : '';
      el.innerHTML = `${badge}<span class="chat-nick"></span><span class="chat-text"></span>`;
      if (m.kind === 'system' || (m.kind === 'poke' && m.event === 'poke' && !m.text)) {
        // Structured event row: localised here so connectors stay language-free.
        const name = clean(m.nickname || m.text || '…');
        const detail = clean(m.detail || '');
        let s2 = '';
        if (m.event === 'join') s2 = detail ? t('evJoinChannel').replace('{n}', name).replace('{c}', detail) : t('evJoinServer').replace('{n}', name);
        else if (m.event === 'leave') s2 = detail ? t('evLeaveChannel').replace('{n}', name).replace('{c}', detail) : t('evLeaveServer').replace('{n}', name);
        else if (m.event === 'move') s2 = t('evMove').replace('{n}', name).replace('{c}', detail || '…');
        else s2 = name;
        el.querySelector('.chat-text').textContent = s2;
        feed.appendChild(el);
        const rec = { el, timer: 0 };
        rec.timer = setTimeout(() => {
          el.classList.add('fading');
          setTimeout(() => {
            el.remove();
            chatRows.delete(m.id);
          }, 650);
        }, delayMs);
        chatRows.set(m.id, rec);
        continue;
      }
      // Resolve the sender's nick robustly: the event may omit it — fall back
      // to the live user list by id.
      const sender = (state.users || []).find((u) => String(u.id) === String(m.userId));
      const eventName = m.nickname && m.nickname !== '?' ? m.nickname : (m.senderName || m.name || '');
      // Server messages have no sender at all — label them instead of '…'
      const nick = eventName || sender?.nickname || (m.kind === 'server' ? t('chatServerBadge') : '');
      const isSelf = state.selfId != null && String(m.userId) === String(state.selfId);
      el.querySelector('.chat-nick').textContent = clean(nick)
        ? `${clean(nick)}${isSelf ? ` (${t('you')})` : ''}:`
        : '…:';
      el.querySelector('.chat-text').textContent = clean(m.text);
      feed.appendChild(el);
      const rec = { el, timer: 0 };
      rec.timer = setTimeout(() => {
        el.classList.add('fading');
        setTimeout(() => {
          el.remove();
          chatRows.delete(m.id);
        }, 650);
      }, delayMs);
      chatRows.set(m.id, rec);
    }
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
    $('elevateToggle').checked = config.desktop?.elevate !== false;
    $('moveHotkey').value = config.desktop?.moveHotkey || 'CommandOrControl+Shift+M';
    $('overlayWindowToggle').checked = config.desktop?.overlayWindow !== false;
    const obsUrl = `${location.origin}/?clean=1`;
    const obs = $('obsHint');
    if (obs) obs.textContent = t('obsHint').replace('{url}', obsUrl);
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

    $('elevateToggle').addEventListener('change', () => {
      putConfig({ desktop: { elevate: $('elevateToggle').checked } });
    });

    $('moveHotkey').addEventListener('change', () => {
      const value = $('moveHotkey').value.trim() || 'CommandOrControl+Shift+M';
      $('moveHotkey').value = value;
      putConfig({ desktop: { moveHotkey: value } });
    });

    $('overlayWindowToggle').addEventListener('change', () => {
      putConfig({ desktop: { overlayWindow: $('overlayWindowToggle').checked } });
    });
  }

  /* ---------------- websocket ---------------- */

  function syncFeatures() {
    const TSN = (window.TSN = window.TSN || {});
    TSN.config = config;
    TSN.state = state;
    try {
      document.dispatchEvent(new CustomEvent('tsn:config'));
      document.dispatchEvent(new CustomEvent('tsn:state'));
    } catch {
      /* ignore */
    }
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
      if (msg.type === 'hello') {
        config = msg.config;
        Settings.syncServer(config);
        fillConnectionTab();
        syncFeatures();
        if (msg.state) {
          state = msg.state;
          syncFeatures();
          scheduleRender();
        }
      } else if (msg.type === 'state') {
        state = msg.state;
        syncFeatures();
        scheduleRender();
      } else if (msg.type === 'config') {
        config = msg.config;
        Settings.syncServer(config);
        fillConnectionTab();
        syncFeatures();
      } else if (msg.type === 'move-mode') {
        if (moveMode) exitMoveMode();
        else enterMoveMode();
      } else if (msg.type === 'sound') {
        // Only the overlay page (clean mode) plays hotkey triggers —
        // otherwise open windows would double-play.
        if (Settings.clean) window.TSN?.Soundboard?.playById(msg.id);
      }
    };

    ws.onclose = () => setTimeout(connectWS, 1600);
    ws.onerror = () => ws.close();
  }

  /* ---------------- boot ---------------- */

  injectIcons();
  Settings.init();
  wireConnectionTab();
  /* ---------------- overlay move mode (drag with hotkey) ---------------- */

  function postClickthrough(ignore) {
    try {
      fetch('/api/overlay/clickthrough', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ignore }),
      }).catch(() => {});
    } catch {
      /* plain web — no desktop hook */
    }
  }

  let moveMode = false;
  let moveModeTimer = 0;

  function exitMoveMode() {
    if (!moveMode) return;
    moveMode = false;
    if (moveModeTimer) clearTimeout(moveModeTimer);
    moveModeTimer = 0;
    document.body.classList.remove('move-mode');
    const hint = $('moveHint');
    if (hint) hint.hidden = true;
    postClickthrough(clickThroughWanted());
  }

  function enterMoveMode() {
    if (document.body.classList.contains('bare')) return;
    moveMode = true;
    document.body.classList.add('move-mode');
    const hint = $('moveHint');
    if (hint) {
      hint.hidden = false;
      hint.textContent = t('moveHint');
    }
    // The overlay is click-through for gameplay — take the mouse only while
    // move mode is armed so the panel (and its buttons) can be used.
    postClickthrough(false);
    if (moveModeTimer) clearTimeout(moveModeTimer);
    moveModeTimer = setTimeout(exitMoveMode, 60000);
  }

  function wireOverlayDrag() {
    const header = $('panelHeader');
    if (!header) return;
    let dragging = false;
    let startX = 0;
    let startY = 0;
    let baseX = 0;
    let baseY = 0;
    let curX = 0;
    let curY = 0;

    header.addEventListener('mousedown', (e) => {
      // Dragging is ONLY possible in move mode (armed via the "Переместить
      // оверлей" hotkey) so the game never loses the mouse by accident.
      if (!moveMode || e.target.closest('button')) return;
      e.preventDefault();
      dragging = true;
      document.body.classList.add('dragging');
      startX = e.clientX;
      startY = e.clientY;
      baseX = curX = Number(Settings.appearance.marginX) || 0;
      baseY = curY = Number(Settings.appearance.marginY) || 0;
      const panel = $('panel');

      const move = (ev) => {
        const dx = ev.clientX - startX;
        const dy = ev.clientY - startY;
        const corner = document.body.dataset.corner || 'tr';
        const rightSide = corner === 'tr' || corner === 'br';
        // No arbitrary limits: the panel can travel to any point of the
        // screen — only clamped so it stays fully visible. The top edge is
        // anchored, so the panel fills downward as users join.
        const maxX = Math.max(0, (window.innerWidth || 0) - (panel?.offsetWidth || 0));
        const maxY = Math.max(0, (window.innerHeight || 0) - (panel?.offsetHeight || 0));
        curX = Math.min(maxX, Math.max(0, baseX + (rightSide ? -dx : dx)));
        curY = Math.min(maxY, Math.max(0, baseY + dy));
        document.documentElement.style.setProperty('--margin-x', `${curX}px`);
        document.documentElement.style.setProperty('--margin-y', `${curY}px`);
      };
      const up = () => {
        dragging = false;
        document.body.classList.remove('dragging');
        document.removeEventListener('mousemove', move);
        document.removeEventListener('mouseup', up);
        Settings.appearance.marginX = curX;
        Settings.appearance.marginY = curY;
        putConfig({ appearance: { ...Settings.appearance, marginX: curX, marginY: curY } });
        // A finished drag ends move mode right away — the key is a one-shot.
        exitMoveMode();
      };
      document.addEventListener('mousemove', move);
      document.addEventListener('mouseup', up);
    });
  }

  wireOverlayDrag();

  $('chatUnread').addEventListener('click', clearUnread);
  $('chatUnread').addEventListener('mouseenter', () => setBadgeHover(true));
  $('chatUnread').addEventListener('mouseleave', () => setBadgeHover(false));

  if (Settings.clean) {
    $('settingsBtn').style.display = 'none';
  }

  if (new URLSearchParams(location.search).get('open') === 'settings') {
    Settings.openDrawer();
  }

  Settings.onChange(() => render());
  window.addEventListener('tsn:lang', () => render());
  window.addEventListener('hashchange', () => location.reload());

  connectWS();
})();
