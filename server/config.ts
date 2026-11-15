import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { AppConfig, Appearance, DEFAULT_CONFIG, DEFAULT_APPEARANCE } from './model.js';

function merge<T extends object>(base: T, patch: unknown): T {
  if (!patch || typeof patch !== 'object') return { ...base };
  const baseRec = base as Record<string, unknown>;
  const out: Record<string, unknown> = { ...baseRec };
  for (const [key, value] of Object.entries(patch as Record<string, unknown>)) {
    if (!(key in baseRec)) continue;
    const current = baseRec[key];
    if (value && typeof value === 'object' && !Array.isArray(value) && current && typeof current === 'object' && !Array.isArray(current)) {
      out[key] = merge(current as object, value);
    } else if (value !== undefined && value !== null) {
      out[key] = value;
    }
  }
  return out as T;
}

export function mergeAppearance(patch: unknown): Appearance {
  return merge(DEFAULT_APPEARANCE, patch);
}

export class ConfigStore {
  private config: AppConfig;
  private configPath: string;

  constructor(dataDir: string) {
    this.configPath = join(dataDir, 'config.json');
    this.config = this.load();
  }

  private load(): AppConfig {
    try {
      const raw = JSON.parse(readFileSync(this.configPath, 'utf8')) as unknown;
      const merged = merge(DEFAULT_CONFIG, raw);
      merged.appearance = merge(DEFAULT_APPEARANCE, (raw as { appearance?: unknown })?.appearance);
      return merged;
    } catch {
      return structuredClone(DEFAULT_CONFIG);
    }
  }

  private save(): void {
    try {
      mkdirSync(dirname(this.configPath), { recursive: true });
      writeFileSync(this.configPath, JSON.stringify(this.config, null, 2), 'utf8');
    } catch (err) {
      console.error('[config] failed to persist:', err);
    }
  }

  get(): AppConfig {
    return this.config;
  }

  /** Merge a patch into the config. Returns the fields that require a connector restart. */
  update(patch: unknown): { restart: boolean } {
    const before = JSON.stringify({ source: this.config.source, ts3: this.config.ts3, ts6: this.config.ts6 });
    this.config = merge(this.config, patch);
    // Appearance is validated/normalised separately so junk keys never leak through.
    if (patch && typeof patch === 'object' && 'appearance' in (patch as object)) {
      this.config.appearance = mergeAppearance((patch as { appearance?: unknown }).appearance);
    }
    this.save();
    const after = JSON.stringify({ source: this.config.source, ts3: this.config.ts3, ts6: this.config.ts6 });
    return { restart: before !== after };
  }
}
