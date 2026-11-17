import { EventEmitter } from 'node:events';
import WebSocket from 'ws';
import { ChatMessage, OverlayState, ServerGroup, StatusCode, UserState, diag, emptyState, makeUser } from '../model.js';
import {
  Connector,
  JObj,
  asObj,
  hasKey,
  jArr,
  jBool,
  jInt,
  jStr,
} from './wire.js';

interface ChannelInfo {
  id: number;
  parentId: number;
  name: string;
}

const APP_IDENTIFIER = 'dev.tsnotifier.overlay';
const APP_VERSION = '1.0.0';
const APP_NAME = 'TS3-TS6 Notifier';
const APP_DESCRIPTION = 'TeamSpeak overlay: channel roster, live speaking indicators, icons';

/**
 * TeamSpeak 5/6 data source via the Remote Apps WebSocket API
 * (ws://127.0.0.1:5899 by default).
 *
 * On first connect the TS6 client shows a permission prompt; after the user
 * accepts it, the client issues an API key which we persist for later runs.
 */
export class Ts6Connector extends EventEmitter implements Connector {
  private ws: WebSocket | null = null;
  private stopped = false;
  private reconnectTimer: NodeJS.Timeout | null = null;
  private flushTimer: NodeJS.Timeout | null = null;
  private authTimer: NodeJS.Timeout | null = null;

  private clients = new Map<number, UserState>();
  private channels = new Map<number, ChannelInfo>();
  private ownByConnection = new Map<number, number>();
  private groupInfos = new Map<number, ServerGroup>();
  private tracedProps = false;
  private chat: ChatMessage[] = [];
  private chatSeq = 0;
  private connectionId = 0;
  private preferredConnectionId = 0;
  private ownClientId = 0;
  private ownChannelId = 0;
  private serverName = '';
  private authenticated = false;
  private authDenied = false;
  private fastReconnect = false;
  private resyncRequested = false;
  private curStatus: StatusCode = 'connecting';
  private curDetail = '';
  private lastEmit = 0;

  constructor(
    private host: string,
    private port: number,
    private apiKey: string,
    private saveApiKey: (key: string) => void,
  ) {
    super();
  }

  start(): void {
    this.stopped = false;
    void this.loop();
  }

  stop(): void {
    this.stopped = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    if (this.authTimer) clearTimeout(this.authTimer);
    if (this.flushTimer) clearTimeout(this.flushTimer);
    this.reconnectTimer = this.authTimer = this.flushTimer = null;
    this.closeSocket(true);
  }

  /**
   * Terminate the socket. `detach=true` also strips listeners (shutdown path);
   * resyncs must keep them so the session promise settles via `close`.
   */
  private closeSocket(detach = false): void {
    const ws = this.ws;
    if (detach) this.ws = null;
    if (!ws) return;
    try {
      if (detach) ws.removeAllListeners();
      ws.terminate();
    } catch {
      /* ignore */
    }
  }

  /* -------------------------------------------------- connection */

  private async loop(): Promise<void> {
    while (!this.stopped) {
      let wasAuth = false;
      let sessionError = '';
      try {
        await this.connectOnce();
      } catch (err) {
        if (this.stopped) return;
        sessionError = err instanceof Error ? err.message : String(err);
      } finally {
        wasAuth = this.authenticated;
        this.authenticated = false;
        this.closeSocket(true);
      }
      if (this.stopped) return;

      if (!wasAuth && this.authDenied) {
        this.publish('waiting-auth', 'permission denied — allow the app in TS6 → Settings → Remote Apps');
      } else if (sessionError) {
        this.publish('offline', `TeamSpeak 6 Remote Apps unreachable (${sessionError})`);
      } else {
        this.publish('connecting', '');
      }

      const delay = this.fastReconnect ? 250 : wasAuth ? 1200 : 3000;
      this.fastReconnect = false;
      await new Promise((r) => {
        this.reconnectTimer = setTimeout(r, delay);
      });
    }
  }

