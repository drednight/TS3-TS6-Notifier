//! Стиль «Сонар»: радарная развёртка с затухающим шлейфом, дальномерные кольца,
//! сонарные «пинги» из центра и отметки-блипы участников (у говорящего — ряби).
//!
//! Модуль самодостаточный (только std): рисует ФОН панели в premultiplied
//! ARGB (0xAARRGGBB — в памяти это BGRA, как ждёт DIB для UpdateLayeredWindow).
//! Строки (аватары, ники) рисует существующий рендерер поверх. Всё обрезано по
//! скруглённой форме панели — никаких квадратных контуров.
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy)]
pub struct SonarParams {
    pub accent: [u8; 3],  // R, G, B
    pub panel_alpha: f32, // 0..1
    pub corner: f32,      // радиус скругления, px
    pub period_ms: u32,   // оборот луча
    pub trail: f32,       // длина шлейфа, доли оборота
    pub ping_ms: u32,     // период пингов из центра
}

impl Default for SonarParams {
    fn default() -> Self {
        Self { accent: [57, 255, 140], panel_alpha: 0.82, corner: 18.0, period_ms: 4200, trail: 0.22, ping_ms: 2600 }
    }
}

#[derive(Clone, Copy)]
pub struct Blip {
    pub id: u64,
    pub talking: bool,
    pub talk_since_ms: u64,
}

#[derive(Default)]
pub struct Sonar {
    w: usize,
    h: usize,
    key: u64,
    ang: Vec<u16>,  // угол пикселя относительно центра, 0..65535 = оборот
    rad: Vec<f32>,  // расстояние до центра
    mask: Vec<u8>,  // покрытие скруглённой панели
    base: Vec<u32>, // статический слой: фон, сетка, кольца (premultiplied)
}

fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Добавляет свет (аддитивно) к premultiplied-пикселю; m — покрытие панели.
fn add_light(px: &mut u32, acc: [u8; 3], l: f32, m: u8) {
    let l = l.clamp(0.0, 1.0) * (m as f32 / 255.0);
    if l <= 0.0 {
        return;
    }
    let a0 = (*px >> 24) as f32;
    let a = a0.max(l * 255.0);
    let ch = |shift: u32, c: u8| -> u32 {
        let v = ((*px >> shift) & 0xFF) as f32 + c as f32 * l;
        v.min(a) as u32
    };
    *px = ((a as u32) << 24) | (ch(16, acc[0]) << 16) | (ch(8, acc[1]) << 8) | ch(0, acc[2]);
}

impl Sonar {
    pub fn new() -> Self {
        Self::default()
    }

