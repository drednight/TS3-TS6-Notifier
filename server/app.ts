import { EventEmitter } from 'node:events';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, unlinkSync, writeFileSync } from 'node:fs';
import { basename, extname, join } from 'node:path';
import { WebSocketServer, WebSocket } from 'ws';
import { ConfigStore } from './config.js';
import { DIAG, OverlayState, diag, emptyState } from './model.js';
import type { Connector } from './connectors/wire.js';
import { MockConnector } from './connectors/mock.js';
import { Ts3Connector } from './connectors/ts3.js';
import { PluginBridge, Ts3WithPluginConnector } from './connectors/plugin-bridge.js';
import { Ts6Connector } from './connectors/ts6.js';
import { findTs3Icon } from './icons.js';

export interface AppOptions {
  port: number;
  host: string;
  dataDir: string;
  publicDir: string;
  log?: (msg: string) => void;
  /** Desktop only: enumerate connected displays for the overlay target setting. */
  listDisplays?: () => Array<{ id: string; label: string; primary: boolean; width?: number; height?: number }>;
  /** Desktop only: temporarily take/release mouse input on the overlay window. */
  setOverlayClickThrough?: (ignore: boolean) => void;
  /** Try this port first (stable URL for OBS browser sources); falls back to a random one. */
  preferredPort?: number;
  /** Desktop only: trigger an update check in the shell. */
  checkUpdates?: () => void;
  /** Desktop only: quit and install a downloaded update right now. */
  restartToUpdate?: () => void;
  /** App version for the diagnostics report. */
  version?: string;
  /** Path of the bundled TS3 plugin DLL (for «Установить плагин»). */
  pluginDll?: string;
}

export interface AppHandle {
  events: EventEmitter;
  store: ConfigStore;
  state(): OverlayState;
  restartConnector(): void;
  /** Push a raw message to every WebSocket client (sound triggers etc.). */
  broadcast(message: unknown): void;
  close(): void;
  port(): number;
}

const MIME: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.ico': 'image/x-icon',
  '.woff2': 'font/woff2',
};