  /** One connection session: resolves when the socket closes for any reason. */
  private connectOnce(): Promise<void> {
    return new Promise((resolve, reject) => {
      this.resyncRequested = false;
      this.connectionId = 0;
      this.ownClientId = 0;
      this.ownChannelId = 0;
      this.authDenied = false;
      this.publish('connecting', '');

      const ws = new WebSocket(`ws://${this.host}:${this.port}/`, {
        handshakeTimeout: 4000,
      });
      this.ws = ws;
      let opened = false;

      ws.on('error', (err) => {
        if (!opened) reject(err instanceof Error ? err : new Error(String(err)));
      });

      // Session end (dropped connection, resync, TS6 shutdown) — let the loop retry.
      ws.on('close', () => resolve());

      ws.on('open', () => {
        opened = true;
        this.sendAuth();
        // First run: the user must accept the permission prompt in the TS6 client.
        if (!this.apiKey) {
          this.authTimer = setTimeout(() => {
            if (!this.authenticated && !this.stopped) {
              this.publish('waiting-auth', 'confirm the permission request in TeamSpeak 6 → Settings → Remote Apps');
            }
          }, 3000);
        }
      });

      ws.on('message', (data) => {
        try {
          this.handleMessage(data.toString());
        } catch (err) {
          console.error('[ts6] message parse error:', err);
        }
      });
    });
  }

  private sendAuth(): void {
    this.ws?.send(
      JSON.stringify({
        type: 'auth',
        payload: {
          identifier: APP_IDENTIFIER,
          version: APP_VERSION,
          name: APP_NAME,
          description: APP_DESCRIPTION,
          content: { apiKey: this.apiKey },
        },
      }),
    );
  }

  /* -------------------------------------------------- messages */

  private handleMessage(raw: string): void {
    const root = asObj(JSON.parse(raw));
    const type = jStr(root, 'type');
    if (!type) return;
    const payload = asObj(root?.['payload']);
    if (!payload) return;

    // TS6 wraps some events: {"type":"proto.ServerEvent","payload":{"connectionId":2,"talkStatusChanged":{...}}}
    if (type.startsWith('proto.')) {
      const connId = jInt(payload, 'connectionId');
      for (const [name, value] of Object.entries(payload)) {
        const sub = asObj(value);
        if (!sub || name === 'connectionId') continue;
        this.dispatch(name, { ...sub, connectionId: connId || jInt(sub, 'connectionId') });
      }
      return;
    }

    this.dispatch(type, payload);
  }

