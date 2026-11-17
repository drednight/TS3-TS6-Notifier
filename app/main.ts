/**
 * TS3-TS6 Notifier — desktop shell.
 *
 * A transparent, always-on-top, click-through overlay window over games plus a
 * tray icon and a separate settings window. The embedded HTTP/WS server and
 * TeamSpeak connectors are the exact same code as the standalone web version.
 */
import { spawnSync } from 'node:child_process';
import { appendFileSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import {
  app,
  BrowserWindow,
  clipboard,
  globalShortcut,
  Menu,
  nativeImage,
  Notification,
  screen,
  Tray,
} from 'electron';
import { autoUpdater } from 'electron-updater';
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
let boardWin: BrowserWindow | null = null;
const childWindows = new Map<string, BrowserWindow>();
let tray: Tray | null = null;
let screenHooksInstalled = false;
let overlayVisible = true;

app.setAppUserModelId('dev.tsnotifier.app');

/* ---------------- auto update (GitHub Releases) ---------------- */

/**
 * Self-update via electron-updater: on packaged builds the app checks the
 * GitHub release for a newer version, downloads the NSIS installer in the
 * background and installs it silently on quit.
 */
function setupAutoUpdate(): void {
  if (!app.isPackaged) return;
  try {
    autoUpdater.setFeedURL({ provider: 'github', owner: 'drednight', repo: 'TS3-TS6-Notifier' });
    autoUpdater.autoDownload = true;
    autoUpdater.autoInstallOnAppQuit = true;
    autoUpdater.on('checking-for-update', () => {
      log('[update] checking for updates');
      handle?.broadcast({ type: 'update-status', status: 'checking' });
    });
    autoUpdater.on('update-available', (info) => {
      log(`[update] available: ${info.version}`);
      handle?.broadcast({ type: 'update-status', status: 'available', version: info.version });
    });
    autoUpdater.on('update-not-available', () => {
      log('[update] already up to date');
      handle?.broadcast({ type: 'update-status', status: 'latest' });
    });
    autoUpdater.on('update-downloaded', (info) => {
      log(`[update] ${info.version} downloaded — ready to restart`);
      handle?.broadcast({ type: 'update-status', status: 'downloaded', version: info.version });
      try {
        // Offer an immediate restart: the toast action, clicking the toast,
        // and the «Перезапустить сейчас» button in «Ещё» all install + relaunch.
        const notification = new Notification({
          title: tr('updTitle'),
          body: `${tr('updDownloaded').replace('{v}', info.version)} ${tr('updRestartToast')}`,
          actions: [{ type: 'button', text: tr('updRestartAction') }],
        });
        notification.on('action', () => tryQuitAndInstall());
        notification.on('click', () => tryQuitAndInstall());
        notification.show();
      } catch {
        /* notification unavailable */
      }
    });
    autoUpdater.on('error', (err) => {
      log('[update] error: ' + err);
      handle?.broadcast({ type: 'update-status', status: 'error' });
    });
    void autoUpdater.checkForUpdates().catch((err) => log('[update] check failed: ' + err));
    setInterval(() => {
      void autoUpdater.checkForUpdates().catch(() => {});
    }, 4 * 60 * 60 * 1000);
  } catch (err) {
    log('[update] setup failed: ' + err);
  }
}

function tryQuitAndInstall(): void {
  if (!app.isPackaged) {
    log('[update] dev build — nothing to install');
    return;
  }
  log('[update] quit & install requested');
  try {
    autoUpdater.quitAndInstall(true, true);
  } catch (err) {
    log('[update] quitAndInstall failed: ' + err);
  }
}

function checkForUpdatesNow(): void {
  if (!app.isPackaged) {
    log('[update] dev build — nothing to check');
    handle?.broadcast({ type: 'update-status', status: 'error' });
    return;
  }
  void autoUpdater.checkForUpdates().catch((err) => log('[update] check failed: ' + err));
}

/* ---------------- elevation (Windows) ---------------- */

const ELEVATED_ARG = '--elevated';

/**
 * "Run as administrator" (desktop.elevate, default ON): before grabbing the
 * single-instance lock, relaunch ourselves through the UAC runas verb. The
 * elevated copy carries --elevated so it never loops. If the user cancels the
 * UAC prompt or elevation fails, we simply keep running unprivileged.
 */
function tryRelaunchElevated(): boolean {
  if (!app.isPackaged || process.platform !== 'win32') return false;
  if (process.argv.includes(ELEVATED_ARG)) return false;
  try {
    const raw = JSON.parse(readFileSync(join(DATA_DIR, 'config.json'), 'utf8')) as {
      desktop?: { elevate?: boolean };
    };
    if (raw?.desktop?.elevate === false) return false;
  } catch {
    /* no config yet → the default (on) applies */
  }
  try {
    // `net session` only succeeds with an elevated administrator token.
    if (spawnSync('net', ['session'], { stdio: 'ignore', windowsHide: true }).status === 0) return false;
  } catch {
    return false;
  }
  try {
    const exe = process.execPath.replace(/'/g, "''");
    const res = spawnSync(
      'powershell.exe',
      [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        `Start-Process -FilePath '${exe}' -ArgumentList @('${ELEVATED_ARG}') -Verb RunAs`,
      ],
      { stdio: 'ignore', windowsHide: true, timeout: 60000 },
    );
    return res.status === 0;
  } catch {
    return false;
  }
}

if (tryRelaunchElevated()) {
  // Exit BEFORE taking the single-instance lock so the elevated copy can grab it.
  app.exit(0);
}

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
  // bare=1 → the settings window shows the settings UI only (no overlay page).
  return `http://127.0.0.1:${handle?.port() ?? 0}/?open=settings&bare=1`;
}

function boardUrl(): string {
  return `http://127.0.0.1:${handle?.port() ?? 0}/board.html`;
}

function fitOverlay(): void {
  if (!overlayWin) return;
  const want = handle?.store.get().desktop.display ?? 'primary';
  let target = screen.getPrimaryDisplay();
  if (want && want !== 'primary') {
    const found = screen.getAllDisplays().find((d) => String(d.id) === want);
    if (found) target = found;
  }
  overlayWin.setBounds(target.workArea);
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
      if (!win.isDestroyed()) {
        win.loadURL(tag === 'overlay' ? overlayUrl() : tag === 'board' ? boardUrl() : settingsUrl());
      }
    }, 1200);
  });
  // Child pages (settings/board) open via window.open — keep ONE window per
  // name instead of spawning a new BrowserWindow on every click.
  win.webContents.setWindowOpenHandler(({ url, frameName }) => {
    try {
      const target = new URL(url, settingsUrl());
      if (target.origin === new URL(settingsUrl()).origin) {
        const name = frameName || 'tsn-child';
        // Reuse the app's own single-instance windows when possible.
        const managed = name === 'tsn-board' ? boardWin : name === 'tsn-settings' ? settingsWin : null;
        const existing = managed && !managed.isDestroyed() ? managed : childWindows.get(name);
        if (existing && !existing.isDestroyed()) {
          if (managed === settingsWin) void existing.loadURL(target.toString());
          existing.focus();
          return { action: 'deny' };
        }
        const child = new BrowserWindow({
          width: name === 'tsn-board' ? 540 : 430,
          height: name === 'tsn-board' ? 560 : 820,
          autoHideMenuBar: true,
          title: 'TS3-TS6 Notifier',
        });
        childWindows.set(name, child);
        child.on('closed', () => childWindows.delete(name));
        watchWebContents(child, name === 'tsn-board' ? 'board' : 'settings');
        void child.loadURL(target.toString());
        return { action: 'deny' };
      }
    } catch {
      /* fall through */
    }
    return { action: 'deny' };
  });
}

