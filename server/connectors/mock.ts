import { EventEmitter } from 'node:events';
import { ChatMessage, OverlayState, ServerGroup, UserState, emptyState, makeUser } from '../model.js';
import type { Connector } from './wire.js';

interface MockActor {
  user: UserState;
  /** Talking burst end timestamp (0 = silent). */
  talkUntil: number;
  /** Earliest timestamp of the next talking burst. */
  nextTalkAt: number;
  /** When false the actor is out of the channel (reserve pool). */
  present: boolean;
}

const SEED: Array<[string, Partial<UserState>]> = [
  ['Nikita', { serverGroups: '6' }],
  ['Артём Соколов', { inputMuted: true, serverGroups: '12' }],
  ['ShadowFox', { prioritySpeaker: true, serverGroups: '6,12' }],
  ['мурка :3', { serverGroups: '16' }],
  ['DJ Vasya', { outputMuted: true, serverGroups: '6' }],
  ['Ольга', { away: true, awayMessage: 'brb', serverGroups: '12' }],
  ['Kirill_S', { channelCommander: true, serverGroups: '6,10' }],
  ['Призрак', { serverGroups: '16' }],
  ['toster.exe', { inputMuted: true, serverGroups: '6' }],
  ['Максим', { serverGroups: '12' }],
];

const CHANNELS = ['Рандеву', 'Игровой канал', 'Lounge', 'Тихая гавань', 'Музыкальная'];

const MOCK_GROUPS: ServerGroup[] = [
  { id: 6, name: 'Воины', iconId: 1006 },
  { id: 12, name: 'Маги', iconId: 1012 },
  { id: 16, name: 'Тени', iconId: 1016 },
  { id: 10, name: 'Командиры', iconId: 1010 },
];

const CHAT_LINES = [
  'всем привет!',
  'кто сегодня в рейде?',
  'гг',
  'погнали на арену',
  'у меня лагает голос',
  'кто-нибудь слышит меня?',
  'спасибо за игру!',
  'через 5 минут буду',
];

/**
 * Demo data source: simulates a living TeamSpeak channel so the overlay can
 * be previewed (and demoed) without any client running.
 */
export class MockConnector extends EventEmitter implements Connector {
  private timer: NodeJS.Timeout | null = null;
  private actors: MockActor[] = [];
  private selfId = 'self';
  private channelName = CHANNELS[0]!;
  private chat: ChatMessage[] = [];
  private chatSeq = 0;
  private chatLine = 0;
  private lastEmit = 0;
  private pending: OverlayState | null = null;
  private flushTimer: NodeJS.Timeout | null = null;

  start(): void {
    this.seed();
    this.timer = setInterval(() => this.tick(), 450);
    this.emitState();
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    if (this.flushTimer) clearTimeout(this.flushTimer);
    this.timer = null;
    this.flushTimer = null;
    this.actors = [];
  }

  private seed(): void {
    const now = Date.now();
    this.actors = SEED.map(([nickname, patch], i) => ({
      user: makeUser(i === 0 ? this.selfId : `u${i}`, nickname, {
        isSelf: i === 0,
        avatarHash: `mock-${nickname}`,
        ...patch,
      }),
      talkUntil: 0,
      nextTalkAt: now + 500 + Math.random() * 4000,
      present: i < 6, // a few actors start outside the channel
    }));
  }

  private tick(): void {
    const now = Date.now();
    let changed = false;

    for (const actor of this.actors) {
      if (!actor.present) continue;
      const u = actor.user;
      if (u.talking && now >= actor.talkUntil) {
        actor.user = { ...u, talking: false, whispering: false };
        actor.nextTalkAt = now + 400 + Math.random() * 2600;
        changed = true;
      } else if (!u.talking && !u.inputMuted && !u.away && now >= actor.nextTalkAt && Math.random() < 0.3) {
        actor.user = { ...u, talking: true, whispering: Math.random() < 0.22 };
        actor.talkUntil = now + 1100 + Math.random() * 3600;
        changed = true;
      }
    }

    // Occasional chat traffic.
    if (Math.random() < 0.03) {
      const present = this.actors.filter((a) => a.present && !a.user.isSelf);
      if (present.length) {
        const a = present[Math.floor(Math.random() * present.length)]!;
        const msg: ChatMessage = {
          id: `m${++this.chatSeq}`,
          userId: a.user.id,
          nickname: a.user.nickname,
          text: CHAT_LINES[this.chatLine++ % CHAT_LINES.length]!,
          kind: Math.random() < 0.2 ? 'private' : 'channel',
          at: Date.now(),
        };
        this.chat.push(msg);
        if (this.chat.length > 20) this.chat = this.chat.slice(-20);
        changed = true;
      }
    }

    // Random life events: mute / away / join / leave / channel hop.
    if (Math.random() < 0.055) {
      const present = this.actors.filter((a) => a.present && !a.user.isSelf);
      const absent = this.actors.filter((a) => !a.present);
      const pick = <T,>(arr: T[]): T | undefined => (arr.length ? arr[Math.floor(Math.random() * arr.length)] : undefined);

      const roll = Math.random();
      if (roll < 0.3) {
        const a = pick(present);
        if (a) {
          a.user = { ...a.user, inputMuted: !a.user.inputMuted, talking: false };
          changed = true;
        }
      } else if (roll < 0.45) {
        const a = pick(present);
        if (a) {
          a.user = { ...a.user, away: !a.user.away, awayMessage: a.user.away ? '' : 'brb', talking: false };
          changed = true;
        }
      } else if (roll < 0.62 && absent.length > 0) {
        const a = pick(absent);
        if (a) {
          a.present = true;
          a.user = { ...a.user, talking: false };
          a.nextTalkAt = now + 800 + Math.random() * 3000;
          changed = true;
        }
      } else if (roll < 0.78 && present.length > 3) {
        const a = pick(present);
        if (a) {
          a.present = false;
          a.user = { ...a.user, talking: false };
          changed = true;
        }
      } else if (roll < 0.88) {
        // Channel hop: everyone moves together occasionally.
        const next = CHANNELS[Math.floor(Math.random() * CHANNELS.length)]!;
        if (next !== this.channelName) {
          this.channelName = next;
          changed = true;
        }
      } else {
        const a = pick(present);
        if (a) {
          a.user = { ...a.user, outputMuted: !a.user.outputMuted };
          changed = true;
        }
      }
    }

    if (changed) this.emitState();
  }

  private snapshot(): OverlayState {
    const users = this.actors.filter((a) => a.present).map((a) => a.user);
    return {
      ...emptyState('mock', 'live'),
      serverName: 'TeamSpeak Demo',
      channelName: this.channelName,
      channelId: 'demo-channel',
      selfId: this.selfId,
      users,
      groups: MOCK_GROUPS,
      chat: this.chat,
      updatedAt: Date.now(),
    };
  }

  /** Coalesce bursts of changes into at most ~8 broadcasts per second. */
  private emitState(): void {
    this.pending = this.snapshot();
    const now = Date.now();
    const wait = Math.max(0, 125 - (now - this.lastEmit));
    if (this.flushTimer) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      this.lastEmit = Date.now();
      if (this.pending) {
        this.emit('state', this.pending);
        this.pending = null;
      }
    }, wait);
  }
}
