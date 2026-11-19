//! Software rasterizer: straight-alpha RGBA buffer, SDF-AA shapes, gradients,
//! scaled image blits. Used by the native overlay window (and QA dumps).

use crate::theme::Rgba;

pub struct Buf {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>, // RGBA
    clip: Option<(i32, i32, i32, i32)>,
}

impl Buf {
    pub fn new(w: i32, h: i32) -> Buf {
        Buf { w, h, px: vec![0u8; (w * h * 4) as usize], clip: None }
    }

    /// Restrict drawing to a rect (scrollable panes). One level; `pop_clip` restores.
    pub fn set_clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let x0 = x.floor().max(0.0) as i32;
        let y0 = y.floor().max(0.0) as i32;
        let x1 = (x + w).ceil().min(self.w as f32) as i32;
        let y1 = (y + h).ceil().min(self.h as f32) as i32;
        self.clip = Some((x0, y0, x1, y1));
    }

    pub fn clear_clip(&mut self) {
        self.clip = None;
    }

    #[inline]
    pub fn blend_px(&mut self, x: i32, y: i32, c: Rgba) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || c.0[3] == 0 {
            return;
        }
        if let Some((cx0, cy0, cx1, cy1)) = self.clip {
            if x < cx0 || y < cy0 || x >= cx1 || y >= cy1 {
                return;
            }
        }
        let i = ((y * self.w + x) * 4) as usize;
        let a = c.0[3] as u32;
        let inv = 255 - a;
        self.px[i] = ((c.0[0] as u32 * a + self.px[i] as u32 * inv) / 255) as u8;
        self.px[i + 1] = ((c.0[1] as u32 * a + self.px[i + 1] as u32 * inv) / 255) as u8;
        self.px[i + 2] = ((c.0[2] as u32 * a + self.px[i + 2] as u32 * inv) / 255) as u8;
        self.px[i + 3] = (a + self.px[i + 3] as u32 * inv / 255) as u8;
    }

    pub fn clear(&mut self) {
        self.px.fill(0);
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
        if c.0[3] == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let x0 = x.floor().max(0.0) as i32;
        let y0 = y.floor().max(0.0) as i32;
        let x1 = (x + w).ceil().min(self.w as f32) as i32;
        let y1 = (y + h).ceil().min(self.h as f32) as i32;
        for yy in y0..y1 {
            for xx in x0..x1 {
                self.blend_px(xx, yy, c);
            }
        }
    }

    /// Vertical gradient (alpha premultiplied interpolation of two colors).
    pub fn fill_rect_vgrad(&mut self, x: f32, y: f32, w: f32, h: f32, top: Rgba, bottom: Rgba) {
        let y0 = y.floor().max(0.0) as i32;
        let y1 = (y + h).ceil().min(self.h as f32) as i32;
        for yy in y0..y1 {
            let t = ((yy as f32 + 0.5 - y) / h.max(1.0)).clamp(0.0, 1.0) as f64;
            let c = crate::theme::mix(top, bottom, t);
            self.fill_rect(x, yy as f32, w, 1.0, c);
        }
    }

    /// Anti-aliased rounded rect via SDF coverage.
    pub fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c: Rgba) {
        if c.0[3] == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
        let x0 = x.floor().max(0.0) as i32;
        let y0 = y.floor().max(0.0) as i32;
        let x1 = (x + w).ceil().min(self.w as f32) as i32;
        let y1 = (y + h).ceil().min(self.h as f32) as i32;
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        let hx = w * 0.5 - r;
        let hy = h * 0.5 - r;
        for yy in y0..y1 {
            for xx in x0..x1 {
                let px = xx as f32 + 0.5 - cx;
                let py = yy as f32 + 0.5 - cy;
                let qx = px.abs() - hx;
                let qy = py.abs() - hy;
                let dx = qx.max(0.0);
                let dy = qy.max(0.0);
                let outside = (dx * dx + dy * dy).sqrt() + qx.max(qy).min(0.0) - r;
                let cov = (0.5 - outside).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_px(xx, yy, c.with_a((c.0[3] as f32 * cov) as u8));
                }
            }
        }
    }

    /// Rounded-rect outline (AA both edges).
    pub fn stroke_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c: Rgba, thickness: f32) {
        if c.0[3] == 0 || thickness <= 0.0 {
            return;
        }
        let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
        let x0 = (x - 2.0).floor().max(0.0) as i32;
        let y0 = (y - 2.0).floor().max(0.0) as i32;
        let x1 = (x + w + 2.0).ceil().min(self.w as f32) as i32;
        let y1 = (y + h + 2.0).ceil().min(self.h as f32) as i32;
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        let hx = w * 0.5 - r;
        let hy = h * 0.5 - r;
        let half = thickness * 0.5;
        for yy in y0..y1 {
            for xx in x0..x1 {
                let px = xx as f32 + 0.5 - cx;
                let py = yy as f32 + 0.5 - cy;
                let qx = px.abs() - hx;
                let qy = py.abs() - hy;
                let dx = qx.max(0.0);
                let dy = qy.max(0.0);
                let outside = (dx * dx + dy * dy).sqrt() + qx.max(qy).min(0.0) - r;
                let cov = ((half + 0.5) - outside.abs()).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_px(xx, yy, c.with_a((c.0[3] as f32 * cov) as u8));
                }
            }
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        if c.0[3] == 0 || r <= 0.0 {
            return;
        }
        let x0 = (cx - r - 1.0).floor().max(0.0) as i32;
        let y0 = (cy - r - 1.0).floor().max(0.0) as i32;
        let x1 = (cx + r + 1.0).ceil().min(self.w as f32) as i32;
        let y1 = (cy + r + 1.0).ceil().min(self.h as f32) as i32;
        for yy in y0..y1 {
            for xx in x0..x1 {
                let px = xx as f32 + 0.5 - cx;
                let py = yy as f32 + 0.5 - cy;
                let outside = (px * px + py * py).sqrt() - r;
                let cov = (0.5 - outside).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_px(xx, yy, c.with_a((c.0[3] as f32 * cov) as u8));
                }
            }
        }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba, thickness: f32) {
        self.stroke_round_rect(cx - r, cy - r, r * 2.0, r * 2.0, r, c, thickness);
    }

    /// Thick line segment (AA), for pictograms.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, thickness: f32, c: Rgba) {
        let half = thickness * 0.5;
        let min_x = x0.min(x1) - half - 1.0;
        let min_y = y0.min(y1) - half - 1.0;
        let max_x = x0.max(x1) + half + 1.0;
        let max_y = y0.max(y1) + half + 1.0;
        let px0 = min_x.floor().max(0.0) as i32;
        let py0 = min_y.floor().max(0.0) as i32;
        let px1 = max_x.ceil().min(self.w as f32) as i32;
        let py1 = max_y.ceil().min(self.h as f32) as i32;
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len2 = dx * dx + dy * dy;
        for yy in py0..py1 {
            for xx in px0..px1 {
                let px = xx as f32 + 0.5;
                let py = yy as f32 + 0.5;
                let t = if len2 <= 0.0 {
                    0.0
                } else {
                    (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0)
                };
                let ex = px - (x0 + dx * t);
                let ey = py - (y0 + dy * t);
                let d = (ex * ex + ey * ey).sqrt() - half;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_px(xx, yy, c.with_a((c.0[3] as f32 * cov) as u8));
                }
            }
        }
    }

    /// Soft dark shadow around a rounded rect (concentric AA rings).
    pub fn shadow_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, c: Rgba) {
        let rings = (blur / 2.0).ceil().max(1.0) as i32;
        for i in (0..rings).rev() {
            let grow = (i as f32 + 1.0) * blur / rings as f32;
            let a = (c.0[3] as f32 * (1.0 - i as f32 / rings as f32) * 0.5).max(1.0) as u8;
            self.stroke_round_rect(
                x - grow,
                y - grow,
                w + grow * 2.0,
                h + grow * 2.0,
                radius + grow,
                c.with_a(a),
                blur / rings as f32 + 1.0,
            );
        }
    }

    /// CSS-like opacity over a backdrop: lerp colors+alpha toward `towards`.
    /// Blit another buffer with straight-alpha blending (screen-layer mode).
    pub fn blit_buf(&mut self, dx: i32, dy: i32, src: &Buf) {
        for y in 0..src.h {
            for x in 0..src.w {
                let idx = ((y * src.w + x) * 4) as usize;
                let a = src.px[idx + 3];
                if a == 0 {
                    continue;
                }
                let c = crate::theme::Rgba([src.px[idx], src.px[idx + 1], src.px[idx + 2], a]);
                self.blend_px(dx + x, dy + y, c);
            }
        }
    }

    pub fn fade_rect(&mut self, x: f32, y: f32, w: f32, h: f32, f: f32, towards: Rgba) {
        let f = f.clamp(0.0, 1.0);
        if f >= 1.0 {
            return;
        }
        let x0 = x.floor().max(0.0) as i32;
        let y0 = y.floor().max(0.0) as i32;
        let x1 = (x + w).ceil().min(self.w as f32) as i32;
        let y1 = (y + h).ceil().min(self.h as f32) as i32;
        for yy in y0..y1 {
            for xx in x0..x1 {
                let i = ((yy * self.w + xx) * 4) as usize;
                for ch in 0..4 {
                    let v = self.px[i + ch] as f32 * f + towards.0[ch] as f32 * (1.0 - f);
                    self.px[i + ch] = v.round() as u8;
                }
            }
        }
    }

    /// Bilinear-scaled RGBA blit with optional circle clip (for avatars/icons).
    pub fn blit_rgba(&mut self, dx: f32, dy: f32, dw: f32, dh: f32, src: &[u8], sw: i32, sh: i32, circle_clip: bool) {
        if dw <= 0.0 || dh <= 0.0 {
            return;
        }
        let x0 = dx.floor().max(0.0) as i32;
        let y0 = dy.floor().max(0.0) as i32;
        let x1 = (dx + dw).ceil().min(self.w as f32) as i32;
        let y1 = (dy + dh).ceil().min(self.h as f32) as i32;
        let cx = dx + dw * 0.5;
        let cy = dy + dh * 0.5;
        let cr = dw.min(dh) * 0.5;
        for yy in y0..y1 {
            for xx in x0..x1 {
                let px = xx as f32 + 0.5;
                let py = yy as f32 + 0.5;
                if circle_clip {
                    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - cr;
                    if d > 0.5 {
                        continue;
                    }
                }
                let sx = ((px - dx) / dw) * sw as f32 - 0.5;
                let sy = ((py - dy) / dh) * sh as f32 - 0.5;
                let sample = |ix: i32, iy: i32| -> [f32; 4] {
                    if ix < 0 || iy < 0 || ix >= sw || iy >= sh {
                        return [0.0; 4];
                    }
                    let i = ((iy * sw + ix) * 4) as usize;
                    [
                        src[i] as f32,
                        src[i + 1] as f32,
                        src[i + 2] as f32,
                        src[i + 3] as f32,
                    ]
                };
                let fx = sx.floor();
                let fy = sy.floor();
                let tx = sx - fx;
                let ty = sy - fy;
                let bx = fx as i32;
                let by = fy as i32;
                let mut out = [0.0f32; 4];
                for i in 0..4 {
                    out[i] = sample(bx, by)[i] * (1.0 - tx) * (1.0 - ty)
                        + sample(bx + 1, by)[i] * tx * (1.0 - ty)
                        + sample(bx, by + 1)[i] * (1.0 - tx) * ty
                        + sample(bx + 1, by + 1)[i] * tx * ty;
                }
                let mut a = out[3];
                if circle_clip {
                    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - cr;
                    a *= (0.5 - d).clamp(0.0, 1.0);
                }
                if a <= 1.0 {
                    continue;
                }
                self.blend_px(xx, yy, Rgba([out[0] as u8, out[1] as u8, out[2] as u8, a.min(255.0) as u8]));
            }
        }
    }
}

