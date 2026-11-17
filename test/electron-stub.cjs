/**
 * Minimal 'electron' module mock for smoke-testing the bundled desktop main
 * process without a display:
 *   node -r ./test/electron-stub.cjs app/dist/main.cjs
 *
 * Env:
 *   TSN_APP_PATH       — override app.getAppPath() (packaged-layout tests)
 *   TSN_RESOURCES_PATH — override process.resourcesPath equivalent
 *   TSN_SMOKE_EXIT=0   — don't auto-exit after 4s (external kill instead)
 */
const Module = require('module');
const os = require('os');
const path = require('path');

function noop() {}

const workArea = { x: 0, y: 0, width: 1920, height: 1080 };
const APP_PATH = process.env.TSN_APP_PATH || path.join(__dirname, '..');
const RES_PATH = process.env.TSN_RESOURCES_PATH || APP_PATH;
// Electron injects process.resourcesPath at runtime — emulate it.
process.resourcesPath = RES_PATH;

class BrowserWindow {
  constructor(opts = {}) {
    this.opts = opts;
    this.bounds = { x: opts.x ?? 0, y: opts.y ?? 0, width: opts.width ?? 800, height: opts.height ?? 600 };
    this.webContents = { openDevTools: noop, on: noop, send: noop, setWindowOpenHandler: noop };
    console.log('[stub] BrowserWindow created', JSON.stringify({
      w: opts.width, h: opts.height, transparent: opts.transparent, frame: opts.frame,
      alwaysOnTop: opts.alwaysOnTop, skipTaskbar: opts.skipTaskbar, focusable: opts.focusable,
    }));
  }
  setMenuBarVisibility() {}
  setAlwaysOnTop(flag, level) {
    console.log('[stub] alwaysOnTop =', !!flag, level || '');
  }
  setVisibleOnAllWorkspaces() {}
  setFullScreen(flag) {
    this.fullscreen = !!flag;
    console.log('[stub] fullscreen =', !!flag);
  }
  isFullScreen() {
    return !!this.fullscreen;
  }
  isVisible() {
    return this.visible !== false;
  }
  setIgnoreMouseEvents(flag) {
    console.log('[stub] click-through =', !!flag);
  }
  loadURL(url) {
    console.log('[stub] loadURL', url);
    fetch(url).then(
      (r) => console.log('[stub] page status', r.status),
      (e) => console.log('[stub] page fetch failed', e.message),
    );
  }
  once(ev, cb) {
    if (ev === 'ready-to-show') setTimeout(cb, 50);
  }
  on() {}
  show() {
    this.visible = true;
  }
  showInactive() {
    this.visible = true;
    console.log('[stub] overlay shown');
  }
  hide() {
    this.visible = false;
    console.log('[stub] overlay hidden');
  }
  focus() {}
  isDestroyed() {
    return false;
  }
  getBounds() {
    return this.bounds;
  }
  setBounds(b) {
    this.bounds = { ...this.bounds, ...b };
  }
  close() {}
}

class Tray {
  constructor(img) {
    console.log('[stub] Tray created');
  }
  setToolTip(t) {
    console.log('[stub] tray tooltip:', t);
  }
  setContextMenu(menu) {
    const labels = (menu?.items ?? []).map((i) => i.label || '---');
    console.log('[stub] tray menu:', labels.join(' | '));
  }
  on() {}
}

const electron = {
  app: {
    isPackaged: !!process.env.TSN_APP_PATH,
    getLocale: () => 'en-US',
    getVersion: () => process.env.TSN_STUB_VERSION || '1.0.0',
    getAppPath: () => APP_PATH,
    getPath: (name) => path.join(os.tmpdir(), 'tsn-stub', name),
    setAppUserModelId: noop,
    commandLine: { appendSwitch: noop },
    getAppMetrics: () => [],
    requestSingleInstanceLock: () => true,
    setLoginItemSettings: (o) => console.log('[stub] login item =', JSON.stringify(o)),
    getLoginItemSettings: () => ({ openAtLogin: false }),
    whenReady: () => Promise.resolve(),
    on(ev) {
      console.log('[stub] app.on', ev);
    },
    quit: noop,
    exit: (code) => process.exit(code ?? 0),
  },
  BrowserWindow,
  Tray,
  Menu: {
    buildFromTemplate: (t) => ({ items: t }),
    setApplicationMenu: noop,
  },
  nativeImage: { createFromPath: () => ({}) },
  globalShortcut: { register: () => true, unregisterAll: noop, isRegistered: () => false },
  clipboard: { writeText: noop, readText: () => '' },
  Notification: class { show() {} on() {} },
  shell: { openExternal: async () => {} },
  screen: {
    getPrimaryDisplay: () => ({ workArea, bounds: { x: 0, y: 0, width: 1920, height: 1080 }, id: 1 }),
    getAllDisplays: () => [{ workArea, bounds: { x: 0, y: 0, width: 1920, height: 1080 }, id: 1, label: 'Stub', primary: true }],
    on: noop,
  },
  shell: { openExternal: noop },
  ipcMain: { on: noop },
};

const origLoad = Module._load;
Module._load = function (request, ...rest) {
  if (request === 'electron') return electron;
  return origLoad.call(this, request, ...rest);
};

if (process.env.TSN_SMOKE_EXIT !== '0') {
  setTimeout(() => {
    console.log('BOOT OK');
    process.exit(0);
  }, 4000);
}
