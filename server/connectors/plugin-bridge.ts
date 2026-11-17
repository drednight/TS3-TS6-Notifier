/**
 * TS3 plugin bridge — the Overwolf-style data path.
 *
 * A TS3 client plugin (`plugin/ts3plugin_tsn.c`, built as
 * ts3plugin_tsn_win64.dll) streams events from inside the client to this
 * TCP endpoint (127.0.0.1:48912), one JSON object per line. Events arrive
 * the moment they happen inside the client: no ClientQuery polling, no
 * query connection to drop, no response desync.
 *
 * `Ts3WithPluginConnector` prefers the plugin whenever it is alive and
 * falls back to the classic ClientQuery connector after ~15 s of plugin
 * silence, so the overlay keeps working without the plugin too.
 */
import { EventEmitter } from 'node:events';
import net from 'node:net';
import {
  ChatMessage,
  OverlayState,
  ServerGroup,
  UserState,
  diag,
  emptyState,
  makeUser,
  StatusCode,
} from '../model.js';
import type { Connector } from './wire.js';
import { Ts3Connector } from './ts3.js';

interface PluginClient {
  id: number;
  name: string;
  cid: number;
  groups: string;
}

type PluginLine = Record<string, unknown>;

function str(v: unknown): string {
  return typeof v === 'string' ? v : typeof v === 'number' ? String(v) : '';
}
function num(v: unknown): number {
  const n = typeof v === 'number' ? v : parseInt(str(v), 10);
  return Number.isFinite(n) ? n : 0;
}

/** Bridge server that speaks the plugin's line-JSON protocol. */
export class PluginBridge extends EventEmitter {
  private server: net.Server | null = null;
  private sockets = new Set<net.Socket>();
  private buffer = '';
  private lastSeen = 0;
  private idleTimer: NodeJS.Timeout | null = null;
  private wasActive = false;

  constructor(
    private port = 48912,
    /** Quiet seconds before the plugin counts as gone (the plugin heartbeats
     * every ~5s, so this is generous on purpose — swap hysteresis keeps the
     * overlay from flickering between the plugin and ClientQuery). */
    private idleAfterMs = 45000,
  ) {
    super();
  }

  active(): boolean {
    return this.wasActive && Date.now() - this.lastSeen < this.idleAfterMs;
  }

  start(): void {
    if (this.server) return;
    const server = net.createServer((socket) => {
      socket.setEncoding('utf8');
      socket.setNoDelay(true);
      this.sockets.add(socket);
      let buf = '';
      socket.on('data', (chunk: string) => {
        buf += chunk;
        if (buf.length > 1_000_000) buf = '';
        let idx: number;
        while ((idx = buf.indexOf('\n')) >= 0) {
          const line = buf.slice(0, idx).trim();
          buf = buf.slice(idx + 1);
          if (line) this.handleLine(line);
        }
      });
      socket.on('close', () => this.sockets.delete(socket));
      socket.on('error', () => this.sockets.delete(socket));
    });
    server.on('error', (err) => {
      diag('plugin', `bridge listen error: ${err.message}`);
    });
    // Loopback only — the plugin lives on the same machine.
    server.listen(this.port, '127.0.0.1', () => {
      const addr = server.address();
      if (addr && typeof addr === 'object') this.port = addr.port;
      diag('plugin', `bridge listening on 127.0.0.1:${this.port}`);
    });
    this.server = server;
    this.idleTimer = setInterval(
      () => {
        if (this.wasActive && !this.active()) {
          this.wasActive = false;
          this.emit('idle');
        }
      },
      Math.min(5000, this.idleAfterMs),
    );
  }

  stop(): void {
    if (this.idleTimer) clearInterval(this.idleTimer);
    this.idleTimer = null;
    for (const s of this.sockets) s.destroy();
    this.sockets.clear();
    this.server?.close();
    this.server = null;
    this.wasActive = false;
  }

  send(obj: unknown): void {
    const raw = JSON.stringify(obj) + '\n';
    for (const s of this.sockets) {
      try {
        s.write(raw);
      } catch {
        /* dropped — the plugin reconnects on its own */
      }
    }
  }

  private handleLine(line: string): void {
    this.lastSeen = Date.now();
    if (!this.wasActive) {
      this.wasActive = true;
      this.emit('active');
    }
    let msg: PluginLine;
    try {
      msg = JSON.parse(line) as PluginLine;
    } catch {
      diag('plugin', `bad line: ${line.slice(0, 120)}`);
      return;
    }
    // Heartbeats keep the link marked alive — never let them flood the log.
    if (str(msg['e']) === 'ping') return;
    diag('plugin', line.slice(0, 240));
    this.emit('line', msg);
  }