/** Boot the HTTP + WebSocket app and connect the configured data source. */
export function startServer(opts: AppOptions): Promise<AppHandle> {
  const log = opts.log ?? console.log;
  const store = new ConfigStore(opts.dataDir);
  const events = new EventEmitter();
  let connector: Connector | null = null;
  let latestState: OverlayState = emptyState(store.get().source, 'connecting');

  let diagBridge: PluginBridge | null = null;

  function startConnector(): void {
    connector?.stop();
    connector = null;
    diagBridge?.stop();
    diagBridge = null;
    const cfg = store.get();

    const next: Connector =
      cfg.source === 'ts3'
        ? // Prefer the in-client TS3 plugin (instant events, no query drops);
          // falls back to ClientQuery automatically when the plugin is absent.
          new Ts3WithPluginConnector(cfg.ts3.host, cfg.ts3.port, cfg.ts3.apiKey)
        : cfg.source === 'ts6'
          ? new Ts6Connector(cfg.ts6.host, cfg.ts6.port, cfg.ts6.apiKey, (key) => {
              store.update({ ts6: { apiKey: key } });
            })
          : new MockConnector();

    next.on('state', (state: OverlayState) => {
      latestState = state;
      events.emit('state', state);
      broadcast({ type: 'state', state });
    });

    connector = next;
    next.start();
    // Both TeamSpeak clients may run at once — capture TS3 plugin events for
    // the diagnostics report even when TS6 is the active source.
    if (cfg.source !== 'ts3') {
      diagBridge = new PluginBridge(48912);
      diagBridge.start();
    }
    log(`[server] data source: ${cfg.source}`);
  }

  /* ---------------- ws hub ---------------- */

  const wss = new WebSocketServer({ noServer: true, maxPayload: 2 * 1024 * 1024 });

  let moveModeActive = false;
  let panelSize: { w: number; h: number } | null = null;

  function broadcast(message: unknown): void {
    const raw = JSON.stringify(message);
    for (const client of wss.clients) {
      if (client.readyState === WebSocket.OPEN) client.send(raw);
    }
  }

  /* ---------------- http ---------------- */

  function json(res: ServerResponse, code: number, body: unknown): void {
    res.writeHead(code, {
      'Content-Type': 'application/json; charset=utf-8',
      'Access-Control-Allow-Origin': '*',
    });
    res.end(JSON.stringify(body));
  }

  function readBody(req: IncomingMessage): Promise<unknown> {
    return new Promise((resolve) => {
      let data = '';
      req.on('data', (c) => {
        data += c;
        if (data.length > 1_000_000) req.destroy();
      });
      req.on('end', () => {
        try {
          resolve(data ? JSON.parse(data) : {});
        } catch {
          resolve({});
        }
      });
    });
  }

  function serveStatic(res: ServerResponse, urlPath: string, publicDir: string): void {
    // NOTE: path.normalize turns '/' into '\' on Windows, which used to break
    // the root page (404 → empty windows). Split manually instead.
    const rawPath = decodeURIComponent(urlPath.split('?')[0] ?? '/');
    const segments = rawPath.split(/[/\\]+/).filter((s) => s && s !== '.' && s !== '..');
    const rel = segments.join('/');
    const filePath = join(publicDir, rel === '' ? 'index.html' : rel);
    if (!filePath.startsWith(publicDir)) {
      res.writeHead(403).end('Forbidden');
      return;
    }
    try {
      if (statSync(filePath).isDirectory()) throw new Error('dir');
      res.writeHead(200, {
        'Content-Type': MIME[extname(filePath).toLowerCase()] ?? 'application/octet-stream',
        'Cache-Control': 'no-cache',
      });
      res.end(readFileSync(filePath));
    } catch {
      res.writeHead(404).end('Not found');
    }
  }

  const server: Server = createServer(async (req, res) => {
    const url = req.url ?? '/';
    const method = req.method ?? 'GET';

    if (method === 'OPTIONS') {
      res.writeHead(204, {
        'Access-Control-Allow-Origin': '*',
        'Access-Control-Allow-Methods': 'GET,PUT,POST,OPTIONS',
        'Access-Control-Allow-Headers': 'Content-Type',
      });
      res.end();
      return;
    }

    if (url.startsWith('/api/state') && method === 'GET') {
      json(res, 200, latestState);
      return;
    }

    if (url.startsWith('/api/config') && method === 'GET') {
      json(res, 200, store.get());
      return;
    }

    if (url.startsWith('/api/config') && method === 'PUT') {
      const patch = await readBody(req);
      const { restart } = store.update(patch);
      if (restart) startConnector();
      broadcast({ type: 'config', config: store.get() });
      events.emit('config', store.get());
      json(res, 200, { ok: true, restart, config: store.get() });
      return;
    }

    if (url.startsWith('/api/tsicon/') && method === 'GET') {
      const id = parseInt(url.slice('/api/tsicon/'.length).split('?')[0] ?? '', 10);
      const icon = Number.isFinite(id) ? findTs3Icon(id) : null;
      if (!icon) {
        res.writeHead(404).end('Not found');
        return;
      }
      res.writeHead(200, {
        'Content-Type': icon.mime,
        'Cache-Control': 'public, max-age=3600',
        'Access-Control-Allow-Origin': '*',
      });
      res.end(icon.data);
      return;
    }

    if (url.startsWith('/api/displays') && method === 'GET') {
      json(res, 200, {
        ok: true,
        displays: opts.listDisplays ? opts.listDisplays() : [],
        panelSize,
      });
      return;
    }

    if (url.startsWith('/api/reconnect') && method === 'POST') {
      startConnector();
      json(res, 200, { ok: true });
      return;
    }

    // The settings window arms overlay move mode (same signal as the tray).
    // NOTE: exact match — /move-confirm and friends are separate routes.
    if (url === '/api/overlay/move' && method === 'POST') {
      moveModeActive = !moveModeActive;
      broadcast({ type: 'move-mode', active: moveModeActive });
      json(res, 200, { ok: true, moveMode: moveModeActive });
      return;
    }

    // The overlay finished a drag: position is pending until confirmed.
    if (url === '/api/overlay/moved' && method === 'POST') {
      void readBody(req).then((body) => {
        const b = body as { marginX?: number; marginY?: number } | null;
        broadcast({ type: 'move-pending', marginX: b?.marginX, marginY: b?.marginY });
        json(res, 200, { ok: true });
      });
      return;
    }

    if (url === '/api/overlay/move-confirm' && method === 'POST') {
      moveModeActive = false;
      broadcast({ type: 'move-confirm' });
      json(res, 200, { ok: true });
      return;
    }

    if (url === '/api/overlay/move-cancel' && method === 'POST') {
      moveModeActive = false;
      broadcast({ type: 'move-cancel' });
      json(res, 200, { ok: true });
      return;
    }

    // The overlay reports its real panel size so the settings sliders can
    // bound margins by "screen minus panel" exactly.
    if (url === '/api/overlay/size' && method === 'POST') {
      void readBody(req).then((body) => {
        const b = body as { w?: number; h?: number } | null;
        if (Number.isFinite(b?.w) && Number.isFinite(b?.h)) {
          panelSize = { w: Math.round(Number(b?.w)), h: Math.round(Number(b?.h)) };
        }
        json(res, 200, { ok: true });
      });
      return;
    }

    // «Плагин TS3»: install the bundled bridge DLL into the client's plugin folder.
    if (url === '/api/plugin/install' && method === 'POST') {
      const src = opts.pluginDll ?? '';
      const dst = process.env.APPDATA
        ? join(process.env.APPDATA, 'TS3Client', 'plugins', 'ts3plugin_tsn_win64.dll')
        : '';
      try {
        if (!src || !existsSync(src)) {
          json(res, 200, { ok: false, error: 'dll-missing' });
        } else if (!dst) {
          json(res, 200, { ok: false, error: 'windows-only' });
        } else {
          mkdirSync(join(dst, '..'), { recursive: true });
          copyFileSync(src, dst);
          diag('plugin', `installed to ${dst}`);
          json(res, 200, { ok: true, path: dst });
        }
      } catch (err) {
        json(res, 200, { ok: false, error: err instanceof Error ? err.message : String(err) });
      }
      return;
    }

    if (url === '/api/plugin/status' && method === 'GET') {
      const holder = connector as unknown as { getBridge?: () => { active(): boolean } };
      const bridge = typeof holder?.getBridge === 'function' ? holder.getBridge() : null;
      json(res, 200, {
        active: !!bridge?.active(),
        dll: !!(opts.pluginDll && existsSync(opts.pluginDll)),
      });
      return;
    }

    // «Диагностика»: a copyable report of what the app caught from TS3/TS6.
    if (url === '/api/diagnostics' && method === 'GET') {
      const cfg = store.get();
      const ts3Lines = DIAG.filter((l) => l.includes('[ts3]') || l.includes('[plugin]'));
      const ts6Lines = DIAG.filter((l) => l.includes('[ts6]'));
      const otherLines = DIAG.filter((l) => !l.includes('[ts3]') && !l.includes('[plugin]') && !l.includes('[ts6]'));
      const report = [
        `TS3-TS6 Notifier diagnostics — ${new Date().toISOString()}`,
        `version: ${opts.version ?? 'dev'}`,
        `source: ${cfg.source} (ts3 ${cfg.ts3.host}:${cfg.ts3.port}, ts6 ${cfg.ts6.host}:${cfg.ts6.port})`,
        `last status: ${latestState.status} — ${latestState.detail || ''}`,
        `users on channel: ${latestState.users.length}, chat/event rows kept: ${latestState.chat.length}`,
        '',
        '--- recent chat & events ---',
        ...latestState.chat.slice(-30).map(
          (m) => `${new Date(m.at).toISOString().slice(11, 19)} ${m.kind}${m.event ? '/' + m.event : ''} ${m.nickname ? m.nickname + ': ' : ''}${m.text}${m.detail ? ' [' + m.detail + ']' : ''}`,
        ),
        '',
        '--- TS3 (ClientQuery / plugin) ---',
        ...(ts3Lines.length
          ? ts3Lines
          : ['(no data — активный источник не TS3; установите плагин TS3, чтобы видеть TS3-события и при источнике TS6)']),
        '',
        '--- TS6 (Remote Apps) ---',
        ...(ts6Lines.length ? ts6Lines : ['(no data — активный источник не TS6)']),
        ...(otherLines.length ? ['', '--- other ---', ...otherLines] : []),
      ].join('\n');
      res.writeHead(200, { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'no-store' });
      res.end(report);
      return;
    }

    // «Перезапустить сейчас»: install the downloaded update and relaunch.
    if (url === '/api/update/restart' && method === 'POST') {
      try {
        opts.restartToUpdate?.();
      } catch (err) {
        log(`[server] restart-to-update failed: ${err}`);
      }
      json(res, 200, { ok: true, applied: !!opts.restartToUpdate });
      return;
    }

    // Update check runs in the desktop shell (electron-updater).
    if (url.startsWith('/api/update/check') && method === 'POST') {
      try {
        opts.checkUpdates?.();
      } catch (err) {
        log(`[server] update check failed: ${err}`);
      }
      json(res, 200, { ok: true, applied: !!opts.checkUpdates });
      return;
    }

    // Hovering/clicking overlay widgets (the unread badge) needs real mouse
    // input even while the overlay is click-through — the desktop shell
    // provides the hook; in the plain web version this is a no-op.
    if (url.startsWith('/api/overlay/clickthrough') && method === 'POST') {
      void readBody(req).then((body) => {
        const ignore = !!(body as { ignore?: boolean })?.ignore;
        try {
          opts.setOverlayClickThrough?.(ignore);
        } catch (err) {
          log(`[server] clickthrough hook failed: ${err}`);
        }
        json(res, 200, { ok: true, applied: !!opts.setOverlayClickThrough });
      });
      return;
    }

    /* ---------------- soundboard files ---------------- */

    const soundsDir = join(opts.dataDir, 'sounds');

    if (url.startsWith('/sounds/')) {
      const name = basename(decodeURIComponent(url.split('?')[0] ?? '').slice('/sounds/'.length));
      const filePath = join(soundsDir, name);
      try {
        if (!existsSync(soundsDir) || !filePath.startsWith(soundsDir) || statSync(filePath).isDirectory()) throw new Error('bad');
        res.writeHead(200, {
          'Content-Type': MIME[extname(filePath).toLowerCase()] ?? 'application/octet-stream',
          'Cache-Control': 'no-cache',
          'Access-Control-Allow-Origin': '*',
        });
        res.end(readFileSync(filePath));
      } catch {
        res.writeHead(404).end('Not found');
      }
      return;
    }

    if (url.startsWith('/api/sounds') && method === 'GET') {
      const files = existsSync(soundsDir)
        ? readdirSync(soundsDir).map((name) => ({ file: `/sounds/${name}`, name }))
        : [];
      json(res, 200, { ok: true, files });
      return;
    }

    if (url.startsWith('/api/sounds') && method === 'POST') {
      const body = (await readBody(req)) as { name?: string; data?: string };
      const rawName = String(body.name ?? '').replace(/[^\w.\- ]/g, '').slice(0, 64) || 'sound';
      const ext = ['.wav', '.mp3', '.ogg', '.m4a', '.aac', '.opus', '.flac'].includes(extname(rawName).toLowerCase())
        ? extname(rawName).toLowerCase()
        : '.wav';
      const data = String(body.data ?? '');
      const buf = Buffer.from(data, 'base64');
      if (!buf.length || buf.length > 8_000_000) {
        json(res, 400, { ok: false, error: 'empty or too large (8 MB max)' });
        return;
      }
      mkdirSync(soundsDir, { recursive: true });
      const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
      writeFileSync(join(soundsDir, id + ext), buf);
      json(res, 200, { ok: true, file: `/sounds/${id}${ext}`, name: rawName });
      return;
    }

    if (url.startsWith('/api/sounds/') && method === 'DELETE') {
      const name = basename(decodeURIComponent(url.slice('/api/sounds/'.length).split('?')[0] ?? ''));
      const filePath = join(soundsDir, name);
      try {
        if (existsSync(filePath) && filePath.startsWith(soundsDir)) unlinkSync(filePath);
        json(res, 200, { ok: true });
      } catch (err) {
        json(res, 500, { ok: false, error: String(err) });
      }
      return;
    }

    if (url.startsWith('/api/')) {
      json(res, 404, { ok: false, error: 'unknown endpoint' });
      return;
    }

    serveStatic(res, url, opts.publicDir);
  });

  server.on('upgrade', (req, socket, head) => {
    const url = (req.url ?? '/').split('?')[0];
    if (url !== '/ws') {
      socket.destroy();
      return;
    }
    wss.handleUpgrade(req, socket, head, (ws) => {
      wss.emit('connection', ws, req);
    });
  });

  wss.on('connection', (ws) => {
    ws.send(JSON.stringify({ type: 'hello', config: store.get(), state: latestState, moveMode: moveModeActive, version: opts.version ?? 'dev' }));
  });

  return new Promise((resolve, reject) => {
    const start = () => {
      server.listen(currentPort, opts.host, () => {
        const addr = server.address();
        const actualPort = typeof addr === 'object' && addr ? addr.port : currentPort;
        log(`[server] TS3-TS6 Notifier running at http://${opts.host}:${actualPort}`);
        startConnector();
        resolve({
        events,
        store,
        state: () => latestState,
        restartConnector: startConnector,
        broadcast,
        port: () => {
          const addr = server.address();
          return typeof addr === 'object' && addr ? addr.port : opts.port;
        },
          close: () => {
            connector?.stop();
            wss.close();
            server.close();
          },
        });
      });
    };
    // A stable port makes the overlay URL copyable into OBS browser sources;
    // if it is busy we silently fall back to a random port.
    let currentPort = opts.port;
    if (opts.preferredPort) {
      currentPort = opts.preferredPort;
      server.once('error', (err) => {
        if ((err as NodeJS.ErrnoException).code === 'EADDRINUSE') {
          log(`[server] port ${opts.preferredPort} busy — falling back to a random port`);
          currentPort = 0;
          server.once('error', reject);
          start();
        } else {
          reject(err);
        }
      });
    } else {
      server.once('error', reject);
    }
    start();
  });
}
