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
    check('settings: chat delay slider', !!doc.querySelector('[data-slider="chatDelay"]'));
    check('settings: whisper/chat toggles', !!doc.querySelector('[data-toggle="showWhispers"]') && !!doc.querySelector('[data-toggle="showChat"]') && !!doc.querySelector('[data-toggle="chatPrivate"]'));
    check('settings: sound tab wired', !!doc.getElementById('soundItems') && !!doc.getElementById('audioOutput') && !!doc.getElementById('stopAllBtn') && !!doc.getElementById('clearHotkeysBtn'));
    // the whole sound feature is parked: tab button and panel are hidden
    const soundTabBtn = doc.querySelector('.tab[data-tab="sound"]');
    const soundPanel = doc.querySelector('[data-panel="sound"]');
    check('settings: sound tab hidden (feature parked)', soundTabBtn?.hidden === true && soundPanel?.hidden === true);
    check('settings: server groups in a collapsed block at the bottom', !!doc.getElementById('ts6GroupsBlock') && doc.getElementById('ts6GroupsBlock').open === false);
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
} finally {
  handle.close();
}

const failed = results.filter((r) => !r).length;
console.log(`\n${results.length - failed}/${results.length} UI checks passed`);
process.exit(failed ? 1 : 0);