  private dispatch(type: string, payload: JObj): void {
    const connId = jInt(payload, 'connectionId');
    const foreign = connId !== 0 && this.connectionId !== 0 && connId !== this.connectionId;

    // Diagnostics: capture proto messages (talking updates sampled 1-in-10).
    if (/talk|updated/i.test(type)) {
      this.noisyDiagSkipped += 1;
      if (this.noisyDiagSkipped % 10 === 1) diag('ts6', `${type} ${JSON.stringify(payload).slice(0, 200)}`);
    } else {
      diag('ts6', `${type} ${JSON.stringify(payload).slice(0, 200)}`);
    }

    switch (type) {
      case 'auth':
        this.handleAuth(payload);
        return;

      case 'channels': {
        if (foreign) return;
        this.channels.clear();
        const info = asObj(payload['info']);
        if (info) this.parseChannelInfos(info);
        this.emitState();
        return;
      }

      case 'channelCreated':
      case 'channelEdited':
      case 'channelPropertiesUpdated': {
        if (foreign) return;
        const id = jInt(payload, 'channelId') || jInt(payload, 'id');
        const props = asObj(payload['properties']);
        if (id > 0 && props) {
          const prev = this.channels.get(id);
          this.channels.set(id, {
            id,
            parentId: jInt(payload, 'parentId') || prev?.parentId || 0,
            name: jStr(props, 'name') || prev?.name || '',
          });
          this.emitState();
        }
        return;
      }

      case 'channelDeleted': {
        const id = jInt(payload, 'channelId');
        if (id > 0 && this.channels.delete(id)) this.emitState();
        return;
      }

      case 'clientMoved': {
        // A move on another server tab can mean our own client became active there.
        const clientId = jInt(payload, 'clientId');
        if (foreign) {
          if (clientId && this.ownClientIdFor(connId) === clientId) {
            this.noteActiveConnection(connId, 'self moved');
          }
          return;
        }
        const newChannel = jInt(payload, 'newChannelId') || jInt(payload, 'channelToId');
        const oldChannel = jInt(payload, 'oldChannelId') || jInt(payload, 'channelFromId');
        if (!clientId) return;

        const existing = this.clients.get(clientId);
        if (newChannel <= 0) {
          if (existing) this.pushFeed(existing, 'leave', '');
          this.clients.delete(clientId);
        } else if (existing) {
          // v1.4.1: every move on the server is news («весь сервер» scope);
          // the own flag marks touches of our channel for the scope selectors.
          if (!existing.isQuery) {
            this.pushFeed(
              existing,
              'move',
              this.channels.get(newChannel)?.name ?? '',
              newChannel === this.ownChannelId || existing.channelId === this.ownChannelId,
            );
          }
          this.clients.set(clientId, { ...existing, channelId: newChannel });
          if (clientId === this.ownClientId) this.ownChannelId = newChannel;
        } else if (oldChannel <= 0) {
          this.resync('client join without properties');
          return;
        } else {
          // Not in our map yet (no nickname on the wire) — remember the old
          // channel and resync: the snapshot diff announces the move with the
          // proper name once the resync lands.
          this.clients.set(clientId, makeUser(String(clientId), '', { channelId: oldChannel }));
          this.resync('client move of unknown client');
        }
        this.emitState();
        return;
      }

      case 'clientPropertiesUpdated':
      case 'clientUpdated': {
        if (foreign) return;
        const clientId = jInt(payload, 'clientId');
        const props = asObj(payload['properties']);
        if (!clientId || !props) return;
        const existing =
          this.clients.get(clientId) ??
          ({
            ...makeUser(String(clientId), ''),
            channelId: 0,
            isSelf: clientId === this.ownClientId,
          } satisfies UserState);
        this.clients.set(clientId, applyProperties(existing, props));
        this.emitState();
        return;
      }

      case 'talkStatusChanged': {
        const clientId = jInt(payload, 'clientId');
        const status = jInt(payload, 'status');
        // status: 0 = silent, 1 = talking, ≥2 or whisper/targetMode flags = whisper.
        const talking = status >= 1;
        const whisper =
          talking &&
          (status >= 2 ||
            jBool(payload, 'whisper') ||
            ('targetMode' in payload && jInt(payload, 'targetMode') !== 0));
        if (foreign) {
          if (talking && clientId && this.ownClientIdFor(connId) === clientId) {
            this.noteActiveConnection(connId, 'self talk');
          }
          return;
        }
        const client = this.clients.get(clientId);
        if (client && (client.talking !== talking || client.whispering !== whisper)) {
          this.clients.set(clientId, { ...client, talking, whispering: whisper });
          this.emitState();
        }
        return;
      }

      case 'clientSelfPropertyUpdated': {
        const flag = jStr(payload, 'flag');
        const value = jBool(payload, 'newValue');
        if (foreign) {
          // inputHardware=true only fires on the server tab that captured the mic.
          if (flag.toLowerCase() === 'inputhardware' && value) {
            this.noteActiveConnection(connId, 'inputHardware');
          }
          return;
        }
        const me = this.clients.get(this.ownClientId);
        if (!me) return;
        const patch: Partial<UserState> = {};
        switch (flag) {
          case 'flagTalking':
            patch.talking = value;
            break;
          case 'inputMuted':
          case 'isMuted':
          case 'inputDeactivated':
            patch.inputMuted = value;
            break;
          case 'outputMuted':
          case 'outputOnlyMuted':
            patch.outputMuted = value;
            break;
          case 'away':
            patch.away = value;
            break;
          default:
            return;
        }
        this.clients.set(this.ownClientId, { ...me, ...patch });
        this.emitState();
        return;
      }

      case 'connectStatusChanged': {
        const status = jInt(payload, 'status');
        if (status === 0) {
          if (connId === this.connectionId || this.connectionId === 0) {
            this.clients.clear();
            this.channels.clear();
            this.ownClientId = 0;
            this.ownChannelId = 0;
            this.connectionId = 0;
            this.publish('offline', 'not connected to a TeamSpeak server');
          }
          return;
        }
        if (status === 4) {
          const empty = this.clients.size === 0 || this.ownClientId === 0;
          if (this.connectionId === 0 || empty) {
            this.preferredConnectionId = connId || this.preferredConnectionId;
            this.fastReconnect = true;
            this.resync('connectStatus=4');
          }
        }
        return;
      }

      case 'serverPropertiesUpdated': {
        if (foreign) return;
        const props = asObj(payload['properties']);
        const name = jStr(props, 'name');
        if (name) {
          this.serverName = name;
          this.emitState();
        }
        return;
      }

      case 'clientJoined':
      case 'clientConnected':
      case 'clientEnteredView':
      case 'clientOnline': {
        if (!foreign) {
          const clientId = jInt(payload, 'clientId') || jInt(payload, 'id');
          const props = asObj(payload['properties']) ?? payload;
          const nick = jStr(props, 'nickname') || jStr(props, 'name') || '';
          const channelId = jInt(payload, 'channelId') || jInt(payload, 'newChannelId') || jInt(props, 'channelId');
          if (clientId > 0 && nick && clientId !== this.ownClientId) {
            const inOur = channelId === this.ownChannelId;
            this.pushChat({
              userId: String(clientId),
              nickname: nick,
              text: nick,
              kind: 'system',
              event: 'join',
              detail: inOur ? (this.channels.get(channelId)?.name ?? '') : '',
              own: inOur,
            });
          }
        }
        this.resync(`event ${type}`);
        return;
      }

      case 'clientMoved':
      case 'clientChannelChanged':
      case 'clientSwitchedChannel': {
        if (foreign) return;
        const clientId = jInt(payload, 'clientId') || jInt(payload, 'id');
        const to =
          jInt(payload, 'newChannelId') || jInt(payload, 'toChannelId') ||
          jInt(payload, 'channelId') || jInt(asObj(payload['properties']) ?? {}, 'channelId');
        const user = this.clients.get(clientId);
        if (user && clientId === this.ownClientId) {
          this.ownChannelId = to || this.ownChannelId;
        } else if (user) {
          // v1.3.1: every move on the server, not just own-channel touches.
          this.pushFeed(user, 'move', this.channels.get(to)?.name ?? '', to === this.ownChannelId || user.channelId === this.ownChannelId);
        }
        this.resync(`event ${type}`);
        return;
      }

      case 'channelMoved':
      case 'serverEdited':
        this.resync(`event ${type}`);
        return;

      case 'clientLeft':
      case 'clientLeftView':
      case 'clientDisconnected': {
        if (foreign) return;
        const clientId = jInt(payload, 'clientId') || jInt(payload, 'id');
        const existing = this.clients.get(clientId);
        if (existing) {
          if (clientId !== this.ownClientId) {
            this.pushFeed(existing, 'leave', existing.channelId === this.ownChannelId ? (this.channels.get(existing.channelId)?.name ?? '') : '', existing.channelId === this.ownChannelId);
          }
          this.clients.delete(clientId);
          this.emitState();
        }
        return;
      }

      case 'neededPermissions':
      case 'clientNeededPermissions':
        return;

      default: {
        // Server-group tables (name/icon per group) — collect best-effort so
        // class icons can resolve in TS6 too, if a build exposes them.
        if (/group/i.test(type)) {
          this.ingestGroupInfos(payload);
          this.ingestGroupInfos(asObj(payload['properties']) ?? {});
          this.emitState();
          return;
        }
        // Chat messages (names differ between TS6 builds — extract defensively).
        if (/message|chat/i.test(type) && !/perm|status/i.test(type)) {
          this.handleChatEvent(payload);
        }
        return;
      }
    }
  }

