#!/usr/bin/env node
/* Synthesize small WAV soundboard presets into public/presets/.
 *
 * Usage: node tools/make-sounds.mjs
 * Pure Node — no dependencies. Mono 44.1 kHz 16-bit PCM.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'public', 'presets');
const SR = 44100;

function wav(samples) {
  const n = samples.length;
  const buf = Buffer.alloc(44 + n * 2);
  buf.write('RIFF', 0);
  buf.writeUInt32LE(36 + n * 2, 4);
  buf.write('WAVE', 8);
  buf.write('fmt ', 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20); // PCM
  buf.writeUInt16LE(1, 22); // mono
  buf.writeUInt32LE(SR, 24);
  buf.writeUInt32LE(SR * 2, 28);
  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write('data', 36);
  buf.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) {
    const v = Math.max(-1, Math.min(1, samples[i]));
    buf.writeInt16LE(Math.round(v * 32767), 44 + i * 2);
  }
  return buf;
}

const t = (i) => i / SR;

function tone({ dur, freq, type = 'sine', vol = 0.7, attack = 0.005, release = 0.05, sweep = 0, vibrato = 0 }) {
  const n = Math.round(dur * SR);
  const out = new Float64Array(n);
  let phase = 0;
  for (let i = 0; i < n; i++) {
    const tt = t(i);
    const f = freq + sweep * (tt / dur) + (vibrato ? vibrato * Math.sin(2 * Math.PI * 6 * tt) : 0);
    phase += (2 * Math.PI * f) / SR;
    let s = 0;
    if (type === 'sine') s = Math.sin(phase);
    else if (type === 'square') s = Math.sign(Math.sin(phase)) * 0.7;
    else if (type === 'saw') s = 2 * ((phase / (2 * Math.PI)) % 1) - 1;
    else if (type === 'triangle') s = 2 * Math.abs(2 * ((phase / (2 * Math.PI)) % 1) - 1) - 1;
    const env = Math.min(1, tt / attack) * Math.min(1, (dur - tt) / release);
    out[i] = s * vol * env;
  }
  return out;
}

function noise({ dur, vol = 0.5, attack = 0.002, release = 0.05, sweepLP = 0 }) {
  const n = Math.round(dur * SR);
  const out = new Float64Array(n);
  let lp = 0;
  let seed = 1234567;
  for (let i = 0; i < n; i++) {
    const tt = t(i);
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    const white = (seed / 0x3fffffff) - 1;
    const cutoff = sweepLP ? 0.02 + 0.4 * (sweepLP > 0 ? tt / dur : 1 - tt / dur) : 0.25;
    lp += cutoff * (white - lp);
    const env = Math.min(1, tt / attack) * Math.min(1, (dur - tt) / release);
    out[i] = lp * vol * env * 2.2;
  }
  return out;
}

function mix(...parts) {
  const n = Math.max(...parts.map((p) => p.length + (p.offset || 0)));
  const out = new Float64Array(n);
  for (const p of parts) {
    const off = p.offset || 0;
    for (let i = 0; i < p.length; i++) out[off + i] += p[i];
  }
  return out;
}

function concat(...parts) {
  const n = parts.reduce((s, p) => s + p.length, 0);
  const out = new Float64Array(n);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

const at = (arr, offsetSec) => {
  arr.offset = Math.round(offsetSec * SR);
  return arr;
};

function save(name, samples) {
  let peak = 0;
  for (const s of samples) peak = Math.max(peak, Math.abs(s));
  const gain = peak > 0.95 ? 0.95 / peak : 1;
  const scaled = Float64Array.from(samples, (s) => s * gain);
  writeFileSync(join(OUT, name), wav(scaled));
  console.log(`✓ ${name} (${(samples.length / SR).toFixed(2)}s)`);
}

mkdirSync(OUT, { recursive: true });

// coin: quick E6→B6 chime
save('coin.wav', concat(tone({ dur: 0.07, freq: 1318.5, type: 'square', vol: 0.5 }), tone({ dur: 0.22, freq: 1975.5, type: 'square', vol: 0.5, release: 0.18 })));

// powerup: rising sweep
save('powerup.wav', tone({ dur: 0.5, freq: 320, sweep: 900, type: 'triangle', vol: 0.7, release: 0.12 }));

// drum: kick + snare
save('drum.wav', mix(at(tone({ dur: 0.16, freq: 150, sweep: -110, type: 'sine', vol: 0.9, release: 0.1 }), 0), at(noise({ dur: 0.18, vol: 0.42, release: 0.14 }), 0.12)));

// alert: alternating alarm
save('alert.wav', concat(tone({ dur: 0.16, freq: 880, type: 'square', vol: 0.42 }), tone({ dur: 0.16, freq: 660, type: 'square', vol: 0.42 }), tone({ dur: 0.16, freq: 880, type: 'square', vol: 0.42 }), tone({ dur: 0.28, freq: 660, type: 'square', vol: 0.42, release: 0.2 })));

// airhorn: saw blast
save('airhorn.wav', mix(tone({ dur: 0.85, freq: 220, type: 'saw', vol: 0.5, vibrato: 8, release: 0.12 }), tone({ dur: 0.85, freq: 277, type: 'saw', vol: 0.35, vibrato: 8, release: 0.12 })));

// sparkle: shimmering arpeggio
save('sparkle.wav', mix(at(tone({ dur: 0.3, freq: 1568, type: 'sine', vol: 0.4, release: 0.25 }), 0), at(tone({ dur: 0.3, freq: 2093, type: 'sine', vol: 0.4, release: 0.25 }), 0.07), at(tone({ dur: 0.4, freq: 2637, type: 'sine', vol: 0.45, release: 0.35 }), 0.14), at(tone({ dur: 0.5, freq: 3136, type: 'sine', vol: 0.5, release: 0.45 }), 0.22)));

// whoosh: noise sweep
save('whoosh.wav', noise({ dur: 0.55, vol: 0.6, sweepLP: 1, attack: 0.06, release: 0.12 }));

// success: major arpeggio C5-E5-G5-C6
save('success.wav', concat(tone({ dur: 0.13, freq: 523.25, type: 'triangle', vol: 0.55 }), tone({ dur: 0.13, freq: 659.25, type: 'triangle', vol: 0.55 }), tone({ dur: 0.13, freq: 783.99, type: 'triangle', vol: 0.55 }), tone({ dur: 0.42, freq: 1046.5, type: 'triangle', vol: 0.6, release: 0.34 })));

console.log('presets →', OUT);
