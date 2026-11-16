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
  updatedAt: number;
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
  glow: number;
  opacity: number;
  blur: number;
  radius: number;
  scale: number;
  fontSize: number;
  corner: 'tl' | 'tr' | 'bl' | 'br';
  margin: number;
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
  };
}

export const DEFAULT_APPEARANCE: Appearance = {
  theme: 'glass',
  accent: '#7c6cff',
  glow: 0.7,
  opacity: 0.72,
  blur: 18,
  radius: 16,
  scale: 1,
  fontSize: 13,
  corner: 'tr',
  margin: 18,
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
};

export const DEFAULT_CONFIG: AppConfig = {
  source: 'mock',
  ts3: { host: '127.0.0.1', port: 25639, apiKey: '' },
  ts6: { host: '127.0.0.1', port: 5899, apiKey: '' },
  appearance: DEFAULT_APPEARANCE,
  desktop: { clickThrough: true, autostart: false, visible: true, seen: false },
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
    updatedAt: Date.now(),
  };
}

export function makeUser(id: string, nickname: string, patch: Partial<UserState> = {}): UserState {
  return {
    id,
    nickname,
    channelId: 0,
    talking: false,
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