  getPort(): number {
    return this.port;
  }
}

/** Connector that prefers the plugin bridge and falls back to ClientQuery. */
export class Ts3WithPluginConnector extends EventEmitter implements Connector {
  private bridge: PluginBridge;
  private query: Ts3Connector | null = null;
  private usingPlugin = false;
  private stopped = false;

  constructor(
    private host: string,
    private port: number,
    private apiKey: string,
    bridgePort = 48912,
    idleAfterMs = 45000,
    /**
     * v1.4.6 «one at a time»: the ClientQuery fallback starts only after this
     * delay — if the plugin connects first, the query poller never runs at
     * all, so the two data paths are never both active.
     */
    private fallbackDelayMs = 4000,
  ) {
    super();
    this.bridge = new PluginBridge(bridgePort, idleAfterMs);
  }

  start(): void {
    this.stopped = false;
    this.bridge.start();
    this.bridge.on('line', (msg: PluginLine) => {
      if (this.usingPlugin) this.pluginState?.ingest(msg);
    });
    this.bridge.on('active', () => {
      if (this.stopped) return;
      // The plugin is the primary source — put the flaky query link to sleep.
      if (!this.usingPlugin) {
        this.usingPlugin = true;
        this.query?.stop();
        this.query = null;
        if (this.fallbackTimer) {
          clearTimeout(this.fallbackTimer);
          this.fallbackTimer = null;
        }
        this.holdUntil = 0;
        this.pluginState = new PluginState(() => this.emit('state', this.pluginState!.buildState()));
        console.log('[ts3] plugin bridge active — ClientQuery connector paused');
        // Ask for a fresh snapshot so the panel fills instantly.
        this.bridge.send({ cmd: 'snapshot' });
      }
    });
    this.bridge.on('idle', () => {
      if (this.stopped) return;
      if (this.usingPlugin) {
        this.usingPlugin = false;
        // Keep the last plugin state warm for a few seconds so the overlay
        // does not blank while the ClientQuery connector boots.
        this.holdUntil = Date.now() + 6000;
        this.queryLive = false;
        console.log('[ts3] plugin bridge idle — falling back to ClientQuery');
      }
      this.ensureQuery();
    });
    // Lazy fallback: give the plugin a few seconds to connect first so the
    // ClientQuery poller and the plugin never work simultaneously.
    this.fallbackTimer = setTimeout(() => {
      this.fallbackTimer = null;
      this.ensureQuery();
    }, this.fallbackDelayMs);
  }

  stop(): void {
    this.stopped = true;
    if (this.fallbackTimer) {
      clearTimeout(this.fallbackTimer);
      this.fallbackTimer = null;
    }
    this.bridge.removeAllListeners('line');
    this.bridge.removeAllListeners('active');
    this.bridge.removeAllListeners('idle');
    this.bridge.stop();
    this.query?.stop();
    this.query = null;
    this.pluginState = null;
    this.usingPlugin = false;
  }

  /** Testing / diagnostics hook. */
  getBridge(): PluginBridge {
    return this.bridge;
  }

  /** Testing hook: the ClientQuery fallback (null until it starts). */
  getQuery(): Ts3Connector | null {
    return this.query;
  }

  private pluginState: PluginState | null = null;
  private holdUntil = 0;
  private queryLive = false;
  private fallbackTimer: NodeJS.Timeout | null = null;

  private ensureQuery(): void {
    if (this.stopped || this.query) return;
    const q = new Ts3Connector(this.host, this.port, this.apiKey);
    q.on('state', (s: OverlayState) => {
      if (this.usingPlugin) return;
      if (s.status === 'live') this.queryLive = true;
      // Swap gap: hold the last plugin picture instead of an empty shell.
      if (!this.queryLive && this.pluginState && Date.now() < this.holdUntil) {
        this.emit('state', this.pluginState.buildState());
        return;
      }
      this.pluginState = null;
      this.emit('state', s);
    });
    this.query = q;
    q.start();
  }
}

/**
 * Minimal TS3 state store fed by plugin lines — mirrors the ClientQuery
 * connector's state shape (channel roster + global event rows).
 */