  private handleChatEvent(payload: JObj): void {
    // The event shape differs between TS6 builds — walk every nested object
    // and search known field names anywhere. Generic keys (id/name) are only
    // trusted inside sender/author objects: a message's own `id` must never
    // be mistaken for the sender's client id.
    const pools: { obj: JObj; senderish: boolean }[] = [{ obj: payload, senderish: false }];
    const walk = (obj: JObj, senderish: boolean, depth: number): void => {
      if (depth > 3) return;
      for (const [key, value] of Object.entries(obj)) {
        if (!value || typeof value !== 'object') continue;
        const child = asObj(value as JObj);
        if (!child || pools.some((x) => x.obj === child)) continue;
        const childSenderish = senderish || /sender|author|from|invoker/i.test(key);
        pools.push({ obj: child, senderish: childSenderish });
        walk(child, childSenderish, depth + 1);
      }
    };
    walk(payload, false, 0);
    const pickStr = (keys: string[], senderOnly = false): string => {
      for (const k of keys) {
        for (const p of pools) {
          if (senderOnly && !p.senderish) continue;
          const v = p.obj[k];
          if (typeof v === 'string' && v.trim()) return v;
        }
      }
      return '';
    };
    const pickInt = (keys: string[], senderOnly = false): number => {
      for (const k of keys) {
        for (const p of pools) {
          if (senderOnly && !p.senderish) continue;
          const v = p.obj[k];
          const n = typeof v === 'number' ? v : typeof v === 'string' ? parseInt(v, 10) : NaN;
          if (Number.isFinite(n) && n > 0) return n;
        }
      }
      return 0;
    };
    const body = pickStr(['msg', 'message', 'text', 'content', 'body', 'richtext']);
    if (!body) return;
    // sender id: unambiguous keys everywhere first; generic `id` only in sender objects
    const uid =
      pickInt(['invokerId', 'invokerid', 'senderId', 'senderid', 'clientId', 'clid', 'userId', 'userid']) ||
      pickInt(['id', 'from'], true);
    const userId = String(uid);
    // sender name: same strategy
    const nickname =
      pickStr([
        'invokerName',
        'invokername',
        'senderName',
        'sendername',
        'displayName',
        'displayname',
        'nickname',
        'userName',
        'username',
        'sender',
        'author',
        'from',
      ]) ||
      pickStr(['name', 'nick'], true) ||
      this.clients.get(uid)?.nickname ||
      '';
    const mode = pickInt(['targetMode', 'targetmode']);
    this.pushChat({
      userId,
      nickname,
      text: body,
      kind: mode === 1 ? 'private' : mode === 3 ? 'server' : 'channel',
    });
  }

