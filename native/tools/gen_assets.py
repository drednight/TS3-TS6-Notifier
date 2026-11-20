#!/usr/bin/env python3
"""Generate native overlay assets (run from repo root or native/):

  PYTHONPATH=/tmp/pylibs python3 native/tools/gen_assets.py

Outputs (committed, regenerated here):
  native/assets/icons.bin  — 18 class icons, 64x64 RGBA (from public/img/classes/*.webp)
  native/assets/font.bin   — DejaVuSans(-Bold) glyph atlases, sizes 8..30, RLE alpha

Keep the class table below in sync with public/js/features.js CLASS_META.
"""
import os
import struct
import sys

from PIL import Image, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, 'native', 'assets')
CLASSES_DIR = os.path.join(ROOT, 'public', 'img', 'classes')
FONT_R = '/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf'
FONT_B = '/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf'

# key -> icon file (CLASS_META in public/js/features.js)
CLASS_ICONS = [
    ('kanglong', 'canglong.webp'), ('blood', 'dk.webp'), ('werewolf', 'tank.webp'),
    ('warrior', 'var.webp'), ('assassin', 'sin.webp'), ('gunslinger', 'gan.webp'),
    ('guardian', 'sik.webp'), ('priest', 'prist.webp'), ('archer', 'luk.webp'),
    ('paladin', 'pal.webp'), ('shaman', 'sham.webp'), ('bard', 'bard.webp'),
    ('mystic', 'mist.webp'), ('druid', 'dru.webp'), ('monkey', 'mk.webp'),
    ('mage', 'mag.webp'), ('reaper', 'kosa.webp'), ('ghost', 'gost.webp'),
]

# glyphs: ASCII + Latin-1 + Latin Ext-A + Greek + Cyrillic(+ext) + symbols
# (ники пользователей на разных языках: «Σκuλλα», «Łukasz», «Әли», «İlhamə»)
CHARS = []
for cp in range(0x20, 0x7F):            # ASCII
    CHARS.append(chr(cp))
for cp in range(0xA0, 0x100):           # Latin-1 (é ü ñ ç × ÷ ° © …)
    CHARS.append(chr(cp))
for cp in range(0x100, 0x180):          # Latin Ext-A (ł š č ı ş ă ő ž …)
    CHARS.append(chr(cp))
for cp in range(0x384, 0x3A2):          # Greek: ΄΅ΆΈ-Ρ (включая тонусы)
    CHARS.append(chr(cp))
for cp in range(0x3A3, 0x3D0):          # Greek: Σ-ώ (включая ς)
    CHARS.append(chr(cp))
for cp in range(0x400, 0x460):          # Cyrillic Ѐ-џ (Ё, є і ї ў …)
    CHARS.append(chr(cp))
for cp in range(0x490, 0x4C0):          # Ґґ Ғғ Ққ Ңң Үү Ұұ Ҳҳ Һһ Җҗ …
    CHARS.append(chr(cp))
for cp in range(0x4D0, 0x500):          # Ӓӓ Ӗӗ Әә Ӣӣ Өө Ӱӱ Ӹӹ …
    CHARS.append(chr(cp))
CHARS += list('‐‑‒–—―‖‘’‚‛“”„‟†‡‣․…‰′″‴‹›‼‽⁄')          # пунктуация
for cp in range(0x20A0, 0x20C0):        # валюты (₽ € £ ¥ ₩ …)
    CHARS.append(chr(cp))
CHARS += list('Əə')                                     # азербайджанская ə
CHARS += list('№™℃℉⅓⅔⅕⅖⅗⅘⅙⅚⅛⅜⅝⅞')                              # буквы/дроби
CHARS += list('←↑→↓↔↕⇒⇔∞≈≠≡≤≥≪≫±×÷⋅°∙√∑∏∂∅∆∇∈∉∋∩∪∧∨∫∴∵⊕⊗⊥')  # математика
CHARS += list('■□▢▣▪▫▬▭▮▰▱▲△▼▽◀▷◆◇◈◉○◌●◐◑◒◓◔◕')            # геометрия
CHARS += list('☀☁☂☃☄★☆☇☈☉☎☑☒☓☕☘☠☡☢☣☪☭☺☻☼♠♣♥♦♪♫♬♭♮♯⚡⚪⚫')  # символы
CHARS += list('✂✈✉✌✍✎✏✓✔✕✖✗✘✜✝✦✧✨✩✪✫✬✭✮✯✰✱✲✳✴✵✶✷✸✹✺✻✼✽✾✿❀❁❂❃❄❅❆❇❈❉❊❋❌❍❓❔❕❖❗❛❜❝❞')  # дингбаты
CHARS += list('〈〉《》「」『』【】〔〕〜・')                        # CJK-пунктуация
CHARS = sorted(dict.fromkeys(CHARS))  # unique + sorted by cp (binary search)

ICON_SIZE = 64
SIZES = [8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 26, 28, 30]
PAD = 2


def rle_encode(data: bytes) -> bytes:
    out = bytearray()
    i = 0
    n = len(data)
    while i < n:
        v = data[i]
        run = 1
        while i + run < n and data[i + run] == v and run < 255:
            run += 1
        out.append(run)
        out.append(v)
        i += run
    return bytes(out)


