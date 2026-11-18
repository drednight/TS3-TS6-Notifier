//! Overlay palette — resolved from `config.appearance` exactly like
//! `public/css/overlay.css` (theme variables + v1.4.7 user colors).

use crate::config::Appearance;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba([r, g, b, 255])
    }
    pub fn with_a(mut self, a: u8) -> Rgba {
        self.0[3] = a;
        self
    }
    /// Set the alpha channel (absolute, 0..255).
    pub fn scale_a(self, f: f64) -> Rgba {
        self.with_a(f.clamp(0.0, 255.0) as u8)
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa` → Rgba (white on junk).
pub fn parse_color(s: &str) -> Rgba {
    fn opt(s: &str) -> Option<Rgba> {
        let t = s.trim().trim_start_matches('#');
        let hex = |i: usize, len: usize| -> Option<u8> {
            u8::from_str_radix(t.get(i..i + len)?, 16).ok()
        };
        match t.len() {
            3 => Some(Rgba([
                hex(0, 1).map(|v| v * 17)?,
                hex(1, 1).map(|v| v * 17)?,
                hex(2, 1).map(|v| v * 17)?,
                255,
            ])),
            6 => Some(Rgba([hex(0, 2)?, hex(2, 2)?, hex(4, 2)?, 255])),
            8 => Some(Rgba([hex(0, 2)?, hex(2, 2)?, hex(4, 2)?, hex(6, 2)?])),
            _ => None,
        }
    }
    opt(s).unwrap_or(Rgba::rgb(255, 255, 255))
}

/// Overlay panel style variant (mockups 1..5). Paint-only: shapes + palette
/// tints in the software rasterizer — zero extra runtime memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleKind {
    Glass,
    Neon,
    Hud,
    Soft,
    Crt,
}

impl StyleKind {
    pub const ALL: [(&str, &str, &str); 5] = [
        ("glass", "Стекло", "Glass"),
        ("neon", "Неон", "Neon"),
        ("hud", "Тактический", "Tactical"),
        ("soft", "Мягкий", "Soft"),
        ("crt", "Кинескоп", "CRT"),
    ];
    /// Signature accent per style (applied when the style tile is clicked).
    pub const ACCENTS: [&str; 5] = ["#7c6cff", "#22d3ee", "#f59e0b", "#c084fc", "#4ade80"];

    pub fn parse(s: &str) -> StyleKind {
        match s {
            "neon" => StyleKind::Neon,
            "hud" => StyleKind::Hud,
            "soft" => StyleKind::Soft,
            "crt" => StyleKind::Crt,
            _ => StyleKind::Glass,
        }
    }

    pub fn id(&self) -> &'static str {
        StyleKind::ALL[*self as usize].0
    }

    /// Secondary accent (neon magenta, CRT amber) — otherwise the user accent.
    pub fn second(&self, accent: Rgba) -> Rgba {
        match self {
            StyleKind::Neon => Rgba::rgb(255, 47, 214),
            StyleKind::Crt => Rgba::rgb(255, 176, 60),
            _ => accent,
        }
    }
}

pub fn mix(a: Rgba, b: Rgba, t: f64) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let m = |i: usize| (a.0[i] as f64 * (1.0 - t) + b.0[i] as f64 * t).round() as u8;
    Rgba([m(0), m(1), m(2), m(3)])
}

/// hsl(h, 72%, 56%) like `avatarGradient` in public/js/icons.js.
pub fn hsl(h: f64, s: f64, l: f64) -> Rgba {
    let h = ((h % 360.0) + 360.0) % 360.0 / 360.0;
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |t: f64| {
        let t = ((t % 1.0) + 1.0) % 1.0;
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    Rgba([f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0), 255])
}

/// fnv1a → hue, mirrors `hashHue(`${id}|${name}`)` closely enough.
pub fn hash_hue(s: &str) -> f64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    (h % 360) as f64
}

/// Everything the overlay draws, resolved per config.appearance.
#[derive(Debug, Clone)]
pub struct Palette {
    pub accent: Rgba,
    pub accent_rgb: (u8, u8, u8),
    pub panel_bg: Rgba,
    pub panel_border: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub icon: Rgba,
    pub row_hover: Rgba,
    pub talking_bg: Rgba,
    pub talk_outline: Rgba,
    pub nick: Rgba,
    pub msg: Rgba,
    pub danger: Rgba,
    pub ok: Rgba,
    pub warn: Rgba,
    pub shadow: Rgba,
    pub glow: f64,
    pub radius: f64,
    pub opacity: f64,
}

impl Palette {
    /// Copy with every color's alpha multiplied by `k` (row fade-out).
    pub fn fade(&self, k: f32) -> Palette {
        let f = |c: Rgba| {
            let a = ((c.0[3] as f32) * k).clamp(0.0, 255.0) as u8;
            Rgba([c.0[0], c.0[1], c.0[2], a])
        };
        Palette {
            accent: f(self.accent),
            accent_rgb: self.accent_rgb,
            panel_bg: f(self.panel_bg),
            panel_border: f(self.panel_border),
            text: f(self.text),
            muted: f(self.muted),
            icon: f(self.icon),
            row_hover: f(self.row_hover),
            talking_bg: f(self.talking_bg),
            talk_outline: f(self.talk_outline),
            nick: f(self.nick),
            msg: f(self.msg),
            danger: f(self.danger),
            ok: f(self.ok),
            warn: f(self.warn),
            shadow: f(self.shadow),
            glow: self.glow,
            radius: self.radius,
            opacity: self.opacity,
        }
    }

    pub fn from_appearance(a: &Appearance) -> Palette {
        let accent = parse_color(&a.accent);
        let theme = a.theme.as_str();
        // panel base rgb + alpha curve, per overlay.css theme blocks
        let (base, alpha_curve, border, text, muted, icon, hover, talk_a, shadow): (
            (u8, u8, u8),
            f64,
            Rgba,
            &str,
            &str,
            &str,
            Rgba,
            f64,
            Rgba,
        ) = match theme {
            "midnight" => (
                (8, 11, 23),
                0.96,
                Rgba::rgb(86, 108, 255).scale_a(41.0),
                "#dfe6ff",
                "#7d87ad",
                "#8d97ba",
                Rgba::rgb(96, 120, 255).scale_a(20.0),
                0.18,
                Rgba::rgb(0, 0, 0).scale_a(153.0),
            ),
            "frost" => (
                (248, 250, 255),
                0.92,
                Rgba::rgb(30, 41, 82).scale_a(31.0),
                "#171d33",
                "#5c6584",
                "#4c5677",
                Rgba::rgb(30, 41, 82).scale_a(15.0),
                0.13,
                Rgba::rgb(35, 48, 92).scale_a(46.0),
            ),
            "neon" => (
                (5, 6, 14),
                0.9,
                accent.scale_a(128.0),
                "#f2f5ff",
                "#97a1cf",
                "#b3bdf0",
                accent.scale_a(36.0),
                0.24,
                Rgba::rgb(0, 0, 0).scale_a(140.0),
            ),
            "emerald" => (
                (5, 16, 13),
                0.95,
                Rgba::rgb(16, 185, 129).scale_a(56.0),
                "#e6fff4",
                "#83b3a2",
                "#9ad4bf",
                Rgba::rgb(16, 185, 129).scale_a(23.0),
                0.18,
                Rgba::rgb(0, 0, 0).scale_a(128.0),
            ),
            "carmine" => (
                (21, 8, 12),
                0.95,
                Rgba::rgb(244, 63, 94).scale_a(61.0),
                "#ffeef1",
                "#b48a92",
                "#d8a2ac",
                Rgba::rgb(244, 63, 94).scale_a(23.0),
                0.18,
                Rgba::rgb(0, 0, 0).scale_a(133.0),
            ),
            "amber" => (
                (19, 14, 7),
                0.95,
                Rgba::rgb(245, 158, 11).scale_a(56.0),
                "#fff6e6",
                "#bfa57c",
                "#d9bd8d",
                Rgba::rgb(245, 158, 11).scale_a(23.0),
                0.16,
                Rgba::rgb(0, 0, 0).scale_a(128.0),
            ),
            "mono" => (
                (10, 10, 12),
                0.97,
                Rgba::rgb(255, 255, 255).scale_a(36.0),
                "#f0f0f2",
                "#8f8f96",
                "#b6b6bd",
                Rgba::rgb(255, 255, 255).scale_a(15.0),
                0.10,
                Rgba::rgb(0, 0, 0).scale_a(140.0),
            ),
            _ => (
                // glass
                (14, 16, 27),
                1.0,
                Rgba::rgb(255, 255, 255).scale_a(23.0),
                "#eef0fa",
                "#9aa2bd",
                "#aab2cc",
                Rgba::rgb(255, 255, 255).scale_a(14.0),
                0.14,
                Rgba::rgb(0, 0, 0).scale_a(115.0),
            ),
        };
        let opacity = if theme == "glass" {
            a.opacity
        } else {
            // calc(var(--panel-opacity) * K + (1-K)) as in the css
            let k = alpha_curve;
            a.opacity * k + (1.0 - k)
        };
        Palette {
            accent,
            accent_rgb: (accent.0[0], accent.0[1], accent.0[2]),
            panel_bg: Rgba::rgb(base.0, base.1, base.2)
                .with_a((opacity.clamp(0.0, 1.0) * 255.0) as u8),
            panel_border: border,
            text: parse_color(text),
            muted: parse_color(muted),
            icon: parse_color(icon),
            row_hover: hover,
            talking_bg: if theme == "mono" {
                Rgba::rgb(255, 255, 255).scale_a(talk_a * 255.0 as f64)
            } else {
                accent.scale_a(talk_a * 255.0 as f64)
            },
            talk_outline: parse_color(&a.talk_outline),
            nick: parse_color(&a.nick_color),
            msg: parse_color(&a.msg_color),
            danger: Rgba::rgb(248, 113, 113),
            ok: Rgba::rgb(52, 211, 153),
            warn: Rgba::rgb(251, 191, 36),
            shadow,
            glow: a.glow,
            radius: a.radius,
            opacity: a.opacity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(parse_color("#7c6cff").0, [0x7c, 0x6c, 0xff, 255]);
        assert_eq!(parse_color("#f00").0, [0xff, 0, 0, 255]);
        assert_eq!(parse_color("#11223344").0, [0x11, 0x22, 0x33, 0x44]);
        assert_eq!(parse_color("junk").0, [255, 255, 255, 255]);
    }

    #[test]
    fn glass_defaults_match_css() {
        let p = Palette::from_appearance(&Appearance::default());
        assert_eq!(p.panel_bg.0, [14, 16, 27, 183]); // 0.72*255 = 183.6
        assert_eq!(p.accent.0, [0x7c, 0x6c, 0xff, 255]);
        assert_eq!(p.text.0, [0xee, 0xf0, 0xfa, 255]);
    }

    #[test]
    fn all_eight_themes_resolve() {
        for theme in ["glass", "midnight", "frost", "neon", "emerald", "carmine", "amber", "mono"] {
            let mut a = Appearance::default();
            a.theme = theme.to_string();
            let p = Palette::from_appearance(&a);
            assert!(p.panel_bg.0[3] > 30, "{theme}");
        }
    }
}
