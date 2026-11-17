/**
 * Shared state model: everything the overlay needs to render one screen.
 * Connectors (mock / TS3 ClientQuery / TS6 Remote Apps) all normalise
 * into this shape, so the frontend has a single contract.
 */

export type SourceKind = 'mock' | 'ts3' | 'ts6';

/** Coarse connection status, localised by the frontend. */
export type StatusCode =
  | 'connecting' // dialing the client / server
  | 'live' // receiving data
  | 'waiting-auth' // TS6 permission prompt pending / TS3 apikey missing
  | 'offline' // TeamSpeak not reachable / not connected to a server
  | 'error'; // misconfiguration or protocol error (see detail)

export interface UserState {
  id: string;
  nickname: string;
  /** Channel the user is currently in (0 = unknown). */
  channelId: number;
  /** Currently speaking. */
  talking: boolean;
  /** Whispering (target-mode speech) — shown in a separate fading section. */
  whispering: boolean;
  /** Microphone muted (hardware off, client-side mute or server-side mute). */
  inputMuted: boolean;
  /** Speakers/headphones muted. */
  outputMuted: boolean;
  away: boolean;
  awayMessage: string;
  prioritySpeaker: boolean;
  channelCommander: boolean;
  /** This is "you". */
  isSelf: boolean;
  /** Query bot / server query client — usually hidden in the overlay. */
  isQuery: boolean;
  /** MD5 avatar hash (TS3 `client_flag_avatar`, TS6 `flagAvatar`) — used to tint avatars. */
  avatarHash: string;
  /** Comma-separated server group ids, e.g. "6,12". */
  serverGroups: string;
}

export interface OverlayState {
  source: SourceKind;
  status: StatusCode;
  /** Free-form detail (error message, hints). Not localised. */
  detail: string;
  serverName: string;
  channelName: string;
  channelId: string;
  selfId: string | null;
  /** Users of the current channel, including self. */
  users: UserState[];
  /** Server groups (TS3 provides names + icon ids; TS6 usually only ids). */
  groups: ServerGroup[];
  /** Rolling chat log for toast notifications. */
  chat: ChatMessage[];
  updatedAt: number;
}

export interface ServerGroup {
  id: number;
  name: string;
  /** Server-side icon id (0 = none) — used to map groups to game classes. */
  iconId: number;
}

export interface ChatMessage {
  id: string;
  userId: string;
  nickname: string;
  text: string;
  kind: 'channel' | 'private' | 'server' | 'system' | 'poke';
  at: number;
  /** System rows carry structured info so the UI can localise them. */
  event?: 'join' | 'leave' | 'move' | 'poke' | 'group';
  /** Channel name for join/leave/move, poke body for pokes. */
  detail?: string;
  /** True when the event touched the user's own channel (scope filtering). */
  own?: boolean;
}

/* ---------------- diagnostics ring buffer ----------------
 * Raw TS3/TS6 traffic + app notes, exposed via GET /api/diagnostics so the
 * user can copy a report. Secrets are redacted on write. */
const DIAG_LIMIT = 400;
export const DIAG: string[] = [];

export function diag(scope: string, line: string): void {
  const redacted = String(line).replace(
    /(apikey|api_key|token|password|passwd)(\s*[=:]\s*)\S+/gi,
    '$1$2***',
  );
  DIAG.push(`${new Date().toTimeString().slice(0, 8)} [${scope}] ${redacted}`);
  if (DIAG.length > DIAG_LIMIT) DIAG.splice(0, DIAG.length - DIAG_LIMIT);
}

export interface Ts3Config {
  host: string;
  port: number;
  /** Client Query API key (TS3 client → Settings → Plugins → Client Query). */
  apiKey: string;
}

export interface Ts6Config {
  host: string;
  port: number;
  /** Remote Apps API key, issued by the TS6 client after the permission prompt. */
  apiKey: string;
}

export interface Appearance {
  theme: 'glass' | 'midnight' | 'frost' | 'neon';
  accent: string;
  nickColor: string;
  msgColor: string;
  talkOutline: string;
  /** Seconds a whisperer lingers in the whisper section after whispering stops. */
  whisperDelay: number;
  glow: number;
  opacity: number;
  blur: number;
  radius: number;
  scale: number;
  fontSize: number;
  corner: 'tl' | 'tr' | 'bl' | 'br';
  marginX: number;
  marginY: number;
  language: 'auto' | 'ru' | 'en';
  showHeader: boolean;
  showServerName: boolean;
  showUserCount: boolean;
  showBanner: boolean;
  showAvatars: boolean;
  showBadges: boolean;
  showSelf: boolean;
  hideQuery: boolean;
  animations: boolean;
  eqStyle: 'bars' | 'ring' | 'dots' | 'none';
  talkingFirst: boolean;
  onlyTalking: boolean;
  compact: boolean;
  hideEmpty: boolean;
  /** Whisper section with fading rows. */
  showWhispers: boolean;
  /** Chat toasts. */
  showChat: boolean;
  chatDelay: number;
  chatPrivate: boolean;
  /** Unread-message badge on the overlay. */
  showUnreadBadge: boolean;
  /** Recent server events block (join/leave/move/poke/chat). */
  showEvents: boolean;
  showEvJoin: boolean;
  showEvLeave: boolean;
  showEvMove: boolean;
  showEvPoke: boolean;
  showEvChat: boolean;
  eventsCount: number;
  eventsDelay: number;
  /** Per-type event scope: 'all' = whole server, 'own' = own channel only. */
  evScopeJoin: 'all' | 'own';
  evScopeLeave: 'all' | 'own';
  evScopeMove: 'all' | 'own';
  evScopeGroup: 'all' | 'own';
  evScopeSys: 'all' | 'own';
  evScopeChat: 'all' | 'own';
}