def gen_icons():
    blob = bytearray(b'TSNI1\0\0\0')
    blob += struct.pack('<I', len(CLASS_ICONS))
    for key, fname in CLASS_ICONS:
        path = os.path.join(CLASSES_DIR, fname)
        img = Image.open(path).convert('RGBA')
        # fit into ICON_SIZE box, keep aspect, premultiplied-safe paste on transparent
        img.thumbnail((ICON_SIZE, ICON_SIZE), Image.LANCZOS)
        canvas = Image.new('RGBA', (ICON_SIZE, ICON_SIZE), (0, 0, 0, 0))
        canvas.paste(img, ((ICON_SIZE - img.width) // 2, (ICON_SIZE - img.height) // 2))
        raw = canvas.tobytes()
        name = key.encode('ascii')[:15].ljust(16, b'\0')
        blob += name + struct.pack('<HHI', ICON_SIZE, ICON_SIZE, len(raw)) + raw
        print(f'  icon {key:10s} <- {fname}')
    os.makedirs(OUT, exist_ok=True)
    with open(os.path.join(OUT, 'icons.bin'), 'wb') as f:
        f.write(blob)
    print(f'icons.bin: {len(blob)} bytes')


def glyph_bitmap(font, ch, size):
    """Rasterize one glyph: returns (w, h, alpha_bytes, bearing_x, top, advance)."""
    tmp = Image.new('L', (size * 3, size * 3), 0)
    from PIL import ImageDraw
    d = ImageDraw.Draw(tmp)
    ox, oy = size, size * 2  # pen position (baseline left)
    try:
        bbox = d.textbbox((ox, oy), ch, font=font, anchor='ls')
    except Exception:
        return None
    if bbox is None:
        return None
    x0, y0, x1, y1 = bbox
    if x1 <= x0 or y1 <= y0:
        # space or invisible: advance only
        return (0, 0, b'', 0, 0, font.getlength(ch))
    d.text((ox, oy), ch, font=font, fill=255, anchor='ls')
    w, h = x1 - x0, y1 - y0
    crop = tmp.crop((x0, y0, x1, y1))
    return (w, h, crop.tobytes(), x0 - ox, y0 - oy, font.getlength(ch))


def gen_font():
    os.makedirs(OUT, exist_ok=True)
    blob = bytearray(b'TSNF1\0\0\0')
    sections = []
    for bold, path in ((0, FONT_R), (1, FONT_B)):
        for size in SIZES:
            font = ImageFont.truetype(path, size)
            glyphs = []
            for ch in CHARS:
                g = glyph_bitmap(font, ch, size)
                if g is None:
                    continue
                glyphs.append((ch, g))
            # shelf-pack into an atlas
            cols = 16
            cell_w = max((g[0] for _, g in glyphs), default=1) + 1
            cell_h = max((g[1] for _, g in glyphs), default=1) + 1
            rows = (len(glyphs) + cols - 1) // cols
            atlas_w, atlas_h = cols * cell_w, rows * cell_h
            atlas = bytearray(atlas_w * atlas_h)
            table = []
            for idx, (ch, (w, h, alpha, bx, top, adv)) in enumerate(glyphs):
                cx = (idx % cols) * cell_w
                cy = (idx // cols) * cell_h
                for yy in range(h):
                    row = alpha[yy * w:(yy + 1) * w]
                    off = (cy + yy) * atlas_w + cx
                    atlas[off:off + w] = row
                table.append((ord(ch), cx, cy, w, h, bx, top, adv))
            raw = bytes(atlas)
            comp = rle_encode(raw)
            sec = struct.pack('<IIII', size, atlas_w, atlas_h, len(table))
            for cp, cx, cy, w, h, bx, top, adv in table:
                sec += struct.pack('<IHHHHhhf', cp, cx, cy, w, h, bx, top, adv)
            sec += comp
            sections.append(sec)
            print(f'  font bold={bold} size={size:2d}: {len(table)} glyphs, atlas {atlas_w}x{atlas_h}, rle {len(comp)}')
    # header: magic + count + bold flags/sizes rebuilt from sections order
    header = struct.pack('<I', len(sections))
    # section order = bold 0 sizes.., bold 1 sizes.. — encode bold flag into section
    # (rewrite sections with bold bit: we stored size only; patch: high bit of size)
    out_secs = bytearray()
    i = 0
    for bold in (0, 1):
        for _ in SIZES:
            s = sections[i]
            size_val = struct.unpack_from('<I', s, 0)[0] | (0x80000000 if bold else 0)
            out_secs += struct.pack('<I', size_val) + s[4:]
            i += 1
    with open(os.path.join(OUT, 'font.bin'), 'wb') as f:
        f.write(bytes(blob) + header + bytes(out_secs))
    total = os.path.getsize(os.path.join(OUT, 'font.bin'))
    print(f'font.bin: {total} bytes')


if __name__ == '__main__':
    print('generating icons...')
    gen_icons()
    print('generating font...')
    gen_font()