  /** Join/leave/move feed row built from a known user. */
  private pushFeed(user: UserState, event: 'join' | 'leave' | 'move', detail: string, own = false): void {
    if (user.isQuery || !user.nickname) return;
    this.pushChat({ userId: user.id, nickname: user.nickname, text: user.nickname, kind: 'system', event, detail, own });
  }

  private pushChat(msg: Omit<ChatMessage, 'id' | 'at'>): void {
    const now = Date.now();
    // Duplicates arrive twice (own-echo loses the target mode, resync diffs
    // race the original event) — scan the recent tail, not just the last row.
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
    this.chat.push({ ...msg, id: `c${++this.chatSeq}`, at: now });
    if (this.chat.length > 60) this.chat = this.chat.slice(-60);
    this.emitState();
  }

  private handleAuth(payload: JObj): void {
    if (this.authTimer) {
      clearTimeout(this.authTimer);
      this.authTimer = null;
    }

    const status = asObj(payload['status']);
    const code = jInt(status, 'code');
    if (code !== 0) {
      this.authDenied = true;
      this.publish('waiting-auth', `authorization rejected (${jStr(status, 'message') || code})`);
      return;
    }
    this.authDenied = false;

    let apiKey = jStr(payload, 'apiKey');
    if (!apiKey) apiKey = jStr(asObj(payload['content']), 'apiKey');
    if (!apiKey) apiKey = jStr(asObj(payload['auth']), 'apiKey');
    if (apiKey && apiKey !== this.apiKey) {
      this.apiKey = apiKey;
      this.saveApiKey(apiKey);
      console.log('[ts6] Remote Apps API key received and saved');
    }

    const connections = jArr(payload, 'connections');
    if (connections.length === 0) {
      this.resync('auth without connections');
      return;
    }

    this.authenticated = true;
    const current = jInt(payload, 'currentConnectionId');
    if (current > 0) this.preferredConnectionId = current;

    const parsed = connections.map(asObj).filter((c): c is JObj => !!c);
    this.ownByConnection.clear();
    for (const conn of parsed) {
      const id = jInt(conn, 'id');
      const own = jInt(conn, 'clientId');
      if (id > 0 && own > 0) this.ownByConnection.set(id, own);
    }
    const chosen =
      parsed.find((c) => jInt(c, 'id') === this.preferredConnectionId) ??
      parsed.find((c) => jArr(c, 'clientInfos').length > 0) ??
      parsed[0]!;
    this.parseConnection(chosen);
    this.publish('live', '');
  }