/// Minimal PNG writer (stored deflate) for QA frames and tests.
pub fn write_png(path: &str, buf: &Buf) -> std::io::Result<()> {
    fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
        let len = (data.len() as u32).to_be_bytes();
        out.extend_from_slice(&len);
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        let mut crc = 0xffff_ffffu32;
        for b in tag.iter().chain(data.iter()) {
            crc ^= *b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
            }
        }
        out.extend_from_slice(&(!crc).to_be_bytes());
    }
    fn adler(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for byte in data {
            a = (a + *byte as u32) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(buf.w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(buf.h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    // raw scanlines with filter byte 0
    let mut raw = Vec::with_capacity(((buf.w * 4 + 1) * buf.h) as usize);
    for y in 0..buf.h {
        raw.push(0);
        raw.extend_from_slice(&buf.px[(y * buf.w * 4) as usize..((y + 1) * buf.w * 4) as usize]);
    }
    // zlib: stored deflate blocks (max 65535 per block)
    let mut z = vec![0x78, 0x01];
    let total = raw.len();
    let mut off = 0;
    while off < total {
        let n = (total - off).min(65535);
        let final_block = off + n >= total;
        z.push(if final_block { 1 } else { 0 });
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[off..off + n]);
        off += n;
    }
    z.extend_from_slice(&adler(&raw).to_be_bytes());
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blends_src_over() {
        let mut b = Buf::new(4, 4);
        b.fill_rect(0.0, 0.0, 4.0, 4.0, Rgba::rgb(255, 0, 0));
        b.fill_rect(0.0, 0.0, 4.0, 4.0, Rgba([0, 0, 255, 128]));
        let i = (0 * 4 + 0) * 4;
        assert!(b.px[i] > 100 && b.px[i + 2] > 100, "mixed {:?}", &b.px[i..i + 4]);
    }

    #[test]
    fn round_rect_center_opaque_edges_soft() {
        let mut b = Buf::new(32, 32);
        b.round_rect(4.0, 4.0, 24.0, 24.0, 6.0, Rgba::rgb(0, 255, 0));
        let c = (16 * 32 + 16) * 4;
        assert_eq!(b.px[c + 1], 255);
        // corner pixel (rounded away) stays transparent
        let corner = (4 * 32 + 4) * 4;
        assert!(b.px[corner + 3] < 40, "corner alpha {}", b.px[corner + 3]);
    }

    #[test]
    fn clipped_write_is_safe() {
        let mut b = Buf::new(8, 8);
        b.fill_rect(-5.0, -5.0, 30.0, 30.0, Rgba::rgb(1, 2, 3));
        b.circle(-10.0, -10.0, 30.0, Rgba::rgb(1, 2, 3));
        b.line(-5.0, -5.0, 20.0, 20.0, 3.0, Rgba::rgb(1, 2, 3));
    }

    #[test]
    fn png_roundtrip_has_signature() {
        let mut b = Buf::new(4, 4);
        b.fill_rect(0.0, 0.0, 4.0, 4.0, Rgba::rgb(255, 0, 0));
        let path = std::env::temp_dir().join("tsn_test.png");
        write_png(path.to_str().unwrap(), &b).unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(&raw[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    }
}