export class PluginState {
  private clients = new Map<number, UserState>();
  private chat: ChatMessage[] = [];
  private chatSeq = 0;
  private ownId = 0;
  private ownCid = 0;
  private ownChannelName = '';
  private channelNames = new Map<number, string>();
  private serverName = '';
  private groups: ServerGroup[] = [];
  private announcedOnce = false;
  private flushTimer: NodeJS.Timeout | null = null;
  private lastEmit = 0;
  private curStatus: StatusCode = 'live';
  private curDetail = 'TS3 plugin';

  constructor(private emit: () => void) {}

  ingest(msg: PluginLine): void {
    switch (str(msg['e'])) {
      case 'snapshot': {
        const prev = this.clients;
        this.serverName = str(msg['server']) || this.serverName;
        this.ownId = num(msg['own']);
        this.ownCid = num(msg['owncid']);
        this.clients = new Map();
        const list = Array.isArray(msg['clients']) ? (msg['clients'] as PluginLine[]) : [];
        const channelNames = new Map<number, string>();
        for (const raw of list) {
          const id = num(raw['id']);
          if (id <= 0) continue;
          const cid = num(raw['cid']);
          if (raw['cname']) channelNames.set(cid, str(raw['cname']));
          this.clients.set(
            id,
            makeUser(String(id), str(raw['name']), {
              channelId: cid,
              isSelf: id === this.ownId,
              serverGroups: str(raw['groups']),
            }),
          );
        }
        this.channelNames = channelNames;
        this.ownChannelName = channelNames.get(this.ownCid) ?? str(msg['owncname']) ?? '';
        // Diff-announce what the event stream missed while we were away.
        if (this.announcedOnce) {
          for (const [id, u] of this.clients) {
            if (!prev.has(id) && !u.isSelf && !u.isQuery && u.nickname) {
              this.pushRow(u.nickname, u.id, 'join', u.channelId === this.ownCid);
            }
          }
          for (const [id, u] of prev) {
            if (!this.clients.has(id) && !u.isSelf && !u.isQuery && u.nickname) {
              this.pushRow(u.nickname, u.id, 'leave', u.channelId === this.ownCid);
            }
          }
        } else {
          this.announcedOnce = true;
        }
        this.emitState();
        return;
      }

      case 'join': {
        const id = num(msg['clid']);
        const u = makeUser(String(id), str(msg['name']), {
          channelId: num(msg['cid']),
          isSelf: id === this.ownId,
          serverGroups: str(msg['groups']),
        });
        this.clients.set(id, u);
        if (msg['cname']) this.channelNames.set(u.channelId, str(msg['cname']));
        // Own join (server connect, channel return) moves the channel window.
        if (id === this.ownId) {
          this.ownCid = u.channelId;
          this.ownChannelName = str(msg['cname']) || this.channelNames.get(u.channelId) || this.ownChannelName;
        }
        if (!u.isSelf && !u.isQuery && u.nickname) {
          this.pushRow(u.nickname, u.id, 'join', u.channelId === this.ownCid);
        }
        this.emitState();
        return;
      }

      case 'leave': {
        const id = num(msg['clid']);
        const prev = this.clients.get(id);
        const nick = str(msg['name']) || prev?.nickname || '';
        const wasOurs = !!prev && prev.channelId === this.ownCid;
        this.clients.delete(id);
        if (nick && !prev?.isQuery && !prev?.isSelf && id !== this.ownId) {
          this.pushRow(nick, prev?.id ?? String(id), 'leave', wasOurs);
        }
        this.emitState();
        return;
      }

      case 'move': {
        const id = num(msg['clid']);
        const prev = this.clients.get(id);
        const to = num(msg['to']);
        const from = num(msg['from']);
        const nick = str(msg['name']) || prev?.nickname || '';
        const u = prev
          ? { ...prev, channelId: to, serverGroups: str(msg['groups']) || prev.serverGroups }
          : makeUser(String(id), nick, { channelId: to, serverGroups: str(msg['groups']), isSelf: id === this.ownId });
        this.clients.set(id, u);
        const cname = str(msg['cname']);
        if (cname) this.channelNames.set(to, cname);
        // Own move switches the "my channel" window — without this the roster
        // kept filtering against the OLD channel and the overlay went blank
        // (hide-empty hid the panel the moment the filter matched nobody).
        if (id === this.ownId) {
          this.ownCid = to;
          this.ownChannelName = cname || this.channelNames.get(to) || this.ownChannelName;
        }
        if (!u.isQuery && nick) {
          this.pushRow(nick, u.id, 'move', u.isSelf || from === this.ownCid || to === this.ownCid, cname || this.channelNames.get(to) || '');
        }
        this.emitState();
        return;
      }

      case 'talk': {
        const id = num(msg['clid']);
        const prev = this.clients.get(id);
        if (prev) {
          this.clients.set(id, { ...prev, talking: num(msg['s']) === 1 });
          this.emitState();
        }
        return;
      }

      case 'chat': {
        const kindRaw = str(msg['kind']);
        const kind: ChatMessage['kind'] =
          kindRaw === 'private' ? 'private' : kindRaw === 'server' ? 'server' : 'channel';
        this.pushChat({
          userId: num(msg['clid']) ? String(num(msg['clid'])) : str(msg['name']),
          nickname: str(msg['name']) || '?',
          text: str(msg['text']),
          kind,
        });
        return;
      }

      case 'poke': {
        this.pushChat({
          userId: num(msg['clid']) ? String(num(msg['clid'])) : str(msg['name']),
          nickname: str(msg['name']) || 'Кто-то',
          text: str(msg['text']) || str(msg['name']),
          kind: 'poke',
          event: 'poke',
          detail: str(msg['text']),
        });
        return;
      }

      case 'client': {
        // Nickname / server-group update for one client.
        const id = num(msg['clid']);
        const prev = this.clients.get(id);
        if (!prev) return;
        const groups = str(msg['groups']);
        this.clients.set(id, {
          ...prev,
          nickname: str(msg['name']) || prev.nickname,
          serverGroups: groups || prev.serverGroups,
        });
        if (groups && groups !== prev.serverGroups) {
          this.pushChat({
            userId: String(id),
            nickname: prev.nickname,
            text: 'server',
            kind: 'system',
            event: 'group',
            detail: groups.split(',').filter(Boolean).join(', ') || '—',
            own: prev.channelId === this.ownCid,
          });
        }
        this.emitState();
        return;
      }

      case 'disconnect': {
        this.clients.clear();
        this.channelNames.clear();
        this.ownCid = 0;
        this.ownChannelName = '';
        this.announcedOnce = false;
        this.emitState();
        return;
      }

      default:
        return; // hello / pong
    }
  }

