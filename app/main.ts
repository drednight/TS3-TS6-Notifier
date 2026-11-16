/**
 * TS3-TS6 Notifier — desktop shell.
 *
 * A transparent, always-on-top, click-through overlay window over games plus a
 * tray icon and a separate settings window. The embedded HTTP/WS server and
 * TeamSpeak connectors are the exact same code as the standalone web version.
 */
import { appendFileSync, mkdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import {
  app,
  BrowserWindow,
  Menu,
  nativeImage,
  screen,
  Tray,
} from 'electron';
import { startServer, type AppHandle } from '../server/app.js';

const IS_PACKAGED = app.isPackaged;
const ROOT = IS_PACKAGED ? app.getAppPath() : join(__dirname, '..', '..');
const PUBLIC_DIR = join(ROOT, 'public');
const ICONS_DIR = IS_PACKAGED ? join(process.resourcesPath, 'icons') : join(ROOT, 'app', 'icons');
const DATA_DIR = join(app.getPath('userData'), 'data');
const LOG_FILE = join(app.getPath('userData'), 'app.log');

/* ---------------- logging (file + console) ---------------- */

let logReady = false;
function log(msg: string): void {
  const line = `[${new Date().toISOString()}] ${msg}`;
  console.log(line);
  if (!logReady) return;
  try {
    appendFileSync(LOG_FILE, line + '\n');
  } catch {
    /* ignore */
  }
}

function verifyAssets(): void {
  const probes = ['index.html', 'css/overlay.css', 'js/app.js', 'js/settings.js', 'js/i18n.js', 'js/icons.js'];
  const parts = probes.map((p) => {
    try {
      statSync(join(PUBLIC_DIR, p));
      return `${p}=ok`;
    } catch {
      return `${p}=MISSING`;
    }
  });
  log(`[app] assets in ${PUBLIC_DIR}: ${parts.join(' ')}`);
  log(`[app] icons dir: ${ICONS_DIR}, data dir: ${DATA_DIR}, packaged=${IS_PACKAGED}`);
}

let handle: AppHandle | null = null;
let overlayWin: BrowserWindow | null = null;
let settingsWin: BrowserWindow | null = null;
let tray: Tray | null = null;
let overlayVisible = true;

app.setAppUserModelId('dev.tsnotifier.app');

/* ---------------- single instance ---------------- */

const gotLock = app.requestSingleInstanceLock();
if (!gotLock) {
  app.quit();
} else {
  app.on('second-instance', () => openSettings());
}

/* ---------------- windows ---------------- */

function overlayUrl(): string {
  return `http://127.0.0.1:${handle?.port() ?? 0}/?clean=1`;
}

function settingsUrl(): string {
  return `http://127.0.0.1:${handle?.port() ?? 0}/?open=settings`;
}

function fitOverlay(): void {
  if (!overlayWin) return;
  const workArea = screen.getPrimaryDisplay().workArea;
  overlayWin.setBounds(workArea);
}

function applyClickThrough(): void {
  if (!overlayWin) return;
  const on = handle?.store.get().desktop.clickThrough ?? true;
  overlayWin.setIgnoreMouseEvents(!!on, { forward: true });
}

/** Log renderer console + page failures; retry loading on failure. */
function watchWebContents(win: BrowserWindow, tag: string): void {
  win.webContents.on('console-message', (_e, level, message) => {
    if (level >= 2) log(`[${tag}] console: ${message}`);
  });
  win.webContents.on('did-fail-load', (_e, code, desc, url) => {
    log(`[${tag}] did-fail-load code=${code} ${desc} url=${url}`);
    setTimeout(() => {
      if (!win.isDestroyed()) win.loadURL(tag === 'overlay' ? overlayUrl() : settingsUrl());
    }, 1200);
  });
}

function createOverlay(): void {
  const workArea = screen.getPrimaryDisplay().workArea;
  overlayWin = new BrowserWindow({
    x: workArea.x,
    y: workArea.y,
    width: workArea.width,
    height: workArea.height,
    transparent: true,
    frame: false,
    resizable: false,
    movable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    focusable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    hasShadow: false,
    show: false,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      devTools: !IS_PACKAGED,
    },
  });

  overlayWin.setMenuBarVisibility(false);
  overlayWin.setAlwaysOnTop(true, 'screen-saver');
  overlayWin.setVisibleOnAllWorkspaces?.(true, { visibleOnFullScreen: true });
  watchWebContents(overlayWin, 'overlay');
  overlayWin.loadURL(overlayUrl());

  // ready-to-show is unreliable for transparent windows — always fall back.
  let shown = false;
  const show = () => {
    if (shown || !overlayWin || overlayWin.isDestroyed()) return;
    shown = true;
    if (overlayVisible) overlayWin.showInactive();
    applyClickThrough();
    log('[app] overlay shown');
  };
  overlayWin.once('ready-to-show', show);
  setTimeout(show, 1500);

  screen.on('display-metrics-changed', fitOverlay);
  screen.on('display-added', fitOverlay);
  screen.on('display-removed', fitOverlay);
  log(`[app] overlay bounds ${workArea.width}x${workArea.height}+${workArea.x}+${workArea.y}`);
}