    fn prepare(&mut self, w: usize, h: usize, p: &SonarParams) {
        let key = (w as u64) << 40
            ^ (h as u64) << 24
            ^ (p.panel_alpha.to_bits() as u64)
            ^ ((p.corner.to_bits() as u64) << 8)
            ^ u64::from(p.accent[0]) << 48
            ^ u64::from(p.accent[1]) << 52
            ^ u64::from(p.accent[2]) << 56;
        if key == self.key && self.w == w && self.h == h && !self.base.is_empty() {
            return;
        }
        self.w = w;
        self.h = h;
        self.key = key;
        let n = w * h;
        self.ang = vec![0; n];
        self.rad = vec![0.0; n];
        self.mask = vec![0; n];
        self.base = vec![0; n];
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let maxr = cx.hypot(cy);
        let spacing = (w.min(h) as f32 * 0.22).max(24.0);
        let corner = p.corner.min(cx.min(cy));
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                self.ang[i] = (((dy.atan2(dx) / TAU + 0.5) * 65536.0) as u32 & 0xFFFF) as u16;
                let r = dx.hypot(dy);
                self.rad[i] = r;
                // SDF скруглённого прямоугольника → покрытие с 1 px сглаживанием
                let (qx, qy) = (dx.abs() - (cx - corner), dy.abs() - (cy - corner));
                let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - corner;
                let m = (0.5 - d).clamp(0.0, 1.0);
                self.mask[i] = (m * 255.0) as u8;
                // фон: тёмный с лёгким свечением от центра
                let g = 0.05 * (1.0 - r / maxr).max(0.0);
                let bg = [
                    4.0 + p.accent[0] as f32 * g,
                    14.0 + p.accent[1] as f32 * g,
                    12.0 + p.accent[2] as f32 * g,
                ];
                // сетка: кольца дальности и перекрестие
                let rm = r % spacing;
                let ring_d = rm.min(spacing - rm);
                let ring = if r > spacing * 0.5 { (1.0 - ring_d / 1.2).max(0.0) * 0.16 } else { 0.0 };
                let cross = if dx.abs() < 0.6 || dy.abs() < 0.6 { 0.09 } else { 0.0 };
                let l: f32 = ring.max(cross);
                let a = p.panel_alpha.max(l);
                let c = |b: f32, acc: u8| (b * p.panel_alpha + acc as f32 * l).min(a * 255.0) * m;
                self.base[i] = (((a * 255.0 * m) as u32) << 24)
                    | ((c(bg[0], p.accent[0]) as u32) << 16)
                    | ((c(bg[1], p.accent[1]) as u32) << 8)
                    | (c(bg[2], p.accent[2]) as u32);
            }
        }
    }

    /// Рисует фон «Сонара» в `dst` (w*h пикселей, premultiplied ARGB).
    pub fn render(&mut self, dst: &mut [u32], w: usize, h: usize, t_ms: u64, p: &SonarParams, blips: &[Blip]) {
        if w < 16 || h < 16 || dst.len() < w * h {
            return;
        }
        self.prepare(w, h, p);
        dst[..w * h].copy_from_slice(&self.base);
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let maxr = cx.hypot(cy);

        let period = p.period_ms.max(500) as u64;
        let beam = ((t_ms % period) as f32 / period as f32 * 65536.0) as u32 as u16;
        let trail_q = ((p.trail.clamp(0.05, 0.6)) * 65536.0) as u32;
        let mut fall = [0f32; 256];
        for (i, f) in fall.iter_mut().enumerate() {
            *f = (-3.0 * i as f32 / 255.0).exp();
        }
        let ping = p.ping_ms.max(600) as u64;
        let ph1 = (t_ms % ping) as f32 / ping as f32;
        let ph2 = (ph1 + 0.5) % 1.0;
        let pings = [(ph1 * maxr, (1.0 - ph1).powf(1.6) * 0.5), (ph2 * maxr, (1.0 - ph2).powf(1.6) * 0.28)];

        for i in 0..w * h {
            let m = self.mask[i];
            if m == 0 {
                continue;
            }
            let d = beam.wrapping_sub(self.ang[i]) as u32; // 0 на луче, растёт позади него
            let mut l = 0.0f32;
            if d < trail_q {
                l = fall[(d * 255 / trail_q) as usize] * 0.55;
            }
            if d < 520 {
                l = l.max((1.0 - d as f32 / 520.0) * 0.95); // яркая кромка луча
            }
            for &(pr, pa) in &pings {
                let dist = (self.rad[i] - pr).abs();
                if dist < 6.0 {
                    l += (1.0 - dist / 6.0).powi(2) * pa;
                }
            }
            if l > 0.003 {
                add_light(&mut dst[i], p.accent, l, m);
            }
        }

        // блипы участников: яркие, когда луч только прошёл; говорящий — ряби
        for b in blips {
            let h1 = mix64(b.id);
            let h2 = mix64(h1);
            let theta = ((h1 & 0xFFFF) as f32 / 65536.0 - 0.5) * TAU;
            let rf = 0.22 + 0.7 * ((h2 & 0xFFFF) as f32 / 65536.0);
            let x = (cx + theta.cos() * rf * maxr * 0.8).clamp(10.0, w as f32 - 10.0);
            let y = (cy + theta.sin() * rf * maxr * 0.8).clamp(10.0, h as f32 - 10.0);
            let angq = (((theta / TAU + 0.5) * 65536.0) as u32 & 0xFFFF) as u16;
            let since_pass = beam.wrapping_sub(angq) as f32 / 65536.0;
            let glow = if b.talking { 1.0 } else { (-since_pass * 3.2).exp() * 0.9 + 0.08 };
            let size = 2.2 + glow * 1.6;
            let ripple = if b.talking {
                let ph = (t_ms.saturating_sub(b.talk_since_ms) % 1100) as f32 / 1100.0;
                Some((ph * 34.0, (1.0 - ph).powf(1.5) * 0.9))
            } else {
                None
            };
            let reach = if ripple.is_some() { 38 } else { 6 };
            let (x0, x1) = ((x as i32 - reach).max(0), (x as i32 + reach).min(w as i32 - 1));
            let (y0, y1) = ((y as i32 - reach).max(0), (y as i32 + reach).min(h as i32 - 1));
            for py in y0..=y1 {
                for px in x0..=x1 {
                    let i = py as usize * w + px as usize;
                    let dist = (px as f32 + 0.5 - x).hypot(py as f32 + 0.5 - y);
                    let mut l = (1.0 - dist / size).max(0.0).powf(1.5) * glow;
                    if let Some((rr, ra)) = ripple {
                        let rd = (dist - rr).abs();
                        if rd < 1.8 {
                            l += (1.0 - rd / 1.8) * ra;
                        }
                    }
                    if l > 0.003 {
                        add_light(&mut dst[i], p.accent, l, self.mask[i]);
                    }
                }
            }
        }
    }
}

/// Пульс подсветки строки говорящего (0..1) — для рамки/полосы строки.
pub fn talk_pulse(t_ms: u64, talk_since_ms: u64) -> f32 {
    let ph = (t_ms.saturating_sub(talk_since_ms) % 1100) as f32 / 1100.0;
    0.6 + 0.4 * (ph * 2.0 * PI).sin().abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_transparent_and_sweep_moves() {
        let (w, h) = (120, 160);
        let p = SonarParams::default();
        let mut s = Sonar::new();
        let (mut a, mut b) = (vec![0u32; w * h], vec![0u32; w * h]);
        s.render(&mut a, w, h, 0, &p, &[]);
        s.render(&mut b, w, h, p.period_ms as u64 / 2, &p, &[]);
        assert_eq!(a[0] >> 24, 0, "углы вне скруглённой панели прозрачны");
        assert_ne!(a, b, "луч вращается");
        assert!(a[(h / 2) * w + w / 2] >> 24 > 0, "центр панели непрозрачен");
    }

    #[test]
    fn talking_blip_adds_ripple_and_stays_premultiplied() {
        let (w, h) = (120, 160);
        let p = SonarParams::default();
        let mut s = Sonar::new();
        let (mut quiet, mut talk) = (vec![0u32; w * h], vec![0u32; w * h]);
        s.render(&mut quiet, w, h, 500, &p, &[Blip { id: 7, talking: false, talk_since_ms: 0 }]);
        s.render(&mut talk, w, h, 500, &p, &[Blip { id: 7, talking: true, talk_since_ms: 0 }]);
        assert_ne!(quiet, talk);
        for px in &talk {
            let a = px >> 24;
            assert!(((px >> 16) & 0xFF) <= a && ((px >> 8) & 0xFF) <= a && (px & 0xFF) <= a, "premultiplied: rgb <= alpha");
        }
    }
}
