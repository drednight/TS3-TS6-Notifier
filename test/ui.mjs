/**
 * UI smoke test: loads the real overlay page (served by the real server,
 * started in-process), simulates the WebSocket state stream, and asserts that
 * rows, header, labels and the settings drawer all actually render.
 *
 * Run: npm run test:ui
 */
import { JSDOM, VirtualConsole } from 'jsdom';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';
import { startServer } from '../server/app.js';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');

const results = [];
function check(name, cond, extra = '') {
  results.push(!!cond);
  console.log(`${cond ? 'PASS' : 'FAIL'}  ${name}${extra ? ' — ' + extra : ''}`);
}

const fakeState = {
  source: 'mock',
  status: 'live',
  detail: '',
  serverName: 'UI Test Server',
  channelName: 'UI Test Channel',
  channelId: '1',
  selfId: 'u1',
  updatedAt: Date.now(),
  users: [
    { id: 'u1', nickname: 'Tester One', channelId: 1, talking: true, inputMuted: false, outputMuted: false, away: false, awayMessage: '', prioritySpeaker: false, channelCommander: true, isSelf: true, isQuery: false, avatarHash: '', serverGroups: '' },
    { id: 'u2', nickname: 'Мария', channelId: 1, talking: false, inputMuted: true, outputMuted: true, away: true, awayMessage: 'чай', prioritySpeaker: true, channelCommander: false, isSelf: false, isQuery: false, avatarHash: 'abc', serverGroups: '' },
    { id: 'u3', nickname: 'QueryBot', channelId: 1, talking: false, inputMuted: false, outputMuted: false, away: false, awayMessage: '', prioritySpeaker: false, channelCommander: false, isSelf: false, isQuery: true, avatarHash: '', serverGroups: '' },
  ],
};

const fakeConfig = {
  source: 'mock',
  ts3: { host: '127.0.0.1', port: 25639, apiKey: '' },
  ts6: { host: '127.0.0.1', port: 5899, apiKey: '' },
  desktop: { clickThrough: true, autostart: false, visible: true, seen: true },
  appearance: {
    theme: 'glass', accent: '#7c6cff', glow: 0.7, opacity: 0.72, blur: 18, radius: 16,
    scale: 1, fontSize: 13, corner: 'tr', margin: 18, language: 'ru',
    showHeader: true, showServerName: true, showUserCount: true, showBanner: true,
    showAvatars: true, showBadges: true, showSelf: true, hideQuery: true,
    animations: true, eqStyle: 'bars', talkingFirst: true, onlyTalking: false,
    compact: false, hideEmpty: false,
  },
};

class FakeWS {
  constructor(url) {
    FakeWS.last = this;
    this.readyState = 0;
    setTimeout(() => {
      this.readyState = 1;
      this.onopen?.();
      this._emit({ type: 'hello', config: fakeConfig, state: fakeState });
    }, 30);
  }
  _emit(obj) {
    this.onmessage?.({ data: JSON.stringify(obj) });
  }
  send() {}
  close() {
    this.onclose?.();
  }
}

function bootPage(url) {
  const errors = [];
  const virtualConsole = new VirtualConsole();
  virtualConsole.on('jsdomError', (e) => errors.push('jsdomError: ' + e.message));
  virtualConsole.on('error', (...args) => errors.push('console.error: ' + args.join(' ')));

  return new Promise((resolve) => {
    JSDOM.fromURL(url, {
      runScripts: 'dangerously',
      resources: 'usable',
      pretendToBeVisual: true,
      virtualConsole,
      beforeParse(window) {
        window.WebSocket = FakeWS;
        window.fetch = (input, init) => {
          const u = String(input);
          if (u.includes('/api/config') && (!init || init.method === 'GET' || !init.method)) {
            return Promise.resolve(new Response(JSON.stringify(fakeConfig)));
          }
          if (u.includes('/api/config')) {
            return Promise.resolve(new Response(JSON.stringify({ ok: true, config: fakeConfig })));
          }
          return Promise.resolve(new Response(JSON.stringify(fakeState)));
        };
        window.Response = class {
          constructor(body) {
            this.body = body;
          }
          json() {
            return Promise.resolve(JSON.parse(this.body));
          }
        };
      },
    }).then((dom) => {
      setTimeout(() => resolve({ dom, errors }), 700);
    });
  });
}

/* ---------------- start a dedicated in-process server ---------------- */

const handle = await startServer({
  port: 0,
  host: '127.0.0.1',
  dataDir: join(tmpdir(), 'tsn-ui-test'),
  publicDir: join(ROOT, 'public'),
  log: () => {},
});
const BASE = `http://127.0.0.1:${handle.port()}`;

