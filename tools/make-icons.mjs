/* Render app/tray icons from the vector source (PNG + ICO). */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';
import pngToIco from 'png-to-ico';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const svg = readFileSync(join(ROOT, 'app', 'icons-src', 'icon.svg'), 'utf8');
const OUT = join(ROOT, 'app', 'icons');
mkdirSync(OUT, { recursive: true });

function render(width) {
  const resvg = new Resvg(svg, { fitTo: { mode: 'width', value: width } });
  return resvg.render().asPng();
}

const png512 = render(512);
const png256 = render(256);
writeFileSync(join(OUT, 'icon.png'), png512);
writeFileSync(join(OUT, 'tray.png'), render(64));

const ico = await pngToIco([png256, render(48), render(32), render(16)]);
writeFileSync(join(OUT, 'icon.ico'), ico);

console.log('icons written to app/icons/: icon.png (512), icon.ico, tray.png (64)');
