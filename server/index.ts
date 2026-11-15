import { createServer } from 'node:http';
import { readFileSync, statSync } from 'node:fs';
import { extname, join, normalize, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { WebSocketServer, WebSocket } from 'ws';
import { ConfigStore } from './config.js';
import { OverlayState, emptyState } from './model.js';
import type { Connector } from './connectors/wire.js';
import { MockConnector } from './connectors/mock.js';
import { Ts3Connector } from './connectors/ts3.js';
import { Ts6Connector } from './connectors/ts6.js';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const PUBLIC_DIR = join(ROOT, 'public');
const PORT = Number(process.env['PORT'] || 3000);
const HOST = process.env['HOST'] || '0.0.0.0';

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

const store = new ConfigStore();
let connector: Connector | null = null;
let latestState: OverlayState = emptyState('mock', 'connecting');

/* ------------------------------------------------------- connector */

function startConnector(): void {
  connector?.stop();
  connector = null;
  const cfg = store.get();

  const next: Connector =
    cfg.source === 'ts3'
      ? new Ts3Connector(cfg.ts3.host, cfg.ts3.port, cfg.ts3.apiKey)
      : cfg.source === 'ts6'
        ? new Ts6Connector(cfg.ts6.host, cfg.ts6.port, cfg.ts6.apiKey, (key) => {
            store.update({ ts6: { apiKey: key } });
          })
        : new MockConnector();

  next.on('state', (state: OverlayState) => {
    latestState = state;
    broadcast({ type: 'state', state });
  });

  connector = next;
  next.start();
  console.log(`[server] data source: ${cfg.source}`);
}

/* ------------------------------------------------------- ws hub */

const wss = new WebSocketServer({ noServer: true });

function broadcast(message: unknown): void {
  const raw = JSON.stringify(message);
  for (const client of wss.clients) {
    if (client.readyState === WebSocket.OPEN) client.send(raw);
  }
}

/* ------------------------------------------------------- http */

function json(res: import('node:http').ServerResponse, code: number, body: unknown): void {
  const raw = JSON.stringify(body);
  res.writeHead(code, {
    'Content-Type': 'application/json; charset=utf-8',
    'Access-Control-Allow-Origin': '*',
  });
  res.end(raw);
}

function readBody(req: import('node:http').IncomingMessage): Promise<unknown> {
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

function serveStatic(res: import('node:http').ServerResponse, urlPath: string): void {
  const clean = normalize(decodeURIComponent(urlPath.split('?')[0] ?? '/')).replace(/^(\.\.[/\\])+/, '');
  const filePath = join(PUBLIC_DIR, clean === '/' ? 'index.html' : clean);
  if (!filePath.startsWith(PUBLIC_DIR)) {
    res.writeHead(403).end('Forbidden');
    return;
  }
  try {
    if (statSync(filePath).isDirectory()) throw new Error('dir');
    const body = readFileSync(filePath);
    res.writeHead(200, {
      'Content-Type': MIME[extname(filePath).toLowerCase()] ?? 'application/octet-stream',
      'Cache-Control': 'no-cache',
    });
    res.end(body);
  } catch {
    if (clean === '/' || clean === '') {
      res.writeHead(404).end('Not found');
    } else {
      res.writeHead(404).end('Not found');
    }
  }
}

const server = createServer(async (req, res) => {
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
    json(res, 200, { ok: true, restart, config: store.get() });
    return;
  }

  if (url.startsWith('/api/reconnect') && method === 'POST') {
    startConnector();
    json(res, 200, { ok: true });
    return;
  }

  if (url.startsWith('/api/')) {
    json(res, 404, { ok: false, error: 'unknown endpoint' });
    return;
  }

  serveStatic(res, url);
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
  ws.send(JSON.stringify({ type: 'hello', config: store.get(), state: latestState }));
});

server.listen(PORT, HOST, () => {
  console.log(`[server] TS3-TS6 Notifier running at http://${HOST}:${PORT}`);
  startConnector();
});
