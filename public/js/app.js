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

  const whisperSeen = new Map(); // id -> last moment the user whispered

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

    // rows — whisperers go to their own fading section; after whispering
    // stops they linger there for `whisperDelay` seconds (v1.4.9).
    const list = $('userList');
    const whisperList = $('whisperList');
    const whisperBlock = $('whisperBlock');
    const nowTs = Date.now();
    const whisperMs = (Number(a.whisperDelay) || 4) * 1000;
    const whisperNow = (u) => {
      if (u.whispering) whisperSeen.set(u.id, nowTs);
      return u.whispering || nowTs - (whisperSeen.get(u.id) || 0) < whisperMs;
    };
    const whisperActive = a.showWhispers && users.some(whisperNow);
    whisperBlock.hidden = !whisperActive;
    const containerFor = (u) => (a.showWhispers && whisperNow(u) ? whisperList : list);
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
      pack.el.classList.toggle('whisper', a.showWhispers && whisperNow(u));
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
    renderEvents();

    // connection tab
    const pill = $('statusPill');
    pill.className = `status-pill ${state.status}`;
    pill.textContent = t(statusKey(state.status));
    $('statusDetail').textContent = statusHint();
  }

  /* ---------------- chat toasts ---------------- */

  const chatRows = new Map(); // id -> { el, timer }

  /* ---------------- events feed (recent server events) ---------------- */

  // Persistent «Последние события» block (v1.3.0): independent from the chat
  // toasts, filterable per event kind, expires after eventsDelay seconds.
  const evSeen = new Set();
  let evTickTimer = 0;

  function escHtml(s) {
    return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  }

  function nickSpan(n) {
    return `<span class="ev-nick">${escHtml(n)}</span>`;
  }

  function evText(m) {
    const clean = cleanChat;
    const name = nickSpan(clean(m.nickname || m.text || '…'));
    const detail = escHtml(clean(m.detail || ''));
    if (m.event === 'join') return detail ? t('evJoinChannel').replace('{n}', name).replace('{c}', detail) : t('evJoinServer').replace('{n}', name);
    if (m.event === 'leave') return detail ? t('evLeaveChannel').replace('{n}', name).replace('{c}', detail) : t('evLeaveServer').replace('{n}', name);
    if (m.event === 'move') return t('evMove').replace('{n}', name).replace('{c}', detail || '…');
    if (m.event === 'group') {
      return t(m.text === 'server' ? 'evGroupServer' : 'evGroupChannel')
        .replace('{n}', name)
        .replace('{g}', detail || '…');
    }
    return '';
  }

  function renderEvents() {
    const a = Settings.appearance;
    const feed = $('eventsFeed');
    if (!feed) return;
    const list = $('eventsList');
    const show = a.showEvents;
    feed.hidden = !show;
    if (!show || !state) {
      if (list) list.textContent = '';
      return;
    }
    const allow = (kind) =>
      (kind === 'join' && a.showEvJoin) ||
      (kind === 'leave' && a.showEvLeave) ||
      (kind === 'move' && a.showEvMove) ||
      (kind === 'poke' && a.showEvPoke) ||
      (kind === 'chat' && a.showEvChat) ||
      (kind === 'group' && a.showEvGroup) ||
      (kind === 'sys' && a.showEvSys);
    const delayMs = Math.max(5, Number(a.eventsDelay) || 20) * 1000;
    const maxRows = Math.min(10, Math.max(1, Number(a.eventsCount) || 5));
    const now = Date.now();

    // Collect: structured system events + server-chat system lines + chat.
    const items = [];
    for (const m of state.chat || []) {
      let kind = '';
      if (m.kind === 'system' && (m.event === 'join' || m.event === 'leave' || m.event === 'move' || m.event === 'group')) kind = m.event;
      else if (m.kind === 'poke') kind = 'poke';
      else if (m.kind === 'server') kind = 'sys'; // localized system lines from the client (TS6 server chat)
      else if (m.kind !== 'system') kind = 'chat';
      if (!kind || !allow(kind)) continue;
      if (kind === 'chat' && m.kind === 'private' && !a.chatPrivate) continue;
      // Per-type scope: «Мой канал» vs «Весь сервер» (v1.3.2). Channel/private
      // chats and pokes count as own-channel; server lines and remote events
      // carry their own `own` flag from the connectors.
      const scopeKey = {
        join: 'evScopeJoin', leave: 'evScopeLeave', move: 'evScopeMove',
        group: 'evScopeGroup', sys: 'evScopeSys', chat: 'evScopeChat',
      }[kind];
      const ownFlag = m.own ?? (m.kind === 'channel' || m.kind === 'private' || m.kind === 'poke');
      if (scopeKey && a[scopeKey] === 'own' && !ownFlag) continue;
      if (now - (m.at || 0) > delayMs) continue;
      const text = kind === 'sys'
        ? escHtml(cleanChat(m.text))
        : kind === 'chat' || kind === 'poke'
          ? (() => {
              const raw = `${cleanChat(m.nickname || m.senderName || '')}${m.text ? ': ' + cleanChat(m.text) : ''}`.replace(/^[:\s]+/, '') || cleanChat(m.text);
              const nick = cleanChat(m.nickname || m.senderName || '');
              return nick && raw.startsWith(nick) ? nickSpan(nick) + escHtml(raw.slice(nick.length)) : escHtml(raw);
            })()
          : evText(m);
      if (!text) continue;
      items.push({ id: m.id || `${kind}-${m.at}`, kind, text, at: m.at || now });
    }
    items.sort((x, y) => y.at - x.at);
    const shown = items.slice(0, maxRows);

    // Keyed diff — existing rows are NEVER moved (moving a node restarts its
    // CSS animation, which made the block jitter on every state render).
    const want = new Set(shown.map((i) => i.id));
    for (const el of [...list.children]) {
      if (!want.has(el.dataset.evId)) el.remove();
    }
    const have = new Set([...list.children].map((el) => el.dataset.evId));
    for (const item of shown) {
      if (have.has(item.id)) continue;
      const el = document.createElement('div');
      el.className = `event-row ${item.kind === 'sys' ? 'ev-sys' : item.kind === 'group' ? 'ev-group' : 'ev-' + item.kind}`;
      el.dataset.evId = item.id;
      const tag = document.createElement('span');
      tag.className = 'ev-tag';
      tag.textContent = t(
        item.kind === 'chat' ? 'evTagChat'
          : item.kind === 'join' ? 'evTagJoin'
            : item.kind === 'leave' ? 'evTagLeave'
              : item.kind === 'move' ? 'evTagMove'
                : item.kind === 'group' ? 'evTagGroup'
                  : item.kind === 'sys' ? 'evTagSys'
                    : 'evTagPoke',
      );
      const txt = document.createElement('span');
      txt.className = 'ev-msg';
      txt.innerHTML = item.text;
      el.appendChild(tag);
      el.appendChild(txt);
      // Newest first, inserted by timestamp — existing rows are never moved.
      el.dataset.evAt = String(item.at);
      const ref = [...list.children].find((c) => Number(c.dataset.evAt || 0) < item.at);
      if (ref) list.insertBefore(el, ref);
      else list.appendChild(el);
    }
    // Expiry tick: re-render as rows age out (cheap, coalesced).
    if (!evTickTimer && list.children.length) {
      evTickTimer = setTimeout(() => {
        evTickTimer = 0;
        renderEvents();
      }, 2500);
    }
  }

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
    if (!Settings.appearance.showUnreadBadge) return;
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
    if (!Settings.appearance.showUnreadBadge) {
      el.hidden = true;
      return;
    }
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

  function syncSourceCards() {
    const sel = $('sourceSelect');
    if (!sel) return;
    document.querySelectorAll('#sourceCards .source-card').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.source === sel.value);
    });
  }

  function fillConnectionTab() {
    if (!config) return;
    $('sourceSelect').value = config.source;
    syncSourceCards();
    $('ts3Host').value = config.ts3.host;
    $('ts3Port').value = config.ts3.port;
    $('ts3ApiKey').value = config.ts3.apiKey;
    $('ts6Host').value = config.ts6.host;
    $('ts6Port').value = config.ts6.port;
    $('ts6ApiKey').value = config.ts6.apiKey;
    $('ts3Fields').classList.toggle('visible', config.source === 'ts3');
    $('ts6Fields').classList.toggle('visible', config.source === 'ts6');
    $('elevateToggle').checked = config.desktop?.elevate !== false;
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

    // v1.3.0: big source cards drive the hidden select
    document.querySelectorAll('#sourceCards .source-card').forEach((btn) => {
      btn.addEventListener('click', () => {
        const sel = $('sourceSelect');
        sel.value = btn.dataset.source;
        sel.dispatchEvent(new Event('change'));
      });
    });
    $('sourceSelect').addEventListener('change', syncSourceCards);
    syncSourceCards();

    $('reconnectBtn').addEventListener('click', () => {
      fetch('/api/reconnect', { method: 'POST' }).catch(() => {});
    });

    $('elevateToggle').addEventListener('change', () => {
      putConfig({ desktop: { elevate: $('elevateToggle').checked } });
    });

    $('overlayWindowToggle').addEventListener('change', () => {
      putConfig({ desktop: { overlayWindow: $('overlayWindowToggle').checked } });
    });
  }

  function wireMoreTab() {
    // Version line — the live value arrives in the ws hello (the static
    // markup fallback was frozen at an old release and never refreshed).
    const ver = (window.TSN_VERSION || document.querySelector('.about-title .ver')?.textContent?.trim() || '').replace(/^v/, '');
    $('updateVersion').textContent = ver ? `v${ver}` : '';

    $('checkUpdateBtn').addEventListener('click', () => {
      $('updateStatus').textContent = t('updChecking');
      fetch('/api/update/check', { method: 'POST' }).catch(() => {
        $('updateStatus').textContent = t('updError');
      });
    });

    $('copyObsUrlBtn').addEventListener('click', async () => {
      const url = `${location.origin}/?clean=1`;
      let ok = false;
      try {
        await navigator.clipboard.writeText(url);
        ok = true;
      } catch {
        try {
          const ta = document.createElement('textarea');
          ta.value = url;
          document.body.appendChild(ta);
          ta.select();
          ok = document.execCommand('copy');
          ta.remove();
        } catch {
          ok = false;
        }
      }
      const btn = $('copyObsUrlBtn');
      btn.textContent = ok ? '✓' : '!';
      setTimeout(() => {
        btn.textContent = t('copyObsUrlBtn');
      }, 1500);
    });

    $('moveOverlayBtn').addEventListener('click', () => {
      fetch('/api/overlay/move', { method: 'POST' }).catch(() => {});
      const btn = $('moveOverlayBtn');
      btn.textContent = '✓';
      setTimeout(() => {
        btn.textContent = t('moveOverlayBtn');
      }, 1500);
    });

    $('restartNowBtn').addEventListener('click', () => {
      fetch('/api/update/restart', { method: 'POST' }).catch(() => {});
    });

    // «Плагин TS3»: install the bridge DLL into the client's plugin folder.
    const refreshPluginStatus = async () => {
      const el = $('pluginStatus');
      if (!el) return;
      try {
        const st = await (await fetch('/api/plugin/status')).json();
        el.textContent = st.active ? t('pluginActive') : t('pluginOff');
      } catch {
        el.textContent = '';
      }
    };
    $('pluginInstallBtn').addEventListener('click', async () => {
      const el = $('pluginStatus');
      el.textContent = '…';
      try {
        const res = await fetch('/api/plugin/install', { method: 'POST' });
        const body = await res.json();
        if (body.ok && body.same) {
          el.textContent = t('pluginSame');
        } else if (body.ok) {
          el.textContent = t('pluginInstalled');
          setTimeout(refreshPluginStatus, 1500);
        } else if (body.error === 'locked') {
          el.textContent = t('pluginLocked');
        } else if (body.error === 'dll-missing') {
          el.textContent = t('pluginNoDll');
        } else if (body.error === 'windows-only') {
          el.textContent = t('pluginFail') + ': ' + body.error;
        } else {
          el.textContent = t('pluginFail') + ': ' + (body.error || '');
        }
      } catch {
        el.textContent = t('pluginFail');
      }
    });
    refreshPluginStatus();

    // «Диагностика»: build the TS3/TS6 capture report and let the user copy it.
    $('diagBtn').addEventListener('click', async () => {
      const out = $('diagOut');
      out.hidden = false;
      out.value = '…';
      try {
        const res = await fetch('/api/diagnostics');
        out.value = await res.text();
      } catch {
        out.value = t('updError');
      }
      out.scrollTop = out.scrollHeight;
    });
    $('diagCopyBtn').addEventListener('click', async () => {
      const out = $('diagOut');
      if (!out.value) {
        out.hidden = false;
        out.value = '…';
        try {
          out.value = await (await fetch('/api/diagnostics')).text();
        } catch {
          out.value = t('updError');
        }
      }
      let ok = false;
      try {
        await navigator.clipboard.writeText(out.value);
        ok = true;
      } catch {
        out.hidden = false;
        out.select?.();
        ok = false;
      }
      const btn = $('diagCopyBtn');
      btn.textContent = ok ? '✓' : '!';
      setTimeout(() => {
        btn.textContent = t('diagCopyBtn');
      }, 1500);
    });

    // Save/cancel of the dragged position live ON the overlay itself — the
    // settings window may be behind the game or on another display.
    $('moveSaveBtn').addEventListener('click', (e) => {
      e.stopPropagation();
      fetch('/api/overlay/move-confirm', { method: 'POST' }).catch(() => {});
    });
    $('moveDiscardBtn').addEventListener('click', (e) => {
      e.stopPropagation();
      fetch('/api/overlay/move-cancel', { method: 'POST' }).catch(() => {});
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

  let lastConnSig = '';

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
        if (msg.moveMode) enterMoveMode();
        if (msg.state) {
          state = msg.state;
          syncFeatures();
          scheduleRender();
        }
        // Live version from the running binary — the static markup fallback
        // was frozen at an old release and never refreshed after updates.
        if (msg.version) {
          window.TSN_VERSION = String(msg.version).replace(/^v/, '');
          const span = document.querySelector('.about-title .ver');
          if (span) span.textContent = `v${window.TSN_VERSION}`;
          const upd = $('updateVersion');
          if (upd) upd.textContent = `v${window.TSN_VERSION}`;
        }
      } else if (msg.type === 'state') {
        state = msg.state;
        syncFeatures();
        scheduleRender();
      } else if (msg.type === 'config') {
        config = msg.config;
        Settings.syncServer(config);
        // Heavy tab refills only when non-appearance config changed — plain
        // look tweaks must not re-render the whole settings surface.
        const sig = JSON.stringify({ ...config, appearance: undefined });
        if (sig !== lastConnSig) {
          lastConnSig = sig;
          fillConnectionTab();
          syncFeatures();
        }
        // Appearance flags (events feed, unread badge, elements) take effect
        // immediately — one rAF-coalesced re-render.
        scheduleRender();
        // A size/appearance change coming from another window: re-measure the
        // panel so the cropped desktop window follows it.
        if (isCleanMode) {
          lastBoundsSig = '';
          scheduleBounds();
        }
      } else if (msg.type === 'move-mode') {
        const want = typeof msg.active === 'boolean' ? msg.active : !moveMode;
        if (want) enterMoveMode();
        else exitMoveMode();
      } else if (msg.type === 'move-pending') {
        // A drag finished somewhere — wait for the ✓/✗ buttons.
        const hint = $('moveHint');
        if (hint) hint.textContent = t('moveHintPending');
      } else if (msg.type === 'move-confirm') {
        applyPendingPosition(true);
      } else if (msg.type === 'move-cancel') {
        applyPendingPosition(false);
      } else if (msg.type === 'update-status') {
        const el = $('updateStatus');
        if (el) {
          const key = { checking: 'updChecking', available: 'updAvailable', latest: 'updLatest', downloaded: 'updDownloaded' }[msg.status] || 'updError';
          el.textContent = t(key).replace('{v}', msg.version || '');
        }
        const restart = $('restartNowBtn');
        if (restart) restart.hidden = msg.status !== 'downloaded';
      }
    };

    ws.onclose = () => setTimeout(connectWS, 1600);
    ws.onerror = () => ws.close();
  }

  /* ---------------- boot ---------------- */

  injectIcons();
  Settings.init();
  wireConnectionTab();
  wireMoreTab();
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

  /* ---------------- overlay window crop (v1.4.5) ----------------
   * The desktop shell shrinks the overlay window to the panel so a fullscreen
   * transparent layer is never composited. The page reports the panel rect in
   * screen-work-area coordinates; the shell answers with the crop applied. */
  const isCleanMode = Settings.clean;
  // Desktop window token (?w=...). OBS browser sources load the same page
  // without it and must never be cropped.
  const winToken = new URLSearchParams(location.search).get('w') || '';
  let cropX = 0;
  let cropY = 0;
  let lastBoundsSig = '';
  let boundsTimer = 0;

  function applyCropVars() {
    const root = document.documentElement.style;
    root.setProperty('--crop-x', String(cropX));
    root.setProperty('--crop-y', String(cropY));
  }

  function reportBounds() {
    if (!isCleanMode || !winToken || moveMode || typeof fetch !== 'function') return;
    const panel = $('panel');
    if (!panel || !panel.offsetWidth) return; // hidden → keep the last window
    const r = panel.getBoundingClientRect();
    const pad = 48; // room for the drop shadow + move-handle glow
    const rect = {
      x: Math.round(r.x + cropX - pad),
      y: Math.round(r.y + cropY - pad),
      w: Math.round(r.width + pad * 2),
      h: Math.round(r.height + pad * 2),
      token: winToken,
    };
    const sig = `${rect.x},${rect.y},${rect.w},${rect.h}`;
    if (sig === lastBoundsSig) return;
    lastBoundsSig = sig;
    fetch('/api/overlay/bounds', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(rect),
    })
      .then((resp) => resp.json())
      .then((b) => {
        if (b && b.ok) {
          cropX = b.cropX ?? 0;
          cropY = b.cropY ?? 0;
          if (b.screenW) document.documentElement.style.setProperty('--screen-w', `${b.screenW}px`);
          if (b.screenH) document.documentElement.style.setProperty('--screen-h', `${b.screenH}px`);
          applyCropVars();
          // The window is cropped to the panel, so the viewport is NOT the
          // placement surface anymore — hand the real work area to the
          // settings model or every apply() would clamp the panel back into
          // its corner.
          Settings.setSurface(b.screenW, b.screenH);
        }
      })
      .catch(() => {});
  }

  function scheduleBounds() {
    if (boundsTimer) return;
    boundsTimer = setTimeout(() => {
      boundsTimer = 0;
      reportBounds();
    }, 250);
  }

  // Rows appear/expire on their own timers — watch the panel instead of
  // chasing every render path.
  if (isCleanMode) {
    new MutationObserver(scheduleBounds).observe($('panel'), {
      childList: true,
      subtree: true,
      attributes: true,
      characterData: true,
    });
    // Size changes (panel width / max height, fonts, scale…) are layout or
    // CSS-var changes the observer above never sees — without a fresh report
    // the cropped desktop window keeps its old size and clips the panel.
    if (typeof ResizeObserver === 'function') {
      new ResizeObserver(scheduleBounds).observe($('panel'));
    }
    Settings.onChange(() => {
      lastBoundsSig = '';
      scheduleBounds();
    });
    window.addEventListener('resize', () => {
      lastBoundsSig = '';
      scheduleBounds();
    });
  }

  let moveMode = false;
  let moveModeTimer = 0;
  let pendingX = null;
  let pendingY = null;

  function exitMoveMode() {
    if (!moveMode) return;
    moveMode = false;
    if (moveModeTimer) clearTimeout(moveModeTimer);
    moveModeTimer = 0;
    pendingX = null;
    pendingY = null;
    document.body.classList.remove('move-mode');
    const handle = $('moveHandle');
    if (handle) handle.hidden = true;
    const hint = $('moveHint');
    if (hint) hint.hidden = true;
    postClickthrough(clickThroughWanted());
    if (isCleanMode) {
      lastBoundsSig = '';
      scheduleBounds();
    }
  }

  function enterMoveMode() {
    moveMode = true;
    document.body.classList.add('move-mode');
    if (isCleanMode) {
      cropX = 0;
      cropY = 0;
      applyCropVars();
    }
    const handle = $('moveHandle');
    if (handle) handle.hidden = false;
    const hint = $('moveHint');
    if (hint) {
      hint.hidden = false;
      hint.textContent = t('moveHint');
    }
    // Localized tooltips for the ✓/✗ overlay buttons (data-i18n-title is not
    // processed by the i18n runtime — set titles via JS).
    const saveBtn = $('moveSaveBtn');
    if (saveBtn) saveBtn.title = t('moveSaveTip');
    const discardBtn = $('moveDiscardBtn');
    if (discardBtn) discardBtn.title = t('moveCancelTip');
    // The overlay is click-through for gameplay — take the mouse only while
    // move mode is armed so the handle (and buttons) can be used.
    postClickthrough(false);
    if (moveModeTimer) clearTimeout(moveModeTimer);
    // Unconfirmed moves die on timeout — nothing is saved without consent.
    moveModeTimer = setTimeout(() => {
      fetch('/api/overlay/move-cancel', { method: 'POST' }).catch(() => {});
      applyPendingPosition(false);
    }, 120000);
  }

  /** Confirm (save) or cancel (restore) the dragged position. */
  function applyPendingPosition(save) {
    const moved = pendingX !== null || pendingY !== null;
    if (save && moved) {
      // Save through the settings model: the value lands in the local cache,
      // in CSS and on the server in one step, so a later config echo cannot
      // roll the panel back to the corner it was dragged away from.
      Settings.setMany({ marginX: pendingX, marginY: pendingY });
    } else if (moved) {
      // Snap back to the last saved position.
      Settings.reapply();
    }
    exitMoveMode();
  }

  function wireOverlayDrag() {
    const handle = $('moveHandle');
    const header = $('panelHeader');
    if (!handle && !header) return;
    let dragging = false;
    let startX = 0;
    let startY = 0;
    let baseX = 0;
    let baseY = 0;
    let curX = 0;
    let curY = 0;

    const startDrag = (e) => {
      // Dragging is ONLY possible in move mode so the game never loses the
      // mouse by accident. The dedicated «⠿» handle works even when the
      // header (channel name) is hidden by the user.
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
        const bottomSide = corner === 'bl' || corner === 'br';
        // The panel bumps into the screen edges and never leaves them — the
        // bounds match apply()'s clamp exactly so a dropped panel STAYS put.
        // NOTE: the surface, not the viewport — the desktop window is cropped
        // to the panel, so innerWidth/innerHeight are far too small here.
        const surf = Settings.surfaceSize();
        const maxX = Math.max(0, surf.w - (panel?.offsetWidth || 0));
        const maxY = Math.max(0, surf.h - (panel?.offsetHeight || 0));
        curX = Math.min(maxX, Math.max(0, baseX + (rightSide ? -dx : dx)));
        // Bottom-anchored corners measure the margin from the bottom edge —
        // dragging down must shrink it (the panel follows the mouse).
        curY = Math.min(maxY, Math.max(0, baseY + (bottomSide ? -dy : dy)));
        document.documentElement.style.setProperty('--margin-x', `${curX}px`);
        document.documentElement.style.setProperty('--margin-y', `${curY}px`);
      };
      const up = () => {
        dragging = false;
        document.body.classList.remove('dragging');
        document.removeEventListener('mousemove', move);
        document.removeEventListener('mouseup', up);
        // Nothing is saved yet: the user confirms with the «Подтвердить»
        // button («Отмена» snaps the panel back to the saved spot).
        pendingX = curX;
        pendingY = curY;
        const hint = $('moveHint');
        if (hint) hint.textContent = t('moveHintPending');
        fetch('/api/overlay/moved', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ marginX: curX, marginY: curY }),
        }).catch(() => {});
      };
      document.addEventListener('mousemove', move);
      document.addEventListener('mouseup', up);
    };

    if (handle) handle.addEventListener('mousedown', startDrag);
    if (header) header.addEventListener('mousedown', startDrag);
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
