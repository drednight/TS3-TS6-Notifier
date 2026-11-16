import { EventEmitter } from 'node:events';
import WebSocket from 'ws';
import { ChatMessage, OverlayState, ServerGroup, StatusCode, UserState, emptyState, makeUser } from '../model.js';
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
          this.clients.delete(clientId);
        } else if (existing) {
          this.clients.set(clientId, { ...existing, channelId: newChannel });
          if (clientId === this.ownClientId) this.ownChannelId = newChannel;
        } else if (oldChannel <= 0) {
          this.resync('client join without properties');
          return;
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
      case 'channelMoved':
      case 'serverEdited':
        this.resync(`event ${type}`);
        return;

      case 'clientLeft':
      case 'clientLeftView':
      case 'clientDisconnected': {
        if (foreign) return;
        const clientId = jInt(payload, 'clientId') || jInt(payload, 'id');
        if (clientId > 0 && this.clients.delete(clientId)) this.emitState();
        return;
      }

      case 'neededPermissions':
      case 'clientNeededPermissions':
        return;

      default: {
        // Chat messages (names differ between TS6 builds — extract defensively).
        if (/message|chat/i.test(type) && !/perm|status/i.test(type)) {
          this.handleChatEvent(payload);
        }
        return;
      }
    }
  }

  private handleChatEvent(payload: JObj): void {
    const inner = asObj(payload['message']) ?? asObj(payload['properties']) ?? asObj(payload['data']) ?? payload;
    const body =
      jStr(payload, 'msg') ||
      jStr(payload, 'message') ||
      jStr(payload, 'text') ||
      jStr(payload, 'content') ||
      jStr(inner, 'msg') ||
      jStr(inner, 'message') ||
      jStr(inner, 'text') ||
      jStr(inner, 'content');
    if (!body) return;
    const userId = String(jInt(payload, 'invokerId') || jInt(payload, 'senderId') || jInt(payload, 'clientId') || jInt(payload, 'clid'));
    const nickname =
      jStr(payload, 'invokerName') ||
      jStr(payload, 'senderName') ||
      jStr(inner, 'nickname') ||
      this.clients.get(Number(userId))?.nickname ||
      '?';
    const mode = jInt(payload, 'targetMode') || jInt(payload, 'targetmode');
    this.pushChat({
      userId,
      nickname,
      text: body,
      kind: mode === 1 ? 'private' : mode === 3 ? 'server' : 'channel',
    });
  }

  private pushChat(msg: Omit<ChatMessage, 'id' | 'at'>): void {
    this.chat.push({ ...msg, id: `c${++this.chatSeq}`, at: Date.now() });
    if (this.chat.length > 20) this.chat = this.chat.slice(-20);
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
    this.channels.clear();
    this.clients.clear();
    this.connectionId = jInt(conn, 'id');
    this.ownClientId = jInt(conn, 'clientId');
    this.serverName = jStr(asObj(conn['properties']), 'name') || this.serverName;

    const channelInfos = asObj(conn['channelInfos']);
    if (channelInfos) this.parseChannelInfos(channelInfos);

    for (const raw of jArr(conn, 'clientInfos')) {
      const info = asObj(raw);
      if (!info) continue;
      const id = jInt(info, 'id');
      const props = asObj(info['properties']);
      if (id <= 0 || !props) continue;
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
    const groupMap = new Map<number, ServerGroup>();
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

  const nickname = hasKey(props, 'nickname') ? jStr(props, 'nickname') : hasKey(props, 'name') ? jStr(props, 'name') : '';
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