  private parseConnection(conn: JObj): void {
    const prevClients = new Map(this.clients);
    this.channels.clear();
    this.clients.clear();
    this.connectionId = jInt(conn, 'id');
    this.ownClientId = jInt(conn, 'clientId');
    this.serverName = jStr(asObj(conn['properties']), 'name') || this.serverName;

    const channelInfos = asObj(conn['channelInfos']);
    if (channelInfos) this.parseChannelInfos(channelInfos);
    this.ingestGroupInfos(conn);
    // (diff-announced after the client list is built — see below)

    for (const raw of jArr(conn, 'clientInfos')) {
      const info = asObj(raw);
      if (!info) continue;
      const id = jInt(info, 'id');
      const props = asObj(info['properties']);
      if (id <= 0 || !props) continue;
      if (!this.tracedProps) {
        this.tracedProps = true;
        console.log(`[ts6] client property keys: ${Object.keys(props).join(', ')}`);
        for (const [k, v] of Object.entries(conn)) {
          if (/group/i.test(k)) console.log(`[ts6] connection group field "${k}": ${JSON.stringify(v)?.slice(0, 400)}`);
        }
      }
      this.ingestGroupInfos(props);
      const client = applyProperties(
        {
          ...makeUser(String(id), ''),
          channelId: jInt(info, 'channelId'),
          isSelf: id === this.ownClientId,
        },
        props,
      );
      this.clients.set(id, client);
    }

    const me = this.clients.get(this.ownClientId);
    this.ownChannelId = me?.channelId ?? 0;

    // Diff against the previous snapshot: join/leave events can be missed
    // (e.g. a join payload without a nickname) — announce what changed.
    if (this.announcedOnce) {
      for (const [id, u] of this.clients) {
        const prev = prevClients.get(id);
        if (!prev) {
          if (!u.isQuery && !u.isSelf && u.nickname) {
            this.pushFeed(u, 'join', u.channelId === this.ownChannelId ? (this.channels.get(u.channelId)?.name ?? '') : '', u.channelId === this.ownChannelId);
          }
          continue;
        }
        // Moves the event stream missed land here with the real nickname.
        if (!u.isQuery && u.nickname && prev.channelId > 0 && prev.channelId !== u.channelId) {
          this.pushFeed(
            u,
            'move',
            this.channels.get(u.channelId)?.name ?? '',
            u.channelId === this.ownChannelId || prev.channelId === this.ownChannelId,
          );
        }
      }
      for (const [id, u] of prevClients) {
        if (this.clients.has(id) || u.isQuery || u.isSelf || !u.nickname) continue;
        this.pushFeed(u, 'leave', u.channelId === this.ownChannelId ? (this.channels.get(u.channelId)?.name ?? '') : '', u.channelId === this.ownChannelId);
      }
    } else {
      this.announcedOnce = true;
    }
  }

  /**
   * Collect group id/name/icon info when a TS6 build provides it
   * (serverGroups as objects, serverGroupInfos arrays etc.). Best-effort —
   * most builds expose only an id list per client.
   */
  private announcedOnce = false;
  private noisyDiagSkipped = 0;

  private ingestGroupInfos(source: JObj): void {
    for (const [key, value] of Object.entries(source)) {
      if (!/group/i.test(key) || !Array.isArray(value)) continue;
      for (const raw of value) {
        if (typeof raw === 'number' || typeof raw === 'string') {
          const id = parseInt(String(raw), 10);
          if (Number.isFinite(id) && id > 0 && !this.groupInfos.has(id)) {
            this.groupInfos.set(id, { id, name: '', iconId: 0 });
          }
          continue;
        }
        const obj = asObj(raw);
        if (!obj) continue;
        const id = jInt(obj, 'id') || jInt(obj, 'sgid') || jInt(obj, 'groupId');
        if (id <= 0) continue;
        const prev = this.groupInfos.get(id);
        this.groupInfos.set(id, {
          id,
          name: jStr(obj, 'name') || prev?.name || '',
          iconId: jInt(obj, 'iconId') || jInt(obj, 'iconid') || prev?.iconId || 0,
        });
      }
    }
  }

  private parseChannelInfos(info: JObj): void {
    for (const raw of jArr(info, 'rootChannels')) {
      this.addChannel(asObj(raw), 0);
    }
    const subs = asObj(info['subChannels']);
    if (subs) {
      for (const [parentId, list] of Object.entries(subs)) {
        if (!Array.isArray(list)) continue;
        for (const raw of list) {
          this.addChannel(asObj(raw), parseInt(parentId, 10) || 0);
        }
      }
    }
  }

  private addChannel(ch: JObj | null, fallbackParent: number): void {
    if (!ch) return;
    const id = jInt(ch, 'id');
    const props = asObj(ch['properties']);
    if (id <= 0 || !props) return;
    this.channels.set(id, {
      id,
      parentId: jInt(ch, 'parentId') || fallbackParent,
      name: jStr(props, 'name'),
    });
  }

  private ownClientIdFor(connId: number): number {
    return this.ownByConnection.get(connId) ?? 0;
  }