  private pushRow(nick: string, uid: string, event: 'join' | 'leave' | 'move', own: boolean, detail = ''): void {
    this.pushChat({
      userId: uid,
      nickname: nick,
      text: nick,
      kind: 'system',
      event,
      detail,
      own,
    });
  }

  private pushChat(msg: Omit<ChatMessage, 'id' | 'at'>): void {
    const now = Date.now();
    // Same tail-scan dedupe as the ClientQuery connector.
    for (let i = this.chat.length - 1; i >= 0 && i >= this.chat.length - 12; i--) {
      const prev = this.chat[i]!;
      if (now - prev.at > 5000) break;
      if (prev.text === msg.text && prev.userId === msg.userId && (prev.event ?? '') === (msg.event ?? '') && (prev.detail ?? '') === (msg.detail ?? '')) {
        const rank: Record<string, number> = { private: 3, server: 2, channel: 1 };
        const kind = (rank[msg.kind] ?? 0) > (rank[prev.kind] ?? 0) ? msg.kind : prev.kind;
        const nickname = prev.nickname && prev.nickname !== '?' ? prev.nickname : msg.nickname;
        this.chat[i] = { ...prev, kind, nickname, own: prev.own ?? msg.own };
        this.emitState();
        return;
      }
    }
    this.chat.push({ ...msg, id: `p${++this.chatSeq}`, at: now });
    if (this.chat.length > 60) this.chat = this.chat.slice(-60);
    this.emitState();
  }

  buildState(): OverlayState {
    return {
      ...emptyState('ts3', this.curStatus, this.curDetail),
      serverName: this.serverName,
      channelName: this.ownChannelName || (this.channelNames.get(this.ownCid) ?? ''),
      channelId: this.ownCid > 0 ? String(this.ownCid) : '',
      selfId: this.ownId > 0 ? String(this.ownId) : null,
      users: [...this.clients.values()].filter((c) => this.ownCid > 0 && c.channelId === this.ownCid),
      groups: this.groups,
      chat: this.chat,
      updatedAt: Date.now(),
    };
  }

  private emitState(): void {
    const now = Date.now();
    const wait = Math.max(0, 100 - (now - this.lastEmit));
    if (this.flushTimer) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      this.lastEmit = Date.now();
      this.emit();
    }, wait);
  }
}