export interface AppConfig {
  source: SourceKind;
  ts3: Ts3Config;
  ts6: Ts6Config;
  appearance: Appearance;
  /** Desktop (Electron) app preferences. */
  desktop: {
    clickThrough: boolean;
    autostart: boolean;
    visible: boolean;
    /** First run already greeted the user with the settings window. */
    seen: boolean;
    /** Display for the overlay window: 'primary' or an Electron display id. */
    display: string;
    /** Relaunch as administrator (Windows only, UAC prompt at startup). */
    elevate: boolean;
    /** Show the in-game overlay window (off = server-only mode, e.g. for OBS). */
    overlayWindow: boolean;
    /** Preferred local port for the web server (stable URL for OBS). 0 = random. */
    port: number;
  };
  /** Game class icons: map TS server groups / icon ids to class keys. */
  classes: {
    enabled: boolean;
    /** "icon:12345" or "group:6" → class key (see frontend CLASS list). */
    map: Record<string, string>;
  };
}

export const DEFAULT_APPEARANCE: Appearance = {
  theme: 'glass',
  accent: '#7c6cff',
  nickColor: '#e8ecff',
  msgColor: '#b6b6bd',
  talkOutline: '#57e389',
  whisperDelay: 4,
  glow: 0.7,
  opacity: 0.72,
  blur: 18,
  radius: 16,
  scale: 1,
  fontSize: 13,
  corner: 'tr',
  marginX: 18,
  marginY: 18,
  language: 'auto',
  showHeader: true,
  showServerName: true,
  showUserCount: true,
  showBanner: true,
  showAvatars: true,
  showBadges: true,
  showSelf: true,
  hideQuery: true,
  animations: true,
  eqStyle: 'bars',
  talkingFirst: true,
  onlyTalking: false,
  compact: false,
  hideEmpty: false,
  showWhispers: true,
  showChat: true,
  chatDelay: 8,
  chatPrivate: true,
  showUnreadBadge: true,
  showEvents: true,
  showEvJoin: true,
  showEvLeave: true,
  showEvMove: true,
  showEvPoke: true,
  showEvChat: true,
  eventsCount: 5,
  eventsDelay: 20,
  evScopeJoin: 'all',
  evScopeLeave: 'all',
  evScopeMove: 'all',
  evScopeGroup: 'all',
  evScopeSys: 'all',
  evScopeChat: 'all',
};

export const DEFAULT_CONFIG: AppConfig = {
  source: 'mock',
  ts3: { host: '127.0.0.1', port: 25639, apiKey: '' },
  ts6: { host: '127.0.0.1', port: 5899, apiKey: '' },
  appearance: DEFAULT_APPEARANCE,
  desktop: { clickThrough: true, autostart: false, visible: true, seen: false, display: 'primary', elevate: true, overlayWindow: true, port: 48911 },
  classes: {
    enabled: false,
    // Default group-id → class mapping (the user's server). Override in
    // settings → «Подключение» → «Группы сервера» ('none' unmaps an id).
    map: {
      '442': 'archer',
      '444': 'priest',
      '445': 'warrior',
      '446': 'mage',
      '447': 'werewolf',
      '448': 'druid',
      '449': 'assassin',
      '450': 'shaman',
      '451': 'ghost',
      '452': 'reaper',
      '453': 'paladin',
      '454': 'gunslinger',
      '455': 'guardian',
      '456': 'mystic',
      '502': 'bard',
      '503': 'blood',
      '506': 'monkey',
    },
  },
};

export function emptyState(source: SourceKind, status: StatusCode, detail = ''): OverlayState {
  return {
    source,
    status,
    detail,
    serverName: '',
    channelName: '',
    channelId: '',
    selfId: null,
    users: [],
    groups: [],
    chat: [],
    updatedAt: Date.now(),
  };
}

export function makeUser(id: string, nickname: string, patch: Partial<UserState> = {}): UserState {
  return {
    id,
    nickname,
    channelId: 0,
    talking: false,
    whispering: false,
    inputMuted: false,
    outputMuted: false,
    away: false,
    awayMessage: '',
    prioritySpeaker: false,
    channelCommander: false,
    isSelf: false,
    isQuery: false,
    avatarHash: '',
    serverGroups: '',
    ...patch,
  };
}