function openSettings(): void {
  if (settingsWin && !settingsWin.isDestroyed()) {
    settingsWin.show();
    settingsWin.focus();
    return;
  }
  settingsWin = new BrowserWindow({
    width: 392,
    height: 780,
    minWidth: 340,
    minHeight: 520,
    autoHideMenuBar: true,
    icon: nativeImage.createFromPath(join(ICONS_DIR, 'icon.png')),
    title: 'TS3-TS6 Notifier — Settings',
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      devTools: !IS_PACKAGED,
    },
  });
  settingsWin.setMenuBarVisibility(false);
  watchWebContents(settingsWin, 'settings');
  settingsWin.loadURL(settingsUrl());
  settingsWin.on('closed', () => {
    settingsWin = null;
  });
  log('[app] settings window opened');
}

/* ---------------- tray ---------------- */

function setOverlayVisible(visible: boolean): void {
  overlayVisible = visible;
  if (!overlayWin) return;
  if (visible) {
    overlayWin.showInactive();
  } else {
    overlayWin.hide();
  }
  handle?.store.update({ desktop: { visible } });
  buildTrayMenu();
}

function setSource(source: 'mock' | 'ts3' | 'ts6'): void {
  handle?.store.update({ source });
  handle?.restartConnector();
  buildTrayMenu();
  log(`[app] data source → ${source}`);
}

function toggleClickThrough(): void {
  const current = handle?.store.get().desktop.clickThrough ?? true;
  handle?.store.update({ desktop: { clickThrough: !current } });
  applyClickThrough();
  buildTrayMenu();
}

function toggleAutostart(): void {
  const current = handle?.store.get().desktop.autostart ?? false;
  handle?.store.update({ desktop: { autostart: !current } });
  applyAutostart();
  buildTrayMenu();
}

function applyAutostart(): void {
  const { autostart } = handle?.store.get().desktop ?? { autostart: false };
  try {
    app.setLoginItemSettings({
      openAtLogin: autostart,
      path: process.execPath,
      args: [],
    });
  } catch (err) {
    log('[app] autostart error: ' + err);
  }
}

function buildTrayMenu(): void {
  if (!tray || !handle) return;
  const cfg = handle.store.get();
  const template: Electron.MenuItemConstructorOptions[] = [
    {
      label: 'Settings…',
      click: () => openSettings(),
    },
    { type: 'separator' },
    {
      label: 'Data source',
      submenu: [
        { label: 'Demo mode', type: 'radio', checked: cfg.source === 'mock', click: () => setSource('mock') },
        { label: 'TeamSpeak 3 (ClientQuery)', type: 'radio', checked: cfg.source === 'ts3', click: () => setSource('ts3') },
        { label: 'TeamSpeak 6 (Remote Apps)', type: 'radio', checked: cfg.source === 'ts6', click: () => setSource('ts6') },
      ],
    },
    {
      label: 'Reconnect',
      click: () => handle?.restartConnector(),
    },
    { type: 'separator' },
    {
      label: 'Click-through (lock overlay)',
      type: 'checkbox',
      checked: cfg.desktop.clickThrough,
      click: () => toggleClickThrough(),
    },
    {
      label: 'Launch at startup',
      type: 'checkbox',
      checked: cfg.desktop.autostart,
      click: () => toggleAutostart(),
    },
    {
      label: 'Show overlay',
      type: 'checkbox',
      checked: overlayVisible,
      click: () => setOverlayVisible(!overlayVisible),
    },
    { type: 'separator' },
    {
      label: 'Quit',
      click: () => app.quit(),
    },
  ];
  tray.setContextMenu(Menu.buildFromTemplate(template));
}

function createTray(): void {
  tray = new Tray(nativeImage.createFromPath(join(ICONS_DIR, 'tray.png')));
  tray.setToolTip('TS3-TS6 Notifier');
  tray.on('double-click', () => openSettings());
  buildTrayMenu();
  log('[app] tray ready');
}

/* ---------------- boot ---------------- */

async function boot(): Promise<void> {
  try {
    mkdirSync(app.getPath('userData'), { recursive: true });
    logReady = true;
  } catch {
    /* fall through — console logging still works */
  }
  log('=== TS3-TS6 Notifier starting ===');
  verifyAssets();

  handle = await startServer({
    port: 0,
    host: '127.0.0.1',
    dataDir: DATA_DIR,
    publicDir: PUBLIC_DIR,
    log,
  });

  overlayVisible = handle.store.get().desktop.visible !== false;
  createOverlay();
  createTray();
  applyAutostart();

  // First run: open the settings window so the UI is immediately visible.
  const cfg = handle.store.get();
  if (!cfg.desktop.seen) {
    handle.store.update({ desktop: { seen: true } });
    setTimeout(() => openSettings(), 600);
    log('[app] first run → opening settings');
  }
  log('[app] ready');
}

if (gotLock) {
  app.whenReady().then(boot).catch((err) => {
    console.error('[app] fatal:', err);
    try {
      appendFileSync(LOG_FILE, `FATAL ${err instanceof Error ? err.stack : String(err)}\n`);
    } catch {
      /* ignore */
    }
    app.exit(1);
  });

  app.on('window-all-closed', () => {
    // keep running in the tray on all platforms
  });

  app.on('before-quit', () => {
    log('=== shutting down ===');
    handle?.close();
  });
}
