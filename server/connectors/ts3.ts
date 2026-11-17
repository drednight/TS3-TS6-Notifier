import { EventEmitter } from 'node:events';
import net from 'node:net';
import { readFileSync } from 'node:fs';
import { homedir, platform } from 'node:os';
import { join } from 'node:path';
import { ChatMessage, OverlayState, ServerGroup, UserState, diag, emptyState, makeUser, StatusCode } from '../model.js';
import {
  Connector,
  Ts3Fields,
  fBool,
  fInt,
  fStr,
  ts3Escape,
  ts3ParseResponse,
  ts3Unescape,
} from './wire.js';

/** TS3 keeps the ClientQuery key in plain text — pick it up automatically. */
function readTs3ApiKey(): string {
  const candidates =
    platform() === 'win32'
      ? [join(process.env['APPDATA'] ?? join(homedir(), 'AppData', 'Roaming'), 'TS3Client', 'clientquery.ini')]
      : [join(homedir(), '.ts3client', 'clientquery.ini'), join(homedir(), 'snap', 'teamspeak3', 'current', '.ts3client', 'clientquery.ini')];
  for (const path of candidates) {
    try {
      const text = readFileSync(path, 'utf8');
      const m = /^\s*api_key\s*=\s*(.+?)\s*$/im.exec(text);
      if (m?.[1]) return m[1].replace(/^["']|["']$/g, '').trim();
    } catch {
      /* try next */
    }
  }
  return '';
}

interface PendingCommand {
  resolve: (raw: string) => void;
  lines: string[];
}

interface ChannelInfo {
  id: number;
  parentId: number;
  name: string;
}

/**
 * TeamSpeak 3 data source via the built-in ClientQuery plugin
 * (TCP telnet, default 127.0.0.1:25639).
 *
 * Enable: TS3 client → Settings → Plugins → Client Query → Settings…
 * and copy the API key into the overlay connection settings.
 */
export class Ts3Connector extends EventEmitter implements Connector {
  private socket: net.Socket | null = null;
  private buffer = '';
  private pending: PendingCommand[] = [];
  private stopped = false;
  private reconnectTimer: NodeJS.Timeout | null = null;
  private disconnectWaiters: Array<() => void> = [];
  private refreshFails = 0;
  private refreshTimer: NodeJS.Timeout | null = null;
  private periodicTimer: NodeJS.Timeout | null = null;
  private flushTimer: NodeJS.Timeout | null = null;
  private authFailed = false;
  private greetingSeen = false;
  private traceCount = 0;

  private clients = new Map<number, UserState>();
  private groupMem = new Map<number, { sg: string; cg: string }>();
  private announcedOnce = false;
  private channels = new Map<number, ChannelInfo>();
  private groups: ServerGroup[] = [];
  private chat: ChatMessage[] = [];
  private chatSeq = 0;
  private ownClidByHandler = new Map<number, number>();
  private ownClid = 0;
  private ownCid = 0;
  private serverName = '';
  private activeSchandlerId = 1;
  private curStatus: StatusCode = 'connecting';
  private curDetail = '';

  private lastEmit = 0;

  constructor(
    private host: string,
    private port: number,
    private apiKey: string,
  ) {
    super();
  }

  start(): void {
    this.stopped = false;
    void this.connectLoop();
  }

  stop(): void {
    this.stopped = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    if (this.flushTimer) clearTimeout(this.flushTimer);
    this.reconnectTimer = this.refreshTimer = this.flushTimer = null;
    this.signalDisconnect();
  }

  /* -------------------------------------------------- connection */

  private async connectLoop(): Promise<void> {
    let attempt = 0;
    while (!this.stopped) {
      try {
        await this.connectOnce();
        // The link is up — stay here until it ACTUALLY dies. Re-opening the
        // socket on a timer (the old behaviour) made the status flap between
        // "connecting" and "connected" every few seconds.
        attempt = 0;
        await this.waitDisconnected();
      } catch (err) {
        if (this.stopped) return;
        const msg = err instanceof Error ? err.message : String(err);
        this.publish(this.authFailed ? 'waiting-auth' : 'offline', msg);
      }
      if (this.stopped) return;
      attempt += 1;
      // Backoff: 3s, 6s, 12s, 24s, 30s cap — quick first retry, no hammering.
      const delay = this.authFailed ? 15000 : Math.min(30000, 3000 * 2 ** Math.min(attempt - 1, 4));
      await new Promise((r) => {
        this.reconnectTimer = setTimeout(r, delay);
      });
    }
  }

  /** Mark the current connection dead and wake disconnect waiters. */
  private signalDisconnect(socket?: net.Socket): void {
    if (socket && this.socket && this.socket !== socket) return;
    if (this.socket) {
      try {
        this.socket.destroy();
      } catch {
        /* already gone */
      }
      this.socket = null;
    }
    this.pending = [];
    this.buffer = '';
    if (this.periodicTimer) {
      clearInterval(this.periodicTimer);
      this.periodicTimer = null;
    }
    const waiters = this.disconnectWaiters.splice(0);
    for (const wake of waiters) wake();
  }

  private waitDisconnected(): Promise<void> {
    return new Promise((resolve) => {
      if (!this.socket) resolve();
      else this.disconnectWaiters.push(resolve);
    });
  }

  private connectOnce(): Promise<void> {
    return new Promise((resolve, reject) => {
      this.authFailed = false;
      this.greetingSeen = false;
      this.publish('connecting', '');
      const socket = net.createConnection({ host: this.host, port: this.port });
      this.socket = socket;
      socket.setEncoding('utf8');
      socket.setNoDelay(true);

      let settled = false;
      const fail = (err: Error) => {
        if (settled) return;
        settled = true;
        clearTimeout(connectTimer);
        socket.destroy();
        if (this.socket === socket) this.socket = null;
        this.pending = [];
        reject(err);
      };

      // Connect timeout only — the ClientQuery socket can be silent for long
      // stretches, so an inactivity timeout would kill a healthy connection.
      const connectTimer = setTimeout(() => fail(new Error('ClientQuery connect timeout')), 5000);

      socket.on('error', (err) => {
        if (!settled) fail(err instanceof Error ? err : new Error(String(err)));
        else this.signalDisconnect(socket);
      });
      socket.on('close', () => {
        if (!settled) fail(new Error('ClientQuery connection closed'));
        else this.signalDisconnect(socket);
      });
      socket.on('data', (chunk: string) => {
        this.buffer += chunk;
        // A line without terminators must never grow the buffer unbounded.
        if (this.buffer.length > 1_000_000) this.buffer = '';
        let idx: number;
        while ((idx = this.buffer.indexOf('\n')) >= 0) {
          const line = this.buffer.slice(0, idx).replace(/\r$/, '');
          this.buffer = this.buffer.slice(idx + 1);
          this.handleLine(line);
        }
      });

      socket.on('connect', () => {
        clearTimeout(connectTimer);
        void this.bootstrap()
          .then(() => {
            if (settled) return;
            settled = true;
            resolve();
          })
          .catch(fail);
      });
    });
  }

  /** Bounded protocol trace — lands in app.log for diagnostics. */
  private trace(msg: string): void {
    if (this.traceCount++ < 600) console.log(`[ts3] ${msg}`);
  }

  /** Wait for the plugin greeting banner before sending commands. */
  private async drainGreeting(maxMs = 2500): Promise<void> {
    const start = Date.now();
    while (!this.greetingSeen && Date.now() - start < maxMs) {
      await new Promise((r) => setTimeout(r, 50));
    }
  }

  private async bootstrap(): Promise<void> {
    await this.drainGreeting();

    if (!this.apiKey) {
      const auto = readTs3ApiKey();
      if (auto) {
        this.apiKey = auto;
        this.trace('API key picked up from clientquery.ini');
      }
    }

    if (this.apiKey) {
      let auth = await this.tryAuth();
      if (auth === 'timeout') {
        // Some builds answer slowly right after connect — retry once.
        await new Promise((r) => setTimeout(r, 700));
        auth = await this.tryAuth();
      }
      if (auth === 'error') {
        this.authFailed = true;
        throw new Error('ClientQuery auth failed — неверный API key (проверьте настройки)');
      }
      if (auth === 'timeout') {
        // Plugin alive but silent on auth → probe the link before giving up.
        const probe = await this.probeLink();
        if (!probe) {
          throw new Error(
            'ClientQuery не отвечает. Перезапустите TeamSpeak 3 и проверьте плагин Client Query (Настройки → Плагины)',
          );
        }
        console.log('[ts3] plugin ignores auth — old build without API key support, continuing');
      }
    } else {
      // No key: make sure the link answers at all so errors are clear.
      const probe = await this.probeLink();
      if (!probe) {
        throw new Error(
          'ClientQuery не отвечает. Перезапустите TeamSpeak 3 и проверьте плагин Client Query (Настройки → Плагины)',
        );
      }
    }

    await this.command('clientnotifyregister schandlerid=0 event=any');
    await this.findActiveHandler();
    await this.fullRefresh();

    if (this.periodicTimer) clearInterval(this.periodicTimer);
    this.periodicTimer = setInterval(() => void this.fullRefresh(), 45000);
    this.refreshFails = 0;
  }

  /** Try `auth`; classify the outcome instead of throwing on timeout. */
  private async tryAuth(): Promise<'ok' | 'error' | 'timeout'> {
    try {
      const auth = await this.command(`auth apikey=${ts3Escape(this.apiKey)}`, 8000);
      return auth.errorId === 0 ? 'ok' : 'error';
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      return msg.includes('timeout') ? 'timeout' : 'error';
    }
  }

  /** True when the plugin answers ordinary commands (even with an error). */
  private async probeLink(): Promise<boolean> {
    try {
      await this.command('whoami', 5000);
      return true;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      return !msg.includes('timeout');
    }
  }

  /* -------------------------------------------------- protocol I/O */

  private handleLine(line: string): void {
    if (!line) return;
    this.trace(`<< ${line}`);
    // Defensive: some builds glue the banner and the first response together
    // (banner without trailing newline) — split at the embedded error line.
    const errIdx = line.indexOf('error id=');
    if (errIdx > 0) {
      this.handleLine(line.slice(0, errIdx));
      this.handleLine(line.slice(errIdx));
      return;
    }
    if (line.startsWith('TS3 Client') || line.startsWith('selected schandlerid=')) {
      this.greetingSeen = true;
    }
    if (line.startsWith('notify')) {
      this.handleEvent(line);
      return;
    }
    if (line.startsWith('selected schandlerid=')) {
      // Unsolicited GUI tab switches — never part of a command response.
      return;
    }
    const head = this.pending[0];
    if (!head) return; // greeting banner / noise
    if (line.startsWith('error id=')) {
      this.pending.shift();
      head.lines.push(line);
      head.resolve(head.lines.join('\n'));
      return;
    }
    head.lines.push(line);
  }

  private command(cmd: string, timeoutMs = 8000): Promise<{ items: Ts3Fields[]; errorId: number; errorMessage: string }> {
    return new Promise((resolve, reject) => {
      const socket = this.socket;
      if (!socket || socket.destroyed) {
        reject(new Error('not connected'));
        return;
      }
      const timer = setTimeout(() => {
        const idx = this.pending.findIndex((p) => p === entry);
        if (idx >= 0) this.pending.splice(idx, 1);
        reject(new Error(`ClientQuery command timeout: ${cmd.split(' ')[0]}`));
      }, timeoutMs);
      const entry: PendingCommand = {
        lines: [],
        resolve: (raw) => {
          clearTimeout(timer);
          resolve(ts3ParseResponse(raw));
        },
      };
      this.pending.push(entry);
      this.trace(`>> ${cmd}`);
      socket.write(cmd + '\n');
    });
  }

  private async safe(cmd: string) {
    try {
      return await this.command(cmd);
    } catch (err) {
      return {
        items: [] as Ts3Fields[],
        errorId: -1,
        errorMessage: err instanceof Error ? err.message : String(err),
      };
    }
  }

  /* -------------------------------------------------- state */

  private async findActiveHandler(): Promise<void> {
    const list = await this.safe('serverconnectionhandlerlist');
    const ids = list.items.map((i) => fInt(i, 'schandlerid')).filter((n) => n > 0);
    const handlers = ids.length > 0 ? ids : [1];

    this.ownClidByHandler.clear();
    let fallback = handlers[0] ?? 1;

    for (const hid of handlers) {
      const use = await this.safe(`use schandlerid=${hid}`);
      if (use.errorId !== 0) continue;
      const who = await this.safe('whoami');
      if (who.errorId !== 0 || who.items.length === 0) continue;
      const clid = fInt(who.items[0]!, 'clid');
      if (clid <= 0) continue;
      this.ownClidByHandler.set(hid, clid);
      const hw = await this.safe(`clientvariable clid=${clid} client_input_hardware`);
      if (hw.items.length > 0 && fBool(hw.items[0]!, 'client_input_hardware')) {
        this.activeSchandlerId = hid;
        return;
      }
    }
    this.activeSchandlerId = fallback;
    await this.safe(`use schandlerid=${fallback}`);
  }

  private async fullRefresh(): Promise<void> {
    if (this.stopped || !this.socket) return;
    try {
      await this.safe(`use schandlerid=${this.activeSchandlerId}`);
      let who = await this.safe('whoami');
      if (who.errorId !== 0 || who.items.length === 0) {
        // The tracked handler may have gone stale (tab switch, reconnect) —
        // rediscover it before reporting "not connected".
        await this.findActiveHandler();
        await this.safe(`use schandlerid=${this.activeSchandlerId}`);
        who = await this.safe('whoami');
      }
      if (who.errorId !== 0 || who.items.length === 0) {
        this.clients.clear();
        this.ownClid = 0;
        this.ownCid = 0;
        this.publish('offline', 'not connected to a TeamSpeak server');
        return;
      }

      const self = who.items[0]!;
      this.ownClid = fInt(self, 'clid');
      this.ownCid = fInt(self, 'cid');
      this.ownClidByHandler.set(this.activeSchandlerId, this.ownClid);

      const server = await this.safe('servervariable virtualserver_name');
      if (server.items.length > 0) {
        this.serverName = fStr(server.items[0]!, 'virtualserver_name');
      }

      const chRes = await this.safe('channellist');
      this.channels.clear();
      for (const item of chRes.items) {
        const id = fInt(item, 'cid');
        if (id <= 0) continue;
        this.channels.set(id, {
          id,
          parentId: fInt(item, 'pid'),
          name: fStr(item, 'channel_name'),
        });
      }

      let clRes = await this.safe('clientlist -uid -away -voice -groups');
      if (clRes.errorId !== 0) {
        clRes = await this.safe('clientlist -uid -away -voice');
      }

      const clients = new Map<number, UserState>();
      for (const item of clRes.items) {
        const id = fInt(item, 'clid');
        if (id <= 0) continue;
        const inputMuted =
          fBool(item, 'client_input_muted') ||
          (item['client_input_hardware'] !== undefined && !fBool(item, 'client_input_hardware')) ||
          fBool(item, 'client_is_muted');
        const outputMuted =
          fBool(item, 'client_output_muted') ||
          (item['client_output_hardware'] !== undefined && !fBool(item, 'client_output_hardware'));
        clients.set(
          id,
          makeUser(String(id), fStr(item, 'client_nickname'), {
            channelId: fInt(item, 'cid'),
            talking: fBool(item, 'client_flag_talking'),
            inputMuted,
            outputMuted,
            away: fBool(item, 'client_away'),
            awayMessage: fStr(item, 'client_away_message'),
            prioritySpeaker: fBool(item, 'client_is_priority_speaker'),
            channelCommander: fBool(item, 'client_is_channel_commander'),
            isSelf: id === this.ownClid,
            isQuery: fInt(item, 'client_type') === 1,
            serverGroups: fStr(item, 'client_servergroups'),
          }),
        );
      }

      await this.readAvatars(clients);
      // Diff against the previous snapshot: notify lines can be missed (or
      // arrive before we listen) — announce joins/leaves that had no event.
      const prevClients = this.clients;
      if (this.announcedOnce) {
        for (const [id, u] of clients) {
          if (prevClients.has(id) || u.isQuery || u.isSelf || !u.nickname) continue;
          this.pushChat({
            userId: String(id),
            nickname: u.nickname,
            text: u.nickname,
            kind: 'system',
            event: 'join',
            detail: id === this.ownClid ? '' : this.channels.get(u.channelId)?.name && u.channelId === this.ownCid ? (this.channels.get(u.channelId)?.name ?? '') : '',
            own: u.channelId === this.ownCid,
          });
        }
        for (const [id, u] of prevClients) {
          if (clients.has(id) || u.isQuery || u.isSelf || !u.nickname) continue;
          this.pushChat({
            userId: String(id),
            nickname: u.nickname,
            text: u.nickname,
            kind: 'system',
            event: 'leave',
            detail: u.channelId === this.ownCid ? (this.channels.get(u.channelId)?.name ?? '') : '',
            own: u.channelId === this.ownCid,
          });
        }
      } else {
        this.announcedOnce = true;
      }
      this.clients = clients;

      // Server groups (names + icon ids) — used for class icon mapping.
      const grRes = await this.safe('servergrouplist');
      if (grRes.errorId === 0) {
        this.groups = grRes.items.map((i) => ({
          id: fInt(i, 'sgid'),
          name: fStr(i, 'name'),
          iconId: fInt(i, 'iconid'),
        }));
      }

      this.refreshFails = 0;
      this.publish('live', '');
    } catch (err) {
      // One failed refresh is transient noise — only drop to 'offline' after
      // repeated failures so the status pill does not flap.
      this.refreshFails += 1;
      if (this.refreshFails >= 2) {
        this.publish('offline', err instanceof Error ? err.message : String(err));
      }
    }
  }

  /** Avatar hash (`client_flag_avatar`) is not part of clientlist — fetch in batches. */
  private async readAvatars(clients: Map<number, UserState>): Promise<void> {
    const ids = [...clients.keys()].slice(0, 240);
    const batchSize = 40;
    for (let off = 0; off < ids.length; off += batchSize) {
      const blocks = ids.slice(off, off + batchSize).map((id) => `clid=${id} client_flag_avatar`);
      const res = await this.safe('clientvariable ' + blocks.join('|'));
      for (const item of res.items) {
        const id = fInt(item, 'clid');
        const hash = fStr(item, 'client_flag_avatar');
        const user = clients.get(id);
        if (user && hash) {
          clients.set(id, { ...user, avatarHash: hash });
        }
      }
    }
  }

  private scheduleRefresh(): void {
    if (this.refreshTimer) return;
    this.refreshTimer = setTimeout(() => {
      this.refreshTimer = null;
      void this.fullRefresh();
    }, 600);
  }

  /* -------------------------------------------------- events */

  private talkDiagSkipped = 0;

  private handleEvent(line: string): void {
    const space = line.indexOf(' ');
    const name = space < 0 ? line : line.slice(0, space);
    // Diagnostics: capture raw notify lines (talk flaps sampled 1-in-10).
    if (name === 'notifytalkstatuschange') {
      this.talkDiagSkipped += 1;
      if (this.talkDiagSkipped % 10 === 1) diag('ts3', line.slice(0, 240) + (this.talkDiagSkipped > 1 ? ' …' : ''));
    } else {
      diag('ts3', line.slice(0, 240));
    }
    const fields: Ts3Fields = {};
    for (const token of (space < 0 ? '' : line.slice(space + 1)).split(' ')) {
      if (!token) continue;
      const eq = token.indexOf('=');
      if (eq < 0) {
        fields[token] = '';
      } else {
        fields[token.slice(0, eq)] = ts3Unescape(token.slice(eq + 1));
      }
    }

    const schandlerid = fInt(fields, 'schandlerid');
    const inScope = schandlerid <= 0 || schandlerid === this.activeSchandlerId;

    switch (name) {
      case 'notifytalkstatuschange': {
        const clid = fInt(fields, 'clid');
        const talking = fInt(fields, 'status') === 1;
        // Whisper detection (available on newer clients): targetmode ≠ 0 or an
        // explicit whisper flag marks the speech as targeted/whispered.
        const whisper = talking && (('targetmode' in fields && fInt(fields, 'targetmode') !== 0) || fBool(fields, 'whisper'));
        if (!inScope) {
          // Self started speaking on another server tab → follow the mic.
          if (talking && schandlerid > 0 && this.ownClidByHandler.get(schandlerid) === clid) {
            this.activeSchandlerId = schandlerid;
            this.scheduleRefresh();
          }
          return;
        }
        const user = this.clients.get(clid);
        if (user && (user.talking !== talking || user.whispering !== whisper)) {
          this.clients.set(clid, { ...user, talking, whispering: whisper });
          this.emitState();
        }
        return;
      }

      case 'notifytextmessage': {
        // Chat is accepted from every server tab (not just the tracked one):
        // messages on other tabs are still news and feed the unread badge.
        const targetMode = fInt(fields, 'targetmode');
        const kind: ChatMessage['kind'] = targetMode === 1 ? 'private' : targetMode === 3 ? 'server' : 'channel';
        const text = fStr(fields, 'msg');
        if (!text) return;
        const userId = String(fInt(fields, 'invokerid') || fInt(fields, 'clid'));
        this.pushChat({
          userId,
          nickname: fStr(fields, 'invokername') || this.clients.get(Number(userId))?.nickname || '?',
          text,
          kind,
        });
        return;
      }

      case 'notifyclientupdated': {
        if (!inScope) return;
        const clid = fInt(fields, 'clid');
        const user = this.clients.get(clid);
        if (!user) return;
        const patch: Partial<UserState> = {};
        if ('client_nickname' in fields) patch.nickname = fStr(fields, 'client_nickname');
        if ('client_input_muted' in fields || 'client_is_muted' in fields) {
          patch.inputMuted = fBool(fields, 'client_input_muted') || fBool(fields, 'client_is_muted');
        }
        if ('client_output_muted' in fields) patch.outputMuted = fBool(fields, 'client_output_muted');
        if ('client_away' in fields) patch.away = fBool(fields, 'client_away');
        if ('client_away_message' in fields) patch.awayMessage = fStr(fields, 'client_away_message');
        if ('client_is_priority_speaker' in fields) patch.prioritySpeaker = fBool(fields, 'client_is_priority_speaker');
        if ('client_flag_talking' in fields) patch.talking = fBool(fields, 'client_flag_talking');
        this.clients.set(clid, { ...user, ...patch });
        // Group membership changes («добавил пользователя в группу …») —
        // diff the group fields and announce them as sys events.
        const sg = 'client_servergroups' in fields ? fStr(fields, 'client_servergroups') : undefined;
        const cg = 'client_channel_group_id' in fields
          ? fStr(fields, 'client_channel_group_name') || String(fInt(fields, 'client_channel_group_id'))
          : undefined;
        const prev = this.groupMem.get(clid) ?? { sg: '', cg: '' };
        if (sg !== undefined && sg !== prev.sg) {
          this.pushChat({
            userId: String(clid),
            nickname: user.nickname,
            text: 'server',
            kind: 'system',
            event: 'group',
            detail: sg.split(',').filter(Boolean).join(', ') || '—',
            own: user.channelId === this.ownCid,
          });
        }
        if (cg !== undefined && cg !== prev.cg) {
          this.pushChat({
            userId: String(clid),
            nickname: user.nickname,
            text: 'channel',
            kind: 'system',
            event: 'group',
            detail: cg || '—',
            own: user.channelId === this.ownCid,
          });
        }
        this.groupMem.set(clid, {
          sg: sg ?? prev.sg,
          cg: cg ?? prev.cg,
        });
        this.emitState();
        return;
      }

      case 'notifyclientmoved': {
        if (!inScope) return;
        const clid = fInt(fields, 'clid');
        const user = this.clients.get(clid);
        const to = fInt(fields, 'ctid');
        if (user?.isSelf) {
          this.ownCid = to;
        } else if (user && !user.isQuery && clid !== this.ownClid) {
          // v1.3.1: every move on the server («кто куда перешёл») — the user
          // wants the full picture, not only own-channel touches.
          this.pushChat({
            userId: String(clid),
            nickname: user.nickname,
            text: user.nickname,
            kind: 'system',
            event: 'move',
            detail: this.channels.get(to)?.name ?? '',
            own: to === this.ownCid || user.channelId === this.ownCid,
          });
        }
        this.scheduleRefresh();
        return;
      }

      case 'notifyclientleftview': {
        if (!inScope) return;
        const clid = fInt(fields, 'clid');
        const user = this.clients.get(clid);
        const nick = fStr(fields, 'client_nickname') || user?.nickname || '';
        const wasOurs = !!user && user.channelId === this.ownCid;
        if (nick && !user?.isQuery && clid !== this.ownClid) {
          this.pushChat({
            userId: String(clid),
            nickname: nick,
            text: nick,
            kind: 'system',
            event: 'leave',
            detail: wasOurs ? (this.channels.get(user!.channelId)?.name ?? '') : '',
            own: wasOurs,
          });
        }
        if (this.clients.delete(clid)) this.emitState();
        this.scheduleRefresh();
        return;
      }

      case 'notifycliententerview': {
        if (!inScope) return;
        const clid = fInt(fields, 'clid');
        const nick = fStr(fields, 'client_nickname');
        const isQuery = fInt(fields, 'client_type') === 1;
        if (nick && !isQuery && clid !== this.ownClid) {
          const ctid = fInt(fields, 'ctid');
          const inOur = ctid > 0 && ctid === this.ownCid;
          this.pushChat({
            userId: String(clid),
            nickname: nick,
            text: nick,
            kind: 'system',
            event: 'join',
            detail: inOur ? (this.channels.get(ctid)?.name ?? '') : '',
            own: inOur,
          });
        }
        this.scheduleRefresh();
        return;
      }

      case 'notifyclientpoke': {
        if (!inScope) return;
        const nick = fStr(fields, 'invokername') || 'Кто-то';
        const msg = fStr(fields, 'msg');
        this.pushChat({
          userId: String(fInt(fields, 'invokerid')),
          nickname: nick,
          text: msg || nick,
          kind: 'poke',
          event: 'poke',
          detail: msg,
        });
        return;
      }

      case 'notifychannelcreated':
      case 'notifychanneledited':
      case 'notifychanneldeleted':
      case 'notifychannelmoved':
      case 'notifyserverupdated':
      case 'notifyserveredited':
      case 'notifyconnectstatuschange': {
        if (!inScope) return;
        this.scheduleRefresh();
        return;
      }

      case 'notifycurrentserverconnectionchanged': {
        // The user switched server tabs in the TS3 client — follow them,
        // otherwise whoami/chat run against a stale handler and the app
        // shows "no connection" while the client is online.
        const hid = fInt(fields, 'schandlerid');
        if (hid > 0 && hid !== this.activeSchandlerId) {
          this.activeSchandlerId = hid;
          this.scheduleRefresh();
        }
        return;
      }
    }
  }

  /* -------------------------------------------------- publish */

  private pushChat(msg: Omit<ChatMessage, 'id' | 'at'>): void {
    const now = Date.now();
    // Duplicates arrive twice (own-echo with different metadata, refresh
    // diffs racing notify lines): scan the recent tail, not just the last row.
    for (let i = this.chat.length - 1; i >= 0 && i >= this.chat.length - 12; i--) {
      const prev = this.chat[i]!;
      if (now - prev.at > 5000) break;
      if (prev.text === msg.text && prev.userId === msg.userId && (prev.event ?? '') === (msg.event ?? '') && (prev.detail ?? '') === (msg.detail ?? '')) {
        // Merge metadata (echo copies may carry a better kind/nickname).
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

  private buildState(): OverlayState {
    return {
      ...emptyState('ts3', this.curStatus, this.curDetail),
      serverName: this.serverName,
      channelName: this.channels.get(this.ownCid)?.name ?? '',
      channelId: this.ownCid > 0 ? String(this.ownCid) : '',
      selfId: this.ownClid > 0 ? String(this.ownClid) : null,
      users: [...this.clients.values()].filter(
        (c) => this.ownCid > 0 && c.channelId === this.ownCid,
      ),
      groups: this.groups,
      chat: this.chat,
      updatedAt: Date.now(),
    };
  }

  private publish(status: StatusCode, detail: string): void {
    this.curStatus = status;
    this.curDetail = detail;
    this.emitState();
  }

  /** Coalesce bursts into at most ~10 broadcasts per second. */
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
