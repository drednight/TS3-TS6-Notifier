import { EventEmitter } from 'node:events';
import type { OverlayState } from '../model.js';

/** A data source that normalises TeamSpeak state and emits `state` events. */
export interface Connector extends EventEmitter {
  start(): void;
  stop(): void;
}

export type ConnectorEvents = {
  state: [OverlayState];
};

/* ------------------------------------------------------------------ */
/* TS3 ClientQuery wire-format helpers                                 */
/* ------------------------------------------------------------------ */

export function ts3Unescape(value: string): string {
  if (!value.includes('\\')) return value;
  let out = '';
  for (let i = 0; i < value.length; i++) {
    const c = value[i];
    if (c !== '\\' || i + 1 >= value.length) {
      out += c;
      continue;
    }
    const n = value[++i];
    out +=
      n === 's' ? ' '
      : n === 'p' ? '|'
      : n === '/' ? '/'
      : n === '\\' ? '\\'
      : n === 'n' ? '\n'
      : n === 'r' ? '\r'
      : n === 't' ? '\t'
      : n === 'v' ? '\v'
      : n === 'f' ? '\f'
      : n === 'a' ? '\a'
      : n === 'b' ? '\b'
      : n;
  }
  return out;
}

export function ts3Escape(value: string): string {
  return value.replace(/([\\/\s|])/g, (m) => {
    if (m === '\\') return '\\\\';
    if (m === '/') return '\\/';
    if (m === ' ') return '\\s';
    return '\\p';
  });
}

export type Ts3Fields = Record<string, string>;

export function ts3ParseFields(line: string): Ts3Fields {
  const result: Ts3Fields = {};
  for (const token of line.split(' ')) {
    if (!token) continue;
    const eq = token.indexOf('=');
    if (eq < 0) {
      result[token] = '';
      continue;
    }
    result[token.slice(0, eq)] = ts3Unescape(token.slice(eq + 1));
  }
  return result;
}

export interface Ts3Response {
  items: Ts3Fields[];
  errorId: number;
  errorMessage: string;
}

export function ts3ParseResponse(raw: string): Ts3Response {
  const items: Ts3Fields[] = [];
  let errorId = 0;
  let errorMessage = '';
  const dataLines: string[] = [];

  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    if (trimmed.startsWith('error id=')) {
      const fields = ts3ParseFields(trimmed);
      errorId = parseInt(fields['id'] ?? '0', 10) || 0;
      errorMessage = fields['msg'] ?? '';
      continue;
    }
    dataLines.push(trimmed);
  }

  if (dataLines.length > 0) {
    for (const item of dataLines.join(' ').split('|')) {
      if (!item.trim()) continue;
      items.push(ts3ParseFields(item.trim()));
    }
  }

  return { items, errorId, errorMessage };
}

export function fStr(fields: Ts3Fields, key: string): string {
  return fields[key] ?? '';
}

export function fInt(fields: Ts3Fields, key: string, fallback = 0): number {
  const v = parseInt(fields[key] ?? '', 10);
  return Number.isFinite(v) ? v : fallback;
}

export function fBool(fields: Ts3Fields, key: string): boolean {
  return fInt(fields, key) !== 0;
}

/* ------------------------------------------------------------------ */
/* Loose JSON helpers for the TS6 Remote Apps protocol                 */
/* ------------------------------------------------------------------ */

export type JObj = Record<string, unknown>;

export function asObj(value: unknown): JObj | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as JObj) : null;
}

export function jStr(obj: JObj | null, key: string): string {
  const v = obj?.[key];
  if (typeof v === 'string') return v;
  if (typeof v === 'number') return String(v);
  if (typeof v === 'boolean') return v ? '1' : '0';
  return '';
}

export function jInt(obj: JObj | null, key: string): number {
  const v = obj?.[key];
  if (typeof v === 'number') return Math.trunc(v);
  if (typeof v === 'string') {
    const n = parseInt(v, 10);
    return Number.isFinite(n) ? n : 0;
  }
  return 0;
}

export function jBool(obj: JObj | null, key: string): boolean {
  const v = obj?.[key];
  if (typeof v === 'boolean') return v;
  if (typeof v === 'number') return v !== 0;
  if (typeof v === 'string') return v === '1' || v.toLowerCase() === 'true';
  return false;
}

export function hasKey(obj: JObj | null, key: string): boolean {
  return !!obj && Object.prototype.hasOwnProperty.call(obj, key);
}

export function jArr(obj: JObj | null, key: string): unknown[] {
  const v = obj?.[key];
  return Array.isArray(v) ? v : [];
}
