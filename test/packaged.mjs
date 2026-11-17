/**
 * Packaged-layout test: reproduces the exact file layout electron-builder
 * ships (resources/app/{app/dist/main.cjs, public/**, package.json} +
 * resources/icons) and verifies the embedded server serves every asset.
 *
 * Run: npm run test:packaged
 */
import { cp, mkdir, rm, writeFile, access } from 'node:fs/promises';
import { spawn, spawnSync } from 'node:child_process';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const PKG = join(ROOT, '.tmp-pkg');

// Always build a fresh bundle — a stale app/dist/main.cjs would test old code.
{
  const r = spawnSync('npm', ['run', 'bundle'], { cwd: ROOT, shell: true, stdio: 'inherit' });
  if (r.status !== 0) process.exit(1);
}

const results = [];
function check(name, cond, extra = '') {
  results.push(!!cond);
  console.log(`${cond ? 'PASS' : 'FAIL'}  ${name}${extra ? ' — ' + extra : ''}`);
}

// Build the packaged layout
await rm(PKG, { recursive: true, force: true });
const APP_DIR = join(PKG, 'resources', 'app');
const RES_DIR = join(PKG, 'resources');
await mkdir(join(APP_DIR, 'app', 'dist'), { recursive: true });
await cp(join(ROOT, 'public'), join(APP_DIR, 'public'), { recursive: true });
await cp(join(ROOT, 'app', 'dist', 'main.cjs'), join(APP_DIR, 'app', 'dist', 'main.cjs'));
await cp(join(ROOT, 'app', 'icons'), join(RES_DIR, 'icons'), { recursive: true });
await writeFile(join(APP_DIR, 'package.json'), JSON.stringify({ name: 'ts3-ts6-notifier', main: 'app/dist/main.cjs' }));

// v1.3.1: the Ctrl+Shift+M move hotkey is gone for good
{
  const fs = await import('node:fs/promises');
  const mainSrc = await fs.readFile(new URL('../app/main.ts', import.meta.url), 'utf8');
  check('packaged: move hotkey removed from main', !/CommandOrControl\+Shift\+M|registerMoveHotkey/.test(mainSrc));
  // v1.3.2: all four screen corners really anchor (bl/br used to mirror tl/tr)
  const css = await fs.readFile(new URL('../public/css/overlay.css', import.meta.url), 'utf8');
  const bl = /data-corner="bl"[^#]*#panel\s*\{[^}]*bottom:\s*var\(--margin-y\)/.test(css);
  const br = /data-corner="br"[^#]*#panel\s*\{[^}]*bottom:\s*var\(--margin-y\)/.test(css);
  check('packaged: bottom corners anchor to the bottom edge', bl && br);
}

// Boot the bundled main against that layout (electron stubbed)
const proc = spawn('node', ['-r', './test/electron-stub.cjs', 'app/dist/main.cjs'], {
  cwd: ROOT,
  env: { ...process.env, TSN_APP_PATH: APP_DIR, TSN_RESOURCES_PATH: RES_DIR, TSN_SMOKE_EXIT: '0' },
  stdio: ['ignore', 'pipe', 'pipe'],
});

let out = '';
proc.stdout.on('data', (d) => (out += d.toString()));
proc.stderr.on('data', (d) => (out += d.toString()));

// Find the port the server printed
let port = 0;
for (let i = 0; i < 60 && !port; i++) {
  await new Promise((r) => setTimeout(r, 250));
  const m = /running at http:\/\/127\.0\.0\.1:(\d+)/.exec(out);
  if (m) port = Number(m[1]);
}

try {
  check('packaged: server started from packaged layout', port > 0, `port=${port}`);

  if (port > 0) {
    const pages = [
      ['/', 'text/html', 'channelNameText'],
      ['/css/overlay.css', 'text/css', '[hidden]'],
      ['/js/app.js', 'text/javascript', 'TSIcons'],
      ['/js/icons.js', 'text/javascript', 'avatarGradient'],
      ['/js/settings.js', 'text/javascript', 'openDrawer'],
      ['/js/i18n.js', 'text/javascript', 'setLanguage'],
      ['/favicon.svg', 'image/svg+xml', '<svg'],
    ];
    for (const [path, mime, marker] of pages) {
      const res = await fetch(`http://127.0.0.1:${port}${path}`);
      const body = await res.text();
      check(`packaged: ${path} → ${mime}`, res.status === 200 && (res.headers.get('content-type') || '').includes(mime), `status=${res.status}`);
      check(`packaged: ${path} content ok`, body.includes(marker), marker);
    }
    const api = await fetch(`http://127.0.0.1:${port}/api/state`);
    check('packaged: /api/state works', api.status === 200);
    const pst = await fetch(`http://127.0.0.1:${port}/api/plugin/status`);
    const pstBody = await pst.json();
    check('packaged: /api/plugin/status works', pst.status === 200 && typeof pstBody.active === 'boolean' && typeof pstBody.dll === 'boolean', JSON.stringify(pstBody));
    const pins = await fetch(`http://127.0.0.1:${port}/api/plugin/install`, { method: 'POST' });
    const pinsBody = await pins.json();
    check('packaged: /api/plugin/install responds safely', pins.status === 200 && (pinsBody.ok === true || typeof pinsBody.error === 'string'), JSON.stringify(pinsBody));
    const diag = await fetch(`http://127.0.0.1:${port}/api/diagnostics`);
    const diagText = await diag.text();
    check('packaged: /api/diagnostics report builds', diag.status === 200 && /TS3\/TS6 traffic/.test(diagText) && /TS3-TS6 Notifier diagnostics/.test(diagText), `len=${diagText.length}`);
    const wsUrl = `ws://127.0.0.1:${port}/ws`;
    const { default: WebSocket } = await import('ws');
    const alive = await new Promise((resolve) => {
      const ws = new WebSocket(wsUrl);
      ws.on('open', () => {
        ws.close();
        resolve(true);
      });
      ws.on('error', () => resolve(false));
    });
    check('packaged: websocket upgrade works', alive);
  }
} finally {
  proc.kill('SIGTERM');
  await rm(PKG, { recursive: true, force: true });
}

const failed = results.filter((r) => !r).length;
console.log(`\n${results.length - failed}/${results.length} packaged checks passed`);
process.exit(failed ? 1 : 0);
