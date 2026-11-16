/**
 * UI smoke test: loads the real overlay page (from the running dev server or a
 * file URL fallback), simulates the WebSocket state stream, and asserts that
 * rows, header, labels and the settings drawer all actually render.
 *
 * Run: npm run test:ui   (requires the dev server on :3000 — started automatically)
 */
import { JSDOM, VirtualConsole } from 'jsdom';
import { spawn } from 'node:child_process';

const PORT = 3100;
const BASE = `http://127.0.0.1:${PORT}`;

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
  desktop: { clickThrough: true, autostart: false, visible: true },
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

/* ---------------- start a dedicated server on :3100 ---------------- */

const serverProc = spawn('npx', ['tsx', 'server/index.ts'], {
  cwd: new URL('..', import.meta.url).pathname,
  env: { ...process.env, PORT: String(PORT), HOST: '127.0.0.1' },
  stdio: 'ignore',
});

await new Promise((r) => setTimeout(r, 2500));

try {
  /* ---- 1. overlay page (clean mode, like the in-game overlay window) ---- */
  {
    const { dom, errors } = await bootPage(`${BASE}/?clean=1`);
    const doc = dom.window.document;

    check('overlay: no JS errors', errors.length === 0, errors.slice(0, 3).join(' | '));
    check('overlay: channel name rendered', doc.getElementById('channelNameText')?.textContent === 'UI Test Channel', doc.getElementById('channelNameText')?.textContent);
    check('overlay: server name rendered', doc.getElementById('serverName')?.textContent === 'UI Test Server');
    check('overlay: user rows rendered', doc.querySelectorAll('#userList .row').length === 2, `rows=${doc.querySelectorAll('#userList .row').length}`);
    check('overlay: query bot hidden', !doc.querySelector('#userList .row.self')?.nextSibling?.textContent?.includes?.('QueryBot') && ![...doc.querySelectorAll('.nick')].some((n) => n.textContent === 'QueryBot'));
    check('overlay: talking row highlighted', !!doc.querySelector('#userList .row.talking'));
    check('overlay: self row marked', !!doc.querySelector('#userList .row.self'));
    check('overlay: muted badge shown', !!doc.querySelector('#userList .badge.mic-off'));
    check('overlay: away shown', !!doc.querySelector('#userList .row.away'));
    check('overlay: settings button hidden in clean mode', doc.getElementById('settingsBtn').style.display === 'none');
    check('overlay: drawer hidden initially', doc.getElementById('settingsDrawer').hidden === true);
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
    const before = dom.window.document.documentElement.style.getPropertyValue('--accent');
    doc.querySelector('#accentSwatches .swatch[data-value="#22d3ee"]').click();
    const after = dom.window.document.documentElement.style.getPropertyValue('--accent');
    check('settings: accent change applies live', after === '#22d3ee', `${before} → ${after}`);
    doc.querySelector('[data-toggle="compact"]').click();
    check('settings: compact toggle applies class', dom.window.document.body.classList.contains('compact'));
    dom.window.close();
  }
} finally {
  serverProc.kill('SIGTERM');
}

const failed = results.filter((r) => !r).length;
console.log(`\n${results.length - failed}/${results.length} UI checks passed`);
process.exit(failed ? 1 : 0);
