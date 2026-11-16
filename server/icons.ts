/* Resolve TeamSpeak 3 server icons from the client's on-disk cache.
 *
 * The TS3 client caches server icons under
 *   %APPDATA%\TS3Client\cache\<server-hash>\icons\icon_<id>   (Windows)
 *   ~/.ts3client/cache/<server-hash>/icons/icon_<id>          (Linux)
 * Older icons are TGA (24/32bpp, optional RLE) — convert them to PNG.
 * Newer icons are PNG/WebP — served as-is.
 */
import { readdirSync, readFileSync, existsSync, statSync } from 'node:fs';
import { homedir, platform } from 'node:os';
import { join } from 'node:path';
import { deflateSync } from 'node:zlib';

export interface IconFile {
  data: Buffer;
  mime: string;
}

const memoryCache = new Map<number, IconFile | null>();
let cacheDirs: string[] | null = null;

function ts3CacheRoots(): string[] {
  const roots: string[] = [];
  if (platform() === 'win32') {
    roots.push(join(process.env['APPDATA'] ?? join(homedir(), 'AppData', 'Roaming'), 'TS3Client', 'cache'));
  } else {
    roots.push(join(homedir(), '.ts3client', 'cache'));
    roots.push(join(homedir(), 'snap', 'teamspeak3', 'current', '.ts3client', 'cache'));
    roots.push(join(homedir(), '.var', 'app', 'com.teamspeak.TeamSpeak3', 'config', 'ts3client', 'cache'));
  }
  return roots.filter((r) => existsSync(r));
}

function iconDirs(): string[] {
  if (cacheDirs) return cacheDirs;
  cacheDirs = [];
  for (const root of ts3CacheRoots()) {
    cacheDirs.push(join(root, 'icons'));
    try {
      for (const entry of readdirSync(root)) {
        const sub = join(root, entry, 'icons');
        if (existsSync(sub)) cacheDirs.push(sub);
      }
    } catch {
      /* ignore */
    }
  }
  return cacheDirs;
}

/** Look up a cached server icon; returns PNG/WebP bytes or null. */
export function findTs3Icon(iconId: number): IconFile | null {
  if (memoryCache.has(iconId)) return memoryCache.get(iconId) ?? null;
  let result: IconFile | null = null;
  for (const dir of iconDirs()) {
    const path = join(dir, `icon_${iconId}`);
    try {
      if (!statSync(path).isFile()) continue;
      const raw = readFileSync(path);
      if (raw.length < 12) continue;
      result = classify(raw);
      if (result) break;
    } catch {
      /* try next */
    }
  }
  memoryCache.set(iconId, result);
  return result;
}

function classify(raw: Buffer): IconFile | null {
  // PNG
  if (raw[0] === 0x89 && raw[1] === 0x50 && raw[2] === 0x4e && raw[3] === 0x47) {
    return { data: raw, mime: 'image/png' };
  }
  // WebP (RIFF....WEBP)
  if (raw.toString('ascii', 0, 4) === 'RIFF' && raw.toString('ascii', 8, 12) === 'WEBP') {
    return { data: raw, mime: 'image/webp' };
  }
  // TGA → PNG
  const png = tgaToPng(raw);
  return png ? { data: png, mime: 'image/png' } : null;
}

/* ---------------- minimal TGA decoder (24/32bpp, raw or RLE) ---------------- */

function tgaToPng(buf: Buffer): Buffer | null {
  try {
    if (buf.length < 18) return null;
    const idLength = buf[0] ?? 0;
    const colorMapType = buf[1] ?? 0;
    const imageType = buf[2] ?? 0; // 2 = uncompressed truecolor, 10 = RLE truecolor
    if (colorMapType !== 0 || (imageType !== 2 && imageType !== 10)) return null;
    const width = buf.readUInt16LE(12);
    const height = buf.readUInt16LE(14);
    const bpp = buf[16] ?? 0;
    const descriptor = buf[17] ?? 0;
    if (!width || !height || (bpp !== 24 && bpp !== 32)) return null;

    const bytes = bpp / 8;
    const pixels = Buffer.alloc(width * height * 4);
    let src = 18 + idLength;
    let dst = 0;

    const putPixel = () => {
      if (src + bytes > buf.length || dst + 4 > pixels.length) throw new Error('truncated');
      // TGA stores BGR(A) — write RGBA
      pixels[dst++] = buf[src + 2] ?? 0; // R
      pixels[dst++] = buf[src + 1] ?? 0; // G
      pixels[dst++] = buf[src] ?? 0; // B
      pixels[dst++] = bytes === 4 ? buf[src + 3] ?? 255 : 255;
      src += bytes;
    };

    const total = width * height;
    if (imageType === 2) {
      for (let i = 0; i < total; i++) putPixel();
    } else {
      let count = 0;
      while (count < total) {
        const packet = buf[src++] ?? 0;
        const n = (packet & 0x7f) + 1;
        if (packet & 0x80) {
          // RLE packet: one pixel repeated n times
          const b = buf[src] ?? 0, g = buf[src + 1] ?? 0, r = buf[src + 2] ?? 0, a = bytes === 4 ? buf[src + 3] ?? 255 : 255;
          src += bytes;
          for (let i = 0; i < n; i++) {
            pixels[dst++] = r; pixels[dst++] = g; pixels[dst++] = b; pixels[dst++] = a;
          }
        } else {
          for (let i = 0; i < n; i++) putPixel();
        }
        count += n;
      }
    }

    // TGA is usually bottom-up (bit 5 of descriptor = 0)
    const flipVertical = (descriptor & 0x20) === 0;
    const rgba = Buffer.alloc(width * height * 4);
    for (let y = 0; y < height; y++) {
      const srcRow = flipVertical ? height - 1 - y : y;
      pixels.copy(rgba, y * width * 4, srcRow * width * 4, (srcRow + 1) * width * 4);
    }

    return encodePng(rgba, width, height);
  } catch {
    return null;
  }
}

/* ---------------- minimal PNG encoder ---------------- */

function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i]!;
    for (let k = 0; k < 8; k++) c = c & 1 ? (c >>> 1) ^ 0xedb88320 : c >>> 1;
  }
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Buffer): Buffer {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'ascii');
  data.copy(out, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}

function encodePng(rgba: Buffer, width: number, height: number): Buffer {
  // filter byte 0 per scanline
  const raw = Buffer.alloc(height * (1 + width * 4));
  for (let y = 0; y < height; y++) {
    raw[y * (1 + width * 4)] = 0;
    rgba.copy(raw, y * (1 + width * 4) + 1, y * width * 4, (y + 1) * width * 4);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}