try {
  /* ---- 1. overlay page (clean mode, like the in-game overlay window) ---- */
  {
    const { dom, errors } = await bootPage(`${BASE}/?clean=1`);
    const doc = dom.window.document;

    check('overlay: no JS errors', errors.length === 0, errors.slice(0, 3).join(' | '));
    check('overlay: channel name rendered', doc.getElementById('channelNameText')?.textContent === 'UI Test Channel', doc.getElementById('channelNameText')?.textContent);
    check('overlay: server name rendered', doc.getElementById('serverName')?.textContent === 'UI Test Server');
    check('overlay: user rows rendered', doc.querySelectorAll('#userList .row').length === 2, `rows=${doc.querySelectorAll('#userList .row').length}`);
    check('overlay: query bot hidden', ![...doc.querySelectorAll('.nick')].some((n) => n.textContent === 'QueryBot'));
    check('overlay: talking row highlighted', !!doc.querySelector('#userList .row.talking'));
    check('overlay: self row marked', !!doc.querySelector('#userList .row.self'));
    check('overlay: muted badge shown', !!doc.querySelector('#userList .badge.mic-off'));
    check('overlay: away shown', !!doc.querySelector('#userList .row.away'));
    check('overlay: settings button hidden in clean mode', doc.getElementById('settingsBtn').style.display === 'none');
    check('overlay: drawer hidden initially', doc.getElementById('settingsDrawer').hidden === true);
    check('overlay: whisper/chat containers present', !!doc.getElementById('whisperBlock') && !!doc.getElementById('chatFeed'));
    check('overlay: class glyph API available', !!dom.window.TSIcons && typeof dom.window.TSIcons.classGlyph === 'function' && dom.window.TSIcons.CLASSES.length === 18);

    // v1.1.1: chat must be visible even in clean mode (the overlay window)
    FakeWS.last._emit({
      type: 'state',
      state: {
        ...fakeState,
        chat: [
          { id: 'c1', mode: 2, userId: 'u2', nickname: 'Мария', text: 'привет', kind: 'channel', at: Date.now() },
          { id: 'c2', mode: 2, userId: 'u1', nickname: '', text: 'своё сообщение', kind: 'channel', at: Date.now() },
        ],
      },
    });
    await new Promise((r) => setTimeout(r, 60));
    check('overlay: chat shows in clean mode', doc.getElementById('chatFeed')?.children.length > 0, `msgs=${doc.getElementById('chatFeed')?.children.length}`);
    const nick1 = doc.querySelector('#chatFeed .chat-toast .chat-nick')?.textContent || '';
    check('overlay: chat shows the sender nick', nick1.includes('Мария'), nick1);
    const nicks = [...doc.querySelectorAll('#chatFeed .chat-nick')].map((n) => n.textContent);
    check('overlay: empty nick resolved from the user list', nicks.some((n) => n.includes('Tester One')), JSON.stringify(nicks));
    // server messages have no sender — must be labeled, not left blank
    FakeWS.last._emit({
      type: 'state',
      state: { ...fakeState, chat: [{ id: 'c3', userId: '0', nickname: '', text: 'добро пожаловать', kind: 'server', at: Date.now() }] },
    });
    await new Promise((r) => setTimeout(r, 60));
    const serverNicks = [...doc.querySelectorAll('#chatFeed .chat-nick')].map((n) => n.textContent);
    check('overlay: server message labeled', serverNicks.some((n) => /сервер|server/i.test(n)), JSON.stringify(serverNicks));

    // v1.1.10: the unread badge is explicit — it clears only on "read" (click)
    const badge = doc.getElementById('chatUnread');
    badge.hidden = false;
    badge.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
    check('overlay: unread badge clears on click (mark as read)', badge.hidden === true);

    FakeWS.last._emit({
      type: 'state',
      state: {
        ...fakeState,
        chat: [
          { id: 'c90', userId: 'u2', nickname: 'Мария', text: 'новое', kind: 'channel', at: Date.now() },
          { id: 'c91', userId: 'u1', nickname: '', text: 'моё', kind: 'channel', at: Date.now() },
        ],
      },
    });
    await new Promise((r) => setTimeout(r, 60));
    check('overlay: unread counts foreign messages only', badge.hidden === false && doc.getElementById('chatUnreadCount').textContent === '1', doc.getElementById('chatUnreadCount').textContent);

    // hovering the badge must take mouse input so the click can land
    // (the desktop overlay is click-through by default)
    const hoverCalls = [];
    dom.window.fetch = (input, init) => {
      hoverCalls.push({ u: String(input), b: init?.body ? JSON.parse(init.body) : null });
      return Promise.resolve(new dom.window.Response('{}'));
    };
    badge.dispatchEvent(new dom.window.MouseEvent('mouseenter'));
    badge.dispatchEvent(new dom.window.MouseEvent('mouseleave'));
    check('overlay: badge hover toggles overlay click-through', (() => {
      return hoverCalls.length === 2 &&
        hoverCalls[0].u.includes('/api/overlay/clickthrough') &&
        hoverCalls[0].b?.ignore === false &&
        hoverCalls[1].b?.ignore === true;
    })(), JSON.stringify(hoverCalls));
    badge.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
    check('overlay: badge stays cleared after read', badge.hidden === true);

    // v1.1.11: system event feed (join/leave/move) — localised row, no '…:'
    FakeWS.last._emit({
      type: 'state',
      state: {
        ...fakeState,
        chat: [
          { id: 'c92', userId: 'u2', nickname: 'Мария', text: 'Мария', kind: 'system', event: 'join', detail: 'UI Test Channel', at: Date.now() },
          { id: 'c93', userId: 'u2', nickname: 'Мария', text: 'привет', kind: 'channel', at: Date.now() },
        ],
      },
    });
    await new Promise((r) => setTimeout(r, 60));
    const sysRow = [...doc.querySelectorAll('#chatFeed .chat-toast.kind-system')].map((el) => el.textContent).join(' | ');
    check('overlay: system event row localised (join)', /Мария/.test(sysRow) && /UI Test Channel/.test(sysRow) && !/…:/.test(sysRow), sysRow);
    check('overlay: system rows are not counted as unread', badge.hidden === false && doc.getElementById('chatUnreadCount').textContent === '1', doc.getElementById('chatUnreadCount').textContent);
    // v1.3.0: persistent events feed (independent of chat toasts)
    const evFeed = doc.getElementById('eventsFeed');
    const evRows = doc.querySelectorAll('#eventsList .event-row');
    check('overlay: events feed shows join/system events', evFeed.hidden === false && evRows.length >= 1, `rows=${evRows.length}`);
    check('overlay: events feed row text localised', /Мария/.test(doc.getElementById('eventsList').textContent), doc.getElementById('eventsList').textContent);
    FakeWS.last._emit({
      type: 'config',
      config: { ...fakeConfig, appearance: { ...fakeConfig.appearance, showEvents: false } },
    });
    await new Promise((r) => setTimeout(r, 40));
    check('overlay: events feed disableable', evFeed.hidden === true);
    FakeWS.last._emit({
      type: 'config',
      config: { ...fakeConfig, appearance: { ...fakeConfig.appearance, showUnreadBadge: false } },
    });
    await new Promise((r) => setTimeout(r, 40));
    check('overlay: unread badge hideable via setting', badge.hidden === true);

    // v1.3.1: server-chat system lines + group changes surface as events
    FakeWS.last._emit({
      type: 'config',
      config: { ...fakeConfig, appearance: { ...fakeConfig.appearance, showEvents: true, showUnreadBadge: true } },
    });
    FakeWS.last._emit({
      type: 'state',
      state: {
        ...fakeState,
        chat: [
          { id: 'sys90', userId: '', nickname: '', text: 'Σκuλλα подключился(-ась) к Fakken', kind: 'server', at: Date.now() },
          { id: 'grp91', userId: 'u2', nickname: 'джусинг', text: 'channel', kind: 'system', event: 'group', detail: 'Guest', at: Date.now() },
        ],
      },
    });
    await new Promise((r) => setTimeout(r, 60));
    const evHtml = doc.getElementById('eventsList').innerHTML;
    check('overlay: server-chat system line shown as event', doc.querySelector('#eventsList .ev-sys') && /Fakken/.test(evHtml));
    check('overlay: group change shown as event', doc.querySelector('#eventsList .ev-group') && /Guest/.test(evHtml));
    check('overlay: events plaque removed (no header)', !doc.querySelector('.events-head'));

    // v1.1.12: dragging is gated behind the "move overlay" hotkey
    const header = doc.getElementById('panelHeader');
    const mxBefore = doc.documentElement.style.getPropertyValue('--margin-x');
    header.dispatchEvent(new dom.window.MouseEvent('mousedown', { clientX: 200, clientY: 100, bubbles: true }));
    doc.dispatchEvent(new dom.window.MouseEvent('mousemove', { clientX: 160, clientY: 120 }));
    doc.dispatchEvent(new dom.window.MouseEvent('mouseup', { clientX: 160, clientY: 120 }));
    check('overlay: drag inert without move mode', doc.documentElement.style.getPropertyValue('--margin-x') === mxBefore, `${mxBefore} → ${doc.documentElement.style.getPropertyValue('--margin-x')}`);

    // v1.1.15: dedicated drag handle + explicit confirm to save
    const moveHandle = doc.getElementById('moveHandle');
    check('overlay: drag handle present', !!moveHandle && moveHandle.hidden === true);

    FakeWS.last._emit({ type: 'move-mode', active: true });
    await new Promise((r) => setTimeout(r, 30));
    check('overlay: move mode armed by hotkey message', doc.body.classList.contains('move-mode'));
    check('overlay: handle visible in move mode', moveHandle.hidden === false);
    header.dispatchEvent(new dom.window.MouseEvent('mousedown', { clientX: 200, clientY: 100, bubbles: true }));
    doc.dispatchEvent(new dom.window.MouseEvent('mousemove', { clientX: 160, clientY: 120 }));
    doc.dispatchEvent(new dom.window.MouseEvent('mouseup', { clientX: 160, clientY: 120 }));
    const mx = doc.documentElement.style.getPropertyValue('--margin-x');
    const my = doc.documentElement.style.getPropertyValue('--margin-y');
    // corner=tr: marginX from the right edge (drag left → grows), marginY from
    // the top (drag down → grows). No 500px cap — only screen bounds.
    check('overlay: drag in move mode updates offsets', mx === '58px' && my === '38px', `x=${mx} y=${my}`);
    // v1.1.5→1.2.0: a drag alone does NOT end move mode — save/cancel are the
    // ✓/✗ icon buttons on the overlay handle itself
    check('overlay: move mode stays after the drag (await confirm)', doc.body.classList.contains('move-mode') === true);
    check('overlay: ✓/✗ save-cancel buttons on the handle', !!doc.getElementById('moveSaveBtn') && !!doc.getElementById('moveDiscardBtn') && doc.getElementById('moveHandle').hidden === false);

    // Cancel snaps the panel back to the saved position
    FakeWS.last._emit({ type: 'move-cancel' });
    await new Promise((r) => setTimeout(r, 30));
    check('overlay: move-cancel restores position + exits', doc.body.classList.contains('move-mode') === false && doc.documentElement.style.getPropertyValue('--margin-x') === `${Number(dom.window.Settings.appearance.marginX) || 0}px`);

    // Drag via the dedicated handle works too; confirm saves
    FakeWS.last._emit({ type: 'move-mode', active: true });
    await new Promise((r) => setTimeout(r, 30));
    moveHandle.dispatchEvent(new dom.window.MouseEvent('mousedown', { clientX: 200, clientY: 100, bubbles: true }));
    doc.dispatchEvent(new dom.window.MouseEvent('mousemove', { clientX: 140, clientY: 130 }));
    doc.dispatchEvent(new dom.window.MouseEvent('mouseup', { clientX: 140, clientY: 130 }));
    const hx = doc.documentElement.style.getPropertyValue('--margin-x');
    check('overlay: handle drag updates offsets', hx === '78px', `x=${hx}`);
    FakeWS.last._emit({ type: 'move-confirm' });
    await new Promise((r) => setTimeout(r, 30));
    check('overlay: move-confirm saves + exits', doc.body.classList.contains('move-mode') === false && Number(dom.window.Settings.appearance.marginX) === 78, String(dom.window.Settings.appearance.marginX));

    // v1.1.12: the panel is top-anchored so it fills downward
    let fillsDown = false;
    try {
      for (const sheet of doc.styleSheets) {
        for (const rule of sheet.cssRules || []) {
          if (rule.selectorText === 'body[data-corner="tr"] #panel' && rule.style.top) fillsDown = true;
        }
      }
    } catch {
      fillsDown = true;
    }
    check('overlay: panel is top-anchored (fills downward)', fillsDown);

    // v1.1.1: class group icon selection — match the CLASS group by name
    const TSN = dom.window.TSN;
    TSN.state = { ...fakeState, groups: [{ id: 4, name: 'Служители света', iconId: 10 }, { id: 5, name: 'Лучники Ордена', iconId: 777 }] };
    check('class icon: picks the bundled class icon by group name', TSN.groupIconUrl({ serverGroups: '4,5' }) === '/img/classes/luk.webp', TSN.groupIconUrl({ serverGroups: '4,5' }));
    check('class icon: classKeyFor matches by name', TSN.classKeyFor({ serverGroups: '4,5' }) === 'archer', String(TSN.classKeyFor({ serverGroups: '4,5' })));
    check('class icon: falls back to server icon without class name', TSN.groupIconUrl({ serverGroups: '4' }) === '/api/tsicon/10');
    check('class icon: classIconFor exposes the webp mapping', TSN.classIconFor({ serverGroups: '5' }) === '/img/classes/luk.webp');
    TSN.state = { ...fakeState, groups: [{ id: 7, name: 'Канглонг', iconId: 0 }] };
    check('class icon: Канглонг mapped', TSN.classIconFor({ serverGroups: '7' }) === '/img/classes/canglong.webp', String(TSN.classIconFor({ serverGroups: '7' })));
    TSN.state = { ...fakeState, groups: [{ id: 8, name: 'Духи Крови', iconId: 0 }] };
    check('class icon: Дух Крови matched loosely', TSN.classIconFor({ serverGroups: '8' }) === '/img/classes/dk.webp', String(TSN.classIconFor({ serverGroups: '8' })));
    const savedCfg = TSN.config;
    TSN.config = { ...fakeConfig, classes: { enabled: true, map: { '442': 'archer', '444': 'priest', '444': 'priest' } } };
    TSN.state = { ...fakeState, groups: [] };
    check('class icon: id map works without server-group list', TSN.classIconFor({ serverGroups: '442,999' }) === '/img/classes/luk.webp', String(TSN.classIconFor({ serverGroups: '442,999' })));
    check('class icon: second mapped id works without groups', TSN.classKeyFor({ serverGroups: '444' }) === 'priest', String(TSN.classKeyFor({ serverGroups: '444' })));
    TSN.config = savedCfg;
    dom.window.close();
  }

  /* ---- 2. settings window (?open=settings) ---- */
  {
    const { dom, errors } = await bootPage(`${BASE}/?open=settings`);
    const doc = dom.window.document;

    check('settings: no JS errors', errors.length === 0, errors.slice(0, 3).join(' | '));
    check('settings: drawer opened', doc.getElementById('settingsDrawer').hidden === false);
    check('settings: backdrop shown', doc.getElementById('drawerBackdrop').hidden === false);
    const label = doc.querySelector('[data-i18n="tabAppearance"]')?.textContent;
    check('settings: i18n labels filled', !!label && label.length > 1, JSON.stringify(label));
    check('settings: sliders present', doc.querySelectorAll('[data-slider]').length >= 7, `count=${doc.querySelectorAll('[data-slider]').length}`);
    check('settings: toggles present', doc.querySelectorAll('[data-toggle]').length >= 12);
    check('settings: slider values painted', !!doc.querySelector('[data-slider="opacity"] output')?.textContent.includes('%'), doc.querySelector('[data-slider="opacity"] output')?.textContent);
    check('settings: connection status pill', !!doc.getElementById('statusPill').textContent, doc.getElementById('statusPill').textContent);

    // interaction: click a theme button and a toggle, expect CSS var change
    const before = doc.documentElement.style.getPropertyValue('--accent');
    doc.querySelector('#accentSwatches .swatch[data-value="#22d3ee"]').click();
    const after = doc.documentElement.style.getPropertyValue('--accent');
    check('settings: accent change applies live', after === '#22d3ee', `${before} → ${after}`);
    doc.querySelector('[data-toggle="compact"]').click();
    check('settings: compact toggle applies class', dom.window.document.body.classList.contains('compact'));

    // v1.0.2 feature round
    check('settings: margin X/Y sliders', !!doc.querySelector('[data-slider="marginX"]') && !!doc.querySelector('[data-slider="marginY"]'));
    check('settings: margin sliders bounded by screen minus panel', (() => {
      const ix = doc.querySelector('[data-slider="marginX"] input[type="range"]');
      const iy = doc.querySelector('[data-slider="marginY"] input[type="range"]');
      const sw = Math.round(dom.window.screen.availWidth || 1920);
      return Number(ix?.max) >= 500 && Number(ix.max) <= sw && Number(iy?.max) >= 500;
    })(), `x=${doc.querySelector('[data-slider="marginX"] input')?.max} y=${doc.querySelector('[data-slider="marginY"] input')?.max}`);
    // v1.1.14: the move hotkey was replaced by a button on the Appearance tab
    check('settings: move-overlay button under the position controls', !!doc.getElementById('moveOverlayBtn') && !doc.getElementById('moveHotkey'));
    // v1.2.0: confirm/cancel buttons removed from settings — ✓/✗ live on the overlay
    check('settings: no confirm/cancel buttons (moved to overlay)', !doc.getElementById('moveConfirmBtn') && !doc.getElementById('moveConfirmRow'));
    check('settings: restart button ready for downloaded update', !!doc.getElementById('restartNowBtn') && doc.getElementById('restartNowBtn').hidden === true);
    // v1.1.15: «Ещё» is a single tab — About/profiles merged in
    check('settings: «Ещё» tab present', !!doc.querySelector('.tab[data-tab="more"]') && !!doc.querySelector('[data-panel="more"]'));
    check('settings: About merged into «Ещё»', !doc.querySelector('.tab[data-tab="about"]') && !!doc.querySelector('[data-panel="more"] #exportBtn') && !!doc.querySelector('[data-panel="more"] .about-card'));
    check('settings: version line shows 1.3.0', /1\.3\.0/.test(doc.querySelector('.about-card .ver')?.textContent || ''), doc.querySelector('.about-card .ver')?.textContent);
    // v1.3.0 — Concept 3 redesign: preview strip, chips, source cards, tiles
    check('settings: live preview strip present', !!doc.querySelector('.preview-strip .preview-panel'));
    check('settings: elements grouped into opt-cards', doc.querySelectorAll('[data-panel="elements"] .opt-group').length >= 4);
    check('settings: chip toggles wired', doc.querySelectorAll('[data-panel="elements"] .chip-toggle input[data-toggle]').length >= 16);
    check('settings: source cards drive the source', !!doc.getElementById('sourceCards') && doc.querySelectorAll('#sourceCards .source-card').length === 3 && doc.getElementById('sourceSelect').hidden === true);
    check('settings: tile buttons in «Ещё»', !!doc.getElementById('checkUpdateBtn').closest('.tile-btn') && !!doc.getElementById('copyObsUrlBtn').closest('.tile-btn'));
    // v1.3.0 — new notification/event settings
    check('settings: unread badge toggle present', !!doc.querySelector('[data-toggle="showUnreadBadge"]'));
    check('settings: events settings present', !!doc.querySelector('[data-toggle="showEvents"]') && !!doc.querySelector('[data-toggle="showEvJoin"]') && !!doc.querySelector('[data-toggle="showEvChat"]') && !!doc.querySelector('[data-slider="eventsCount"]'));
    check('settings: group/system event chips present', !!doc.querySelector('[data-toggle="showEvGroup"]') && !!doc.querySelector('[data-toggle="showEvSys"]'));
    // v1.3.1: event sliders show counts/seconds, not px
    check('settings: event slider units (no px)', (() => {
      const c = doc.querySelector('[data-slider="eventsCount"] output')?.textContent || '';
      const d = doc.querySelector('[data-slider="eventsDelay"] output')?.textContent || '';
      return !/px/.test(c) && /^\d+ s$/.test(d);
    })(), `count='${doc.querySelector('[data-slider="eventsCount"] output')?.textContent}' delay='${doc.querySelector('[data-slider="eventsDelay"] output')?.textContent}'`);
    check('settings: update check button in «Ещё»', !!doc.getElementById('checkUpdateBtn') && !!doc.getElementById('updateStatus') && !!doc.getElementById('updateVersion'));
    // v1.2.0: «Запустить вещание» removed — OBS Browser Source only needs the
    // copied URL (the launch button was not required to show TS in OBS)
    check('settings: no OBS launch button (URL is enough for OBS)', !doc.getElementById('obsLaunchBtn') && !!doc.getElementById('copyObsUrlBtn') && !!doc.getElementById('overlayWindowToggle'));
    check('settings: OBS browser-source hint shows the URL', /\/\?clean=1/.test(doc.getElementById('obsHint')?.textContent || ''), doc.getElementById('obsHint')?.textContent);

    // v1.1.14: placement is clamped so the panel never leaves the screen
    FakeWS.last._emit({
      type: 'config',
      config: { ...fakeConfig, appearance: { ...fakeConfig.appearance, marginX: 99999, marginY: 99999 } },
    });
    await new Promise((r) => setTimeout(r, 60));
    const clampedX = parseFloat(doc.documentElement.style.getPropertyValue('--margin-x'));
    const clampedY = parseFloat(doc.documentElement.style.getPropertyValue('--margin-y'));
    check('settings: margins clamp to the screen bounds', clampedX > 0 && clampedX < 99999 && clampedY > 0 && clampedY < 99999, `x=${clampedX} y=${clampedY}`);

    // v1.1.15: server config wins over a stale local cache (no more "stuck
    // until reconnect" panel states)
    try {
      dom.window.localStorage.setItem('tsnotifier:appearance', JSON.stringify({ marginX: 13, marginY: 13 }));
    } catch {
      /* ignore */
    }
    FakeWS.last._emit({
      type: 'config',
      config: { ...fakeConfig, appearance: { ...fakeConfig.appearance, marginX: 500, marginY: 42 } },
    });
    await new Promise((r) => setTimeout(r, 60));
    const followX = doc.documentElement.style.getPropertyValue('--margin-x');
    check('settings: server config applies instantly over stale cache', followX === '500px', followX);
    check('settings: chat delay slider', !!doc.querySelector('[data-slider="chatDelay"]'));
    check('settings: whisper/chat toggles', !!doc.querySelector('[data-toggle="showWhispers"]') && !!doc.querySelector('[data-toggle="showChat"]') && !!doc.querySelector('[data-toggle="chatPrivate"]'));
    check('settings: sound tab wired', !!doc.getElementById('soundItems') && !!doc.getElementById('audioOutput') && !!doc.getElementById('stopAllBtn') && !!doc.getElementById('clearHotkeysBtn'));
    // the whole sound feature is parked: tab button and panel are hidden
    const soundTabBtn = doc.querySelector('.tab[data-tab="sound"]');
    const soundPanel = doc.querySelector('[data-panel="sound"]');
    check('settings: sound tab hidden (feature parked)', soundTabBtn?.hidden === true && soundPanel?.hidden === true);
    check('settings: server groups in a collapsed block at the bottom', !!doc.getElementById('ts6GroupsBlock') && doc.getElementById('ts6GroupsBlock').open === false);
    // v1.1.9: chat polish + desktop toggles
    check('settings: header close X removed', !doc.getElementById('closeDrawer'));
    check('settings: run-as-admin toggle present', !!doc.getElementById('elevateToggle'));
    check('overlay: unread-message badge present', !!doc.getElementById('chatUnread') && !!doc.getElementById('chatUnreadCount'));
    check('settings: adding sounds lives in the board menu only', !doc.getElementById('soundUploadBtn') && !doc.getElementById('addPresetsBtn'));
    check('settings: sound guide card present', !!doc.querySelector('.guide-card') && !!doc.getElementById('loopbackStatus'));

    // v1.1.1: dual output + hotkey toggle + visible hotkeys + range fill
    check('settings: dual sound targets (мне/в канал)', !!doc.getElementById('localEnabled') && !!doc.getElementById('audioOutputLocal') && !!doc.getElementById('localVolume') && !!doc.getElementById('soundsEnabled'));
    check('settings: hotkeys enable toggle', !!doc.getElementById('hotkeysEnabled'));
    const rangeFill = doc.getElementById('localVolume')?.style.getPropertyValue('--fill');
    check('settings: range fill painted (not stuck at 50%)', !!rangeFill && rangeFill !== '50%', `fill=${rangeFill}`);
    // hotkey visible in the sound list
    dom.window.TSN.config = {
      ...fakeConfig,
      sounds: { enabled: true, volume: 0.8, outputDevice: '', hotkeysEnabled: true, localEnabled: true, localVolume: 0.8, localDevice: '', items: [{ id: 's1', name: 'Монета', file: '/presets/coin.wav', hotkey: 'Ctrl+Alt+1', volume: 1 }] },
    };
    doc.dispatchEvent(new dom.window.CustomEvent('tsn:config'));
    await new Promise((r) => setTimeout(r, 60));
    const cap = doc.querySelector('.sound-row .hotkey-cap');
    check('settings: sound row shows assigned hotkey', cap?.textContent.includes('Ctrl+Alt+1'), cap?.textContent);
    check('settings: hotkey can be cleared', !!doc.querySelector('.sound-row .hotkey-clear'));
    check('settings: sound item editor (group/trim)', !!doc.querySelector('.sound-item .sound-details') && !!doc.querySelector('.sound-item .d-group') && !!doc.querySelector('.sound-item .d-start') && !!doc.querySelector('.sound-item .d-end'));

    // TS6 group → class mapping UI
    check('settings: TS6 group map present', !!doc.getElementById('ts6GroupMap'));
    dom.window.TSN.state = { ...fakeState, groups: [{ id: 6, name: 'Лучники Ордена', iconId: 0 }, { id: 12, name: '', iconId: 0 }] };
    doc.dispatchEvent(new dom.window.CustomEvent('tsn:state'));
    await new Promise((r) => setTimeout(r, 40));
    const mapRows = doc.querySelectorAll('#ts6GroupMap .group-map-row');
    check('settings: TS6 group map lists server groups', mapRows.length === 2, `rows=${mapRows.length}`);
    dom.window.close();
  }

  /* ---- 3. bare settings window (desktop settings URL) ---- */
  {
    const { dom, errors } = await bootPage(`${BASE}/?open=settings&bare=1`);
    const doc = dom.window.document;
    check('bare: no JS errors', errors.length === 0, errors.slice(0, 3).join(' | '));
    check('bare: overlay stage hidden', doc.body.classList.contains('bare') && doc.getElementById('stage').style.display === 'none');
    check('bare: settings drawer open', doc.getElementById('settingsDrawer').hidden === false);
    dom.window.close();
  }

  /* ---- 4. soundboard panel page ---- */
  {
    const { dom, errors } = await bootPage(`${BASE}/board.html`);
    const doc = dom.window.document;
    check('board: no JS errors', errors.length === 0, errors.slice(0, 3).join(' | '));
    check('board: grid + controls present', !!doc.getElementById('boardGrid') && !!doc.getElementById('menuStop') && !!doc.getElementById('boardGear'));
    check('board: top menu present (add/delete/stop/hotkeys/audio)', !!doc.getElementById('menuAdd') && !!doc.getElementById('menuDelete') && !!doc.getElementById('menuStop') && !!doc.getElementById('menuHotkeys') && !!doc.getElementById('menuAudio'));
    check('board: audio popover with dual outputs', !!doc.getElementById('audioPop') && !!doc.getElementById('popLocalVol') && !!doc.getElementById('popChannelVol') && !!doc.getElementById('popLocalEnabled') && !!doc.getElementById('popChannelEnabled'));
    check('board: edit popover (group/trim/volume)', !!doc.getElementById('boardEdit') && !!doc.getElementById('editGroup') && !!doc.getElementById('editStart') && !!doc.getElementById('editEnd') && !!doc.getElementById('editVol'));
    check('board: hotkey assignment in the edit popover', !!doc.getElementById('editKey') && !!doc.getElementById('editKeySet') && !!doc.getElementById('editKeyClear'));
    check('board: presets removed everywhere', !doc.getElementById('menuPresets') && !doc.getElementById('addPresetsBtn'));
    check('board: audio popover close button', !!doc.getElementById('popClose'));
    check('board: theme applied from config', doc.body.dataset.theme === 'glass', doc.body.dataset.theme);
    check('board: empty state shown without sounds', doc.getElementById('boardEmpty').hidden === false);
    dom.window.close();
  }
  /* ---- 5. stable port for OBS sources (fallback when busy) ---- */
  {
    const h2 = await startServer({
      port: 0,
      preferredPort: handle.port(),
      host: '127.0.0.1',
      dataDir: join(tmpdir(), 'tsn-ui-test'),
      publicDir: join(ROOT, 'public'),
      log: () => {},
    });
    check('server: preferred port falls back when busy', h2.port() > 0 && h2.port() !== handle.port(), `main=${handle.port()} fallback=${h2.port()}`);
    h2.close();
  }

  /* ---- 6. move-mode + update endpoints (desktop hook contracts) ---- */
  {
    const mv = await fetch(`${BASE}/api/overlay/move`, { method: 'POST' });
    const mvBody = await mv.json();
    check('server: overlay move endpoint answers', mv.ok && mvBody.ok === true, JSON.stringify(mvBody));
    const up = await fetch(`${BASE}/api/update/check`, { method: 'POST' });
    const upBody = await up.json();
    check('server: update-check endpoint answers', up.ok && upBody.ok === true && upBody.applied === false, JSON.stringify(upBody));
    for (const route of ['/api/overlay/moved', '/api/overlay/move-confirm', '/api/overlay/move-cancel', '/api/overlay/size']) {
      const r = await fetch(`${BASE}${route}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ marginX: 1, marginY: 2, w: 292, h: 140 }),
      });
      const b = await r.json();
      check(`server: ${route} answers`, r.ok && b.ok === true, JSON.stringify(b));
    }
    const rs = await fetch(`${BASE}/api/update/restart`, { method: 'POST' });
    const rsBody = await rs.json();
    check('server: update-restart endpoint answers', rs.ok && rsBody.ok === true && rsBody.applied === false, JSON.stringify(rsBody));
    const dp = await fetch(`${BASE}/api/displays`);
    const dpBody = await dp.json();
    check('server: displays reports panel size', dp.ok && dpBody.panelSize?.w === 292 && dpBody.panelSize?.h === 140, JSON.stringify(dpBody.panelSize));
  }

  /* ---- 7. overlay click-through endpoint (desktop hook contract) ---- */
  {
    const res = await fetch(`${BASE}/api/overlay/clickthrough`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ignore: true }),
    });
    const body = await res.json();
    check('server: overlay clickthrough endpoint answers', res.ok && body.ok === true && body.applied === false, JSON.stringify(body));
  }
} finally {
  handle.close();
}

const failed = results.filter((r) => !r).length;
console.log(`\n${results.length - failed}/${results.length} UI checks passed`);
process.exit(failed ? 1 : 0);