function createOverlay(): void {
  const want = handle?.store.get().desktop.display ?? 'primary';
  let target = screen.getPrimaryDisplay();
  if (want && want !== 'primary') {
    const found = screen.getAllDisplays().find((d) => String(d.id) === want);
    if (found) target = found;
  }
  const workArea = target.workArea;
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

  if (!screenHooksInstalled) {
    screenHooksInstalled = true;
    screen.on('display-metrics-changed', fitOverlay);
    screen.on('display-added', fitOverlay);
    screen.on('display-removed', fitOverlay);
  }
  log(`[app] overlay bounds ${workArea.width}x${workArea.height}+${workArea.x}+${workArea.y}`);
}

function openSettings(): void {
  if (settingsWin && !settingsWin.isDestroyed()) {
    settingsWin.show();
    settingsWin.focus();
    return;
  }
  // Settings open in their own window; the overlay stays visible on top.
  settingsWin = new BrowserWindow({
    width: 480,
    height: 880,
    minWidth: 400,
    minHeight: 640,
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

/* ---------------- soundboard window ---------------- */

function openBoard(): void {
  if (boardWin && !boardWin.isDestroyed()) {
    boardWin.show();
    boardWin.focus();
    return;
  }
  boardWin = new BrowserWindow({
    width: 520,
    height: 440,
    minWidth: 340,
    minHeight: 260,
    autoHideMenuBar: true,
    icon: nativeImage.createFromPath(join(ICONS_DIR, 'icon.png')),
    title: 'TS3-TS6 Notifier — Soundboard',
    alwaysOnTop: true,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      devTools: !IS_PACKAGED,
    },
  });
  boardWin.setMenuBarVisibility(false);
  watchWebContents(boardWin, 'board');
  boardWin.loadURL(boardUrl());
  boardWin.on('closed', () => {
    boardWin = null;
  });
  log('[app] soundboard window opened');
}

/* ---------------- tray ---------------- */

/**
 * The overlay window is optional: in "server-only" mode the app keeps just the
 * small Node core (HTTP/WS + connector) running and the overlay is rendered
 * on demand elsewhere — e.g. as an OBS browser source. That saves a whole
 * renderer process plus the fullscreen compositor window.
 */
function applyOverlayWindow(): void {
  const want = handle?.store.get().desktop.overlayWindow !== false;
  if (want && !overlayWin) {
    createOverlay();
    log('[app] overlay window created');
  } else if (!want && overlayWin) {
    overlayWin.destroy();
    overlayWin = null;
    log('[app] overlay window closed (server-only mode)');
  }
}

function copyOverlayUrl(): void {
  const url = `http://127.0.0.1:${handle?.port() ?? 0}/?clean=1`;
  try {
    clipboard.writeText(url);
    log(`[app] overlay URL copied: ${url}`);
  } catch {
    log(`[app] overlay URL: ${url}`);
  }
}

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

/* ---------------- soundboard hotkeys ---------------- */

/**
 * Soundboard + sound settings are parked (hidden in the UI) for now at the
 * user's request — flip this to false together with unhiding the Sound tab
 * in public/index.html to bring the feature back.
 */
const SOUNDS_UI_HIDDEN = true;

function registerSoundHotkeys(): void {
  if (!handle || SOUNDS_UI_HIDDEN) return;
  try {
    globalShortcut.unregisterAll();
    const { items, enabled, hotkeysEnabled } = handle.store.get().sounds;
    if (!enabled || !hotkeysEnabled) return;
    for (const item of items) {
      if (!item.hotkey) continue;
      const ok = globalShortcut.register(item.hotkey, () => {
        log(`[app] sound hotkey ${item.hotkey} → ${item.name}`);
        handle?.broadcast({ type: 'sound', id: item.id });
      });
      if (!ok) log(`[app] hotkey ${item.hotkey} unavailable`);
    }
  } catch (err) {
    log(`[app] hotkey registration error: ${err instanceof Error ? err.message : err}`);
  }
}

/* ---------------- tray i18n ---------------- */

const TRAY_STRINGS: Record<string, Record<string, string>> = {
  ru: {
    settings: 'Настройки…',
    checkUpdates: 'Проверить обновления',
    overlayWindow: 'Оверлей-окно',
    copyUrl: 'Скопировать ссылку оверлея',
    dataSource: 'Источник данных',
    demo: 'Демо-режим',
    ts3: 'TeamSpeak 3 (ClientQuery)',
    ts6: 'TeamSpeak 6 (Remote Apps)',
    reconnect: 'Переподключиться',
    clickThrough: 'Клик-сквозной (закрепить оверлей)',
    autostart: 'Запускать со старта',
    showOverlay: 'Показывать оверлей',
    quit: 'Выход',
    updTitle: 'TS3-TS6 Notifier — обновление',
    updDownloaded: 'Версия {v} скачана.',
    updRestartAction: 'Перезапустить сейчас',
    updRestartToast: 'Нажмите, чтобы перезапустить приложение сейчас — или оно обновится при выходе.',
  },
  en: {
    settings: 'Settings…',
    checkUpdates: 'Check for updates',
    overlayWindow: 'Overlay window',
    copyUrl: 'Copy overlay URL',
    dataSource: 'Data source',
    demo: 'Demo mode',
    ts3: 'TeamSpeak 3 (ClientQuery)',
    ts6: 'TeamSpeak 6 (Remote Apps)',
    reconnect: 'Reconnect',
    clickThrough: 'Click-through (lock overlay)',
    autostart: 'Launch at startup',
    showOverlay: 'Show overlay',
    quit: 'Quit',
    updTitle: 'TS3-TS6 Notifier — update',
    updDownloaded: 'Version {v} downloaded.',
    updRestartAction: 'Restart now',
    updRestartToast: 'Click to restart the app now — otherwise it updates when you quit.',
  },
};

function tr(key: string): string {
  const raw = handle?.store.get().appearance.language ?? 'auto';
  let lang: string = raw === 'en' || raw === 'ru' ? raw : 'en';
  if (raw !== 'en' && raw !== 'ru') {
    try {
      if (typeof app.getLocale === 'function' && app.getLocale().startsWith('ru')) lang = 'ru';
    } catch {
      /* locale unavailable */
    }
  }
  return TRAY_STRINGS[lang]?.[key] ?? TRAY_STRINGS.en?.[key] ?? key;
}

function buildTrayMenu(): void {
  if (!tray || !handle) return;
  const cfg = handle.store.get();
  const template: Electron.MenuItemConstructorOptions[] = [
    {
      label: tr('settings'),
      click: () => openSettings(),
    },
    {
      label: tr('checkUpdates'),
      click: () => checkForUpdatesNow(),
    },
    {
      label: tr('overlayWindow'),
      type: 'checkbox',
      checked: handle?.store.get().desktop.overlayWindow !== false,
      click: () => {
        const current = handle?.store.get().desktop.overlayWindow !== false;
        handle?.store.update({ desktop: { overlayWindow: !current } });
        applyOverlayWindow();
        buildTrayMenu();
      },
    },
    {
      label: tr('copyUrl'),
      click: () => copyOverlayUrl(),
    },
    // Soundboard UI is parked for now (SOUNDS_UI_HIDDEN).
    ...(SOUNDS_UI_HIDDEN
      ? []
      : [
          {
            label: 'Soundboard',
            click: () => openBoard(),
          } as Electron.MenuItemConstructorOptions,
        ]),
    { type: 'separator' },
    {
      label: tr('dataSource'),
      submenu: [
        { label: tr('demo'), type: 'radio', checked: cfg.source === 'mock', click: () => setSource('mock') },
        { label: tr('ts3'), type: 'radio', checked: cfg.source === 'ts3', click: () => setSource('ts3') },
        { label: tr('ts6'), type: 'radio', checked: cfg.source === 'ts6', click: () => setSource('ts6') },
      ],
    },
    {
      label: tr('reconnect'),
      click: () => handle?.restartConnector(),
    },
    { type: 'separator' },
    {
      label: tr('clickThrough'),
      type: 'checkbox',
      checked: cfg.desktop.clickThrough,
      click: () => toggleClickThrough(),
    },
    {
      label: tr('autostart'),
      type: 'checkbox',
      checked: cfg.desktop.autostart,
      click: () => toggleAutostart(),
    },
    {
      label: tr('showOverlay'),
      type: 'checkbox',
      checked: overlayVisible,
      click: () => setOverlayVisible(!overlayVisible),
    },
    { type: 'separator' },
    {
      label: tr('quit'),
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

  // A stable port keeps the overlay URL usable in OBS browser sources.
  let preferredPort = 48911;
  try {
    const raw = JSON.parse(readFileSync(join(DATA_DIR, 'config.json'), 'utf8')) as {
      desktop?: { port?: number };
    };
    if (typeof raw?.desktop?.port === 'number') preferredPort = raw.desktop.port;
  } catch {
    /* defaults apply */
  }

  handle = await startServer({
    port: 0,
    preferredPort,
    host: '127.0.0.1',
    dataDir: DATA_DIR,
    publicDir: PUBLIC_DIR,
    log,
    listDisplays: () =>
      screen.getAllDisplays().map((d) => ({
        id: String(d.id),
        label: d.label || `${d.size.width}×${d.size.height}`,
        primary: d.id === screen.getPrimaryDisplay().id,
        width: d.workArea.width,
        height: d.workArea.height,
      })),
    setOverlayClickThrough: (ignore: boolean) => {
      if (overlayWin && !overlayWin.isDestroyed()) {
        overlayWin.setIgnoreMouseEvents(!!ignore, { forward: true });
      }
    },
    checkUpdates: () => checkForUpdatesNow(),
    restartToUpdate: () => tryQuitAndInstall(),
    version: app.isPackaged && typeof app.getVersion === 'function' ? app.getVersion() : 'dev',
    pluginDll: IS_PACKAGED
      ? join(process.resourcesPath, 'plugin', 'ts3plugin_tsn_win64.dll')
      : join(ROOT, 'plugin', 'dist', 'ts3plugin_tsn_win64.dll'),
  });

  overlayVisible = handle.store.get().desktop.visible !== false;

  // First run: open the settings window so the UI is immediately visible.
  const cfg = handle.store.get();
  const firstRun = !cfg.desktop.seen;

  createTray();
  applyOverlayWindow();
  applyAutostart();
  setupAutoUpdate();
  registerSoundHotkeys();
  // One guarded listener: heavy shell work only when its own slice changed.
  // (Per-config setBounds + tray rebuilds caused visible lag/freezes.)
  let lastFitSig = '';
  let lastTraySig = '';
  handle.events.on('config', () => {
    const cfg = handle!.store.get();
    const fitSig = String(cfg.desktop.display ?? 'primary');
    if (fitSig !== lastFitSig) {
      lastFitSig = fitSig;
      fitOverlay();
    }
    const traySig = JSON.stringify([
      cfg.appearance.language,
      cfg.desktop.autostart,
      cfg.desktop.clickThrough,
      cfg.desktop.visible,
      cfg.desktop.overlayWindow,
    ]);
    if (traySig !== lastTraySig) {
      lastTraySig = traySig;
      buildTrayMenu();
    }
    applyOverlayWindow();
    registerSoundHotkeys();
  });

  if (firstRun) {
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
    try {
      globalShortcut.unregisterAll();
    } catch {
      /* ignore */
    }
    handle?.close();
  });
}