  /** The TS6 client tracks one active server tab; follow it by re-authenticating. */
  private noteActiveConnection(connId: number, reason: string): void {
    if (connId <= 0 || connId === this.connectionId) return;
    this.preferredConnectionId = connId;
    this.fastReconnect = true;
    this.resync(`${reason}: switch to connection ${connId}`);
  }

  /** Kill the session so `loop()` reconnects and re-auths (TS6 ignores re-auth on a live socket). */
  private resync(reason: string): void {
    if (this.resyncRequested) return;
    this.resyncRequested = true;
    console.log('[ts6] resync:', reason);
    this.closeSocket(false);
  }

  /* -------------------------------------------------- publish */

  private buildState(): OverlayState {
    const groupMap = new Map<number, ServerGroup>(this.groupInfos);
    for (const c of this.clients.values()) {
      for (const id of String(c.serverGroups ?? '')
        .split(',')
        .map((s) => parseInt(s.trim(), 10))
        .filter((n) => Number.isFinite(n) && n > 0)) {
        if (!groupMap.has(id)) groupMap.set(id, { id, name: '', iconId: 0 });
      }
    }
    return {
      ...emptyState('ts6', this.curStatus, this.curDetail),
      serverName: this.serverName,
      channelName: this.channels.get(this.ownChannelId)?.name ?? '',
      channelId: this.ownChannelId > 0 ? String(this.ownChannelId) : '',
      selfId: this.ownClientId > 0 ? String(this.ownClientId) : null,
      users: [...this.clients.values()].filter(
        (c) => this.ownChannelId > 0 && c.channelId === this.ownChannelId,
      ),
      groups: [...groupMap.values()].sort((a, b) => a.id - b.id),
      chat: this.chat,
      updatedAt: Date.now(),
    };
  }

  private publish(status: StatusCode, detail: string): void {
    this.curStatus = status;
    this.curDetail = detail;
    this.emitState();
  }

  private emitState(): void {
    const now = Date.now();
    const wait = Math.max(0, 100 - (now - this.lastEmit));
    if (this.flushTimer) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      this.lastEmit = Date.now();
      this.emit('state', this.buildState());
    }, wait);
  }
}

/** Merge partial TS6 client properties into a client record. */
function applyProperties(client: UserState, props: JObj): UserState {
  const patch: Partial<UserState> = {};

  const nickname =
    jStr(props, 'nickname') || jStr(props, 'displayName') || jStr(props, 'displayname') || jStr(props, 'userName') || jStr(props, 'name');
  if (nickname) patch.nickname = nickname;

  if (hasKey(props, 'inputMuted') || hasKey(props, 'inputDeactivated') || hasKey(props, 'isMuted')) {
    patch.inputMuted = jBool(props, 'inputMuted') || jBool(props, 'inputDeactivated') || jBool(props, 'isMuted');
  }
  if (hasKey(props, 'outputMuted') || hasKey(props, 'outputOnlyMuted')) {
    patch.outputMuted = jBool(props, 'outputMuted') || jBool(props, 'outputOnlyMuted');
  }
  if (hasKey(props, 'flagTalking')) patch.talking = jBool(props, 'flagTalking');
  if (hasKey(props, 'flagTalkingWithWhisper') || hasKey(props, 'isWhispering') || hasKey(props, 'whisper')) {
    const whispering = jBool(props, 'flagTalkingWithWhisper') || jBool(props, 'isWhispering') || jBool(props, 'whisper');
    patch.whispering = whispering;
    if (whispering) patch.talking = true;
  }
  if (hasKey(props, 'away')) patch.away = jBool(props, 'away');
  if (hasKey(props, 'awayMessage')) patch.awayMessage = jStr(props, 'awayMessage');
  if (hasKey(props, 'isPrioritySpeaker')) patch.prioritySpeaker = jBool(props, 'isPrioritySpeaker');
  if (hasKey(props, 'isChannelCommander')) patch.channelCommander = jBool(props, 'isChannelCommander');
  if (hasKey(props, 'flagAvatar')) patch.avatarHash = jStr(props, 'flagAvatar');
  if (hasKey(props, 'serverGroups')) patch.serverGroups = jStr(props, 'serverGroups');

  const type = props['type'];
  if (type !== undefined) {
    patch.isQuery =
      (typeof type === 'string' && type.toUpperCase().includes('QUERY')) || (typeof type === 'number' && type === 1);
  }

  return { ...client, ...patch };
}
