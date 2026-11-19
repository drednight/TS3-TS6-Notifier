//! Overlay panel layout — mirrors `public/css/overlay.css` + `public/js/app.js`
//! row rules (v1.4.7 event rows, v1.4.9 talking-row outline, whisper linger).

use crate::assets::Icons;
use crate::config::Appearance;
use crate::font::Fonts;
use crate::model::UserState;
use crate::render::Buf;
use crate::theme::{hash_hue, hsl, mix, Palette, Rgba, StyleKind};

pub const PANEL_W: f32 = 292.0; // css px (#panel width)
pub const SHADOW_PAD: f32 = 26.0; // css px transparent margin for the drop shadow

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Drag,
    Save,
    Discard,
}

/// One feed (events/chat) line.
#[derive(Debug, Clone)]
pub struct FeedItem {
    pub nickname: String,
    pub text: String,
    pub kind: u8, // 0 = system event row, 1 = private chat, 2 = channel/server chat
    /// 1.0 = fully visible, ramping to 0 as the row ages out
    pub alpha: f32,
}

pub struct FrameInput<'a> {
    pub appearance: &'a Appearance,
    pub pal: &'a Palette,
    pub fonts: &'a Fonts,
    pub icons: &'a Icons,
    pub status: &'a str,
    pub server_name: &'a str,
    pub channel_name: &'a str,
    pub user_count: usize,
    pub users: Vec<UserState>,
    /// per-row talking-highlight alpha: 1 = speaking, ramp → 0 = fading out, 0 = none
    pub talk_alphas: Vec<f32>,
    /// class keys aligned with `users` (avatar slot art)
    pub class_keys: Vec<Option<String>>,
    /// scroll offset (css px) inside a height-capped panel (panelMaxHeight)
    pub roster_scroll: f32,
    /// class keys aligned with `whisperers`
    pub whisper_keys: Vec<(String, Option<String>)>,
    /// whisper-section users (full rows) — split+linger applied by caller
    pub whisperers: Vec<UserState>,
    pub feed: Vec<FeedItem>,
    pub lang: &'a str,
    pub move_mode: bool,
    pub now_ms: u64,
    pub work_h: f32,
}

pub struct Frame {
    pub buf: Buf,
    /// suggested roster scroll (keeps the first talking row visible)
    pub scroll_to: f32,
    pub hits: Vec<(Rect, Hit)>,
    pub panel_w: f32,
    pub panel_h: f32,
}

/// ru/en overlay strings (matches public/js/i18n.js).
fn tr(lang: &str, ru: &str, en: &str) -> String {
    if lang == "en" {
        en.to_string()
    } else {
        ru.to_string()
    }
}

/// CLASS_META port (public/js/features.js).
pub const CLASS_META: [(&str, &str, &str); 18] = [
    ("kanglong", "Канглонг", "Kanglong"),
    ("blood", "Дух Крови", "Blood Spirit"),
    ("werewolf", "Оборотень", "Werewolf"),
    ("warrior", "Воин", "Warrior"),
    ("assassin", "Убийца", "Assassin"),
    ("gunslinger", "Стрелок", "Gunner"),
    ("guardian", "Страж", "Guardian"),
    ("priest", "Прист", "Priest"),
    ("archer", "Лучник", "Archer"),
    ("paladin", "Паладин", "Paladin"),
    ("shaman", "Шаман", "Shaman"),
    ("bard", "Бард", "Bard"),
    ("mystic", "Мистик", "Mystic"),
    ("druid", "Друид", "Druid"),
    ("monkey", "Обезьяна", "Monkey"),
    ("mage", "Маг", "Mage"),
    ("reaper", "Жнец", "Reaper"),
    ("ghost", "Призрак", "Ghost"),
];

pub fn norm_name(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in s.to_lowercase().chars() {
        let ch = if ch == 'ё' { 'е' } else { ch };
        if ch.is_ascii_alphanumeric() || ('а'..='я').contains(&ch) {
            out.push(ch);
            space = false;
        } else if !space && !out.is_empty() {
            out.push(' ');
            space = true;
        }
    }
    out.trim().to_string()
}

/// Word-prefix name match: «Духи Крови» matches «Дух Крови», «Лучники» → «Лучник».
fn name_matches(name: &str, label: &str) -> bool {
    let name_n = norm_name(name);
    let label_n = norm_name(label);
    let n_words: Vec<&str> = name_n.split(' ').filter(|w| !w.is_empty()).collect();
    let l_words: Vec<&str> = label_n.split(' ').filter(|w| !w.is_empty()).collect();
    if n_words.is_empty() || l_words.is_empty() {
        return false;
    }
    l_words.iter().all(|lw| n_words.iter().any(|nw| nw.starts_with(lw) || lw.starts_with(nw)))
}

/// features.js `classGroupFor` → class key (manual map, then name match).
pub fn class_key_for(
    server_groups: &str,
    groups: &[(i64, String)],
    class_map: &[(String, String)],
) -> Option<String> {
    let ids: Vec<i64> = server_groups
        .split(',')
        .filter_map(|x| x.trim().parse::<i64>().ok())
        .filter(|n| *n > 0)
        .collect();
    let mut labels: Vec<(usize, String)> = Vec::new();
    for (i, (_, ru, en)) in CLASS_META.iter().enumerate() {
        labels.push((i, ru.to_string()));
        labels.push((i, en.to_string()));
    }
    labels.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    for gid in &ids {
        let name = groups.iter().find(|(id, _)| id == gid).map(|(_, n)| n.clone()).unwrap_or_default();
        // 1) manual mapping (group id → class) — needed for TS6 (no names there)
        if let Some((_, mapped)) = class_map.iter().find(|(k, _)| k == &gid.to_string()) {
            if mapped != "none" {
                if CLASS_META.iter().any(|(key, _, _)| key == mapped) {
                    return Some(mapped.clone());
                }
            }
        }
        // 2) name matching (TS3 groups carry names)
        if !name.is_empty() {
            for (idx, label) in &labels {
                if name_matches(&name, label) {
                    return Some(CLASS_META[*idx].0.to_string());
                }
            }
        }
    }
    None
}

fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.trim().split_whitespace().collect();
    if parts.is_empty() {
        return "?".into();
    }
    let first = parts[0].chars().next().unwrap_or('?');
    if parts.len() == 1 {
        return first.to_uppercase().to_string();
    }
    let last = parts[parts.len() - 1].chars().next().unwrap_or(' ');
    format!("{}{}", first.to_uppercase(), last.to_uppercase())
}

fn avatar_gradient(id: &str, name: &str) -> (Rgba, Rgba) {
    let hue = hash_hue(&format!("{id}|{name}"));
    (hsl(hue, 0.72, 0.56), hsl(hue + 42.0, 0.70, 0.41))
}

/// tiny 24x24 gradient avatar fallback tile (135deg, like avatarGradient)
fn gradient_tile(g1: Rgba, g2: Rgba) -> Vec<u8> {
    let mut px = vec![0u8; 24 * 24 * 4];
    for y in 0..24 {
        for x in 0..24 {
            let t = ((x + y) as f64 / 46.0).clamp(0.0, 1.0);
            let c = mix(g1, g2, t);
            let i = (y * 24 + x) * 4;
            px[i..i + 4].copy_from_slice(&c.0);
        }
    }
    px
}

struct Ctx<'a> {
    pal: &'a Palette,
    fonts: &'a Fonts,
    icons: &'a Icons,
    k: f32,
    fs: f32,
    now: f64,
    anim: bool,
    sk: StyleKind,
    borderless: bool,
}

impl<'a> Ctx<'a> {
    fn s(&self, v: f32) -> f32 {
        v * self.k
    }
    fn em(&self, em: f32) -> f32 {
        self.fs * em
    }
    /// Style-aware corner radius (from a base px radius).
    fn rad(&self, base: f32) -> f32 {
        match self.sk {
            StyleKind::Glass => base,
            StyleKind::Neon | StyleKind::Hud => (base * 0.3).clamp(self.s(1.5), self.s(6.0)),
            StyleKind::Soft => base * 1.5,
            StyleKind::Crt | StyleKind::Retro => 0.0,
            StyleKind::Minimal | StyleKind::Cyber => (base * 0.2).clamp(self.s(1.5), self.s(6.0)),
            StyleKind::Ice => base * 1.3,
            StyleKind::Lava => base * 0.8,
        }
    }
    /// Style-aware panel/row background tint.
    fn bg(&self, base: Rgba) -> Rgba {
        match self.sk {
            StyleKind::Glass => base,
            StyleKind::Neon => mix(base, Rgba::rgb(2, 3, 8), 0.42),
            StyleKind::Hud => mix(base, Rgba::rgb(6, 8, 10), 0.24),
            StyleKind::Soft => mix(base, Rgba::rgb(255, 255, 255), 0.10),
            StyleKind::Crt | StyleKind::Retro => mix(base, Rgba::rgb(4, 6, 4), 0.35),
            StyleKind::Minimal => mix(base, Rgba::rgb(0, 0, 0), 0.10),
            StyleKind::Cyber => mix(base, Rgba::rgb(12, 0, 22), 0.45),
            StyleKind::Ice => mix(base, Rgba::rgb(10, 22, 38), 0.28),
            StyleKind::Lava => mix(base, Rgba::rgb(22, 7, 3), 0.38),
        }
    }
    /// Style-aware border color.
    fn border(&self, base: Rgba) -> Rgba {
        match self.sk {
            StyleKind::Glass => base,
            StyleKind::Neon => self.pal.accent.scale_a(150.0),
            StyleKind::Hud => mix(base, self.pal.accent, 0.35),
            StyleKind::Soft => mix(base, Rgba::rgb(255, 255, 255), 0.22),
            StyleKind::Crt | StyleKind::Retro => self.pal.accent.scale_a(110.0),
            StyleKind::Minimal => base.scale_a(160.0),
            StyleKind::Cyber => self.pal.accent.scale_a(140.0),
            StyleKind::Ice => mix(base, Rgba::rgb(255, 255, 255), 0.35),
            StyleKind::Lava => mix(base, self.pal.accent, 0.5),
        }
    }
    /// Whisper/talk secondary accent (neon magenta, CRT amber, else user accent).
    fn second(&self) -> Rgba {
        self.sk.second(self.pal.accent)
    }
}

fn draw_you_chip(c: &Ctx, buf: &mut Buf, x: f32, y: f32, label: &str) -> f32 {
    let size = c.em(0.72);
    let tw = c.fonts.measure(label, size, true);
    let pad = c.s(5.0);
    let h = size * 1.8;
    buf.round_rect(x, y, tw + pad * 2.0, h, c.rad(c.s(6.0)), c.pal.accent.scale_a(41.0));
    c.fonts.draw(buf, x + pad, y + h * 0.72, label, size, true, c.pal.accent);
    x + pad * 2.0 + tw
}

fn draw_badges(c: &Ctx, buf: &mut Buf, right: f32, cy: f32, u: &UserState) -> f32 {
    let mut x = right;
    let sz = c.s(12.0);
    let t = c.s(2.0);
    let mut one = |buf: &mut Buf, color: Rgba, draw: &mut dyn FnMut(&mut Buf, f32, f32, f32, Rgba)| {
        x -= sz + c.s(4.0);
        draw(buf, x, cy - sz * 0.5, sz, color);
    };
    if u.priority_speaker {
        one(buf, c.pal.warn, &mut |buf, x, y, s, col| {
            buf.line(x + s * 0.5, y, x + s * 0.08, y + s * 0.28, t, col);
            buf.line(x + s * 0.5, y, x + s * 0.92, y + s * 0.28, t, col);
            buf.line(x + s * 0.08, y + s * 0.28, x + s * 0.5, y + s, t, col);
            buf.line(x + s * 0.92, y + s * 0.28, x + s * 0.5, y + s, t, col);
        });
    }
    if u.channel_commander {
        one(buf, c.pal.accent, &mut |buf, x, y, s, col| {
            buf.line(x + s * 0.1, y + s * 0.85, x + s * 0.9, y + s * 0.85, t, col);
            buf.line(x + s * 0.1, y + s * 0.85, x + s * 0.18, y + s * 0.2, t, col);
            buf.line(x + s * 0.18, y + s * 0.2, x + s * 0.5, y + s * 0.55, t, col);
            buf.line(x + s * 0.5, y + s * 0.55, x + s * 0.82, y + s * 0.2, t, col);
            buf.line(x + s * 0.82, y + s * 0.2, x + s * 0.9, y + s * 0.85, t, col);
        });
    }
    if u.away {
        one(buf, c.pal.muted, &mut |buf, x, y, s, col| {
            buf.stroke_circle(x + s * 0.5, y + s * 0.5, s * 0.38, col, t);
        });
    }
    if u.output_muted {
        one(buf, c.pal.danger.scale_a(204.0), &mut |buf, x, y, s, col| {
            buf.line(x + s * 0.2, y + s * 0.65, x + s * 0.2, y + s * 0.35, t, col);
            buf.line(x + s * 0.2, y + s * 0.35, x + s * 0.8, y + s * 0.35, t, col);
            buf.line(x + s * 0.8, y + s * 0.35, x + s * 0.8, y + s * 0.65, t, col);
            buf.line(x + s * 0.12, y + s * 0.12, x + s * 0.88, y + s * 0.88, t, col);
        });
    }
    if u.input_muted {
        one(buf, c.pal.danger, &mut |buf, x, y, s, col| {
            buf.round_rect(x + s * 0.34, y + s * 0.1, s * 0.32, s * 0.5, s * 0.16, col);
            buf.line(x + s * 0.2, y + s * 0.52, x + s * 0.8, y + s * 0.52, t, col);
            buf.line(x + s * 0.5, y + s * 0.52, x + s * 0.5, y + s * 0.8, t, col);
            buf.line(x + s * 0.12, y + s * 0.12, x + s * 0.88, y + s * 0.88, t, col);
        });
    }
    x
}

fn draw_eq(c: &Ctx, buf: &mut Buf, right: f32, cy: f32, style: &str, talking: bool) -> f32 {
    if style == "none" {
        return right;
    }
    let area_w = c.s(18.0);
    let x0 = right - area_w;
    if style == "ring" {
        let r = c.s(4.5);
        let mut a = if talking { 1.0f32 } else { 0.35 };
        if talking && c.anim {
            a = 0.55 + 0.45 * (((c.now / 1100.0) % 1.0) as f32 * std::f32::consts::PI).sin().abs();
        }
        buf.stroke_circle(x0 + area_w * 0.5, cy, r, c.pal.accent.with_a((255.0 * a) as u8), c.s(2.0));
        return x0 - c.s(4.0);
    }
    if style == "dots" {
        for i in 0..3 {
            let mut h = if talking { 0.6f32 } else { 0.35 };
            if talking && c.anim {
                let ph = ((c.now / 1000.0) + i as f64 * 0.2) % 1.0;
                h = 0.3 + 0.7 * (ph as f32 * std::f32::consts::PI).sin().abs();
            }
            let r = c.s(2.0);
            let bx = x0 + area_w * 0.5 + (i as f32 - 1.0) * c.s(7.0);
            buf.circle(bx, cy, r, c.pal.accent.with_a((255.0 * h.max(0.35)) as u8));
        }
        return x0 - c.s(4.0);
    }
    // bars (default)
    for i in 0..3 {
        let mut h = if talking { 0.6f32 } else { 0.35 };
        if talking && c.anim {
            let ph = ((c.now / 900.0) + i as f64 * 0.18) % 1.0;
            h = 0.3 + 0.7 * (ph as f32 * std::f32::consts::PI).sin().abs();
        }
        let bh = c.s(16.0) * h.max(0.35);
        let bx = x0 + area_w * 0.5 + (i as f32 - 1.0) * c.s(5.5) - c.s(1.5);
        buf.round_rect(bx, cy + c.s(8.0) - bh, c.s(3.0), bh, c.s(1.5), c.pal.accent);
    }
    x0 - c.s(4.0)
}

#[allow(clippy::too_many_arguments)]
/// Word-wrap `text` into at most `cap` lines; line 0 fits `w_first`, later
/// lines `w_rest`. Oversized words are hard-split; the last line gets an
/// ellipsis when text remains. Long nick+message rows wrap to the next line.
fn wrap_flow(
    fonts: &Fonts,
    text: &str,
    size: f32,
    bold: bool,
    w_first: f32,
    w_rest: f32,
    cap: usize,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let w = if lines.is_empty() { w_first } else { w_rest };
        let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if fonts.measure(&cand, size, bold) <= w {
            cur = cand;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        cur = word.to_string();
        let w2 = if lines.is_empty() { w_first } else { w_rest };
        while fonts.measure(&cur, size, bold) > w2 {
            let mut acc = String::new();
            let mut rest = String::new();
            let mut it = cur.chars();
            for ch in it.by_ref() {
                let t = format!("{acc}{ch}");
                if fonts.measure(&t, size, bold) > w2 && !acc.is_empty() {
                    rest.push(ch);
                    rest.extend(it);
                    break;
                }
                acc = t;
            }
            if acc.is_empty() {
                break;
            }
            lines.push(acc);
            cur = rest;
            if cur.is_empty() {
                break;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    if lines.len() > cap {
        lines.truncate(cap);
        let w = if lines.len() == 1 { w_first } else { w_rest };
        let mut last = lines.pop().unwrap();
        while !last.is_empty() && fonts.measure(&format!("{last}…"), size, bold) > w {
            last.pop();
        }
        lines.push(format!("{last}…"));
    }
    lines
}

/// Pre-wrapped feed-row geometry (draw-ready nick, text offset, lines, height).
struct FeedGeo {
    nick: String,
    tx: f32,
    lines: Vec<String>,
    row_h: f32,
}

/// Wrap one feed row against row width `w0`; long nick+message rows wrap to the
/// next line(s) — up to 3 lines total («переносить на некст строчку»).
fn feed_wrap(c: &Ctx, item: &FeedItem, w0: f32) -> FeedGeo {
    let size = c.em(0.92);
    let nick = c.fonts.truncate(&item.nickname, size, true, w0 * 0.42);
    let nick_w = c.fonts.measure(&nick, size, true);
    let tx = c.s(8.0) + nick_w + c.s(6.0);
    let first_w = (w0 - c.s(8.0) - tx).max(40.0);
    let rest_w = (w0 - c.s(16.0)).max(40.0);
    let text = if item.kind == 0 {
        item.text.clone()
    } else {
        format!("— {}", item.text)
    };
    let lines = wrap_flow(c.fonts, &text, size, false, first_w, rest_w, 3);
    let row_h = c.s(26.0) + (lines.len().saturating_sub(1) as f32) * c.s(16.0);
    FeedGeo {
        nick,
        tx,
        lines,
        row_h,
    }
}

fn draw_user_row(
    c: &Ctx,
    buf: &mut Buf,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    u: &UserState,
    class_key: Option<&str>,
    whisper: bool,
    talk_alpha: f32,
    you_label: &str,
    opts: &RowOpts,
) {
    let pal = c.pal;
    // talking-highlight alpha («Задержка исчезновения говорящих»): 1 = speaking,
    // ramping to 0 as the highlight lingers after speech stops
    let tpal = pal.fade(talk_alpha.clamp(0.0, 1.0));
    let radius = c.rad(pal.radius as f32 * 0.5 * c.k);
    let secc = if whisper { c.second() } else { pal.accent };
    if !whisper && (u.talking || talk_alpha < 0.999) {
        // v1.4.9 «Обводка говорящего» — highlight of the whole row strip (every
        // style), lingering + fading out over `talking_fade` after speech stops
        let bg = if c.borderless { Rgba([0, 0, 0, 0]) } else { tpal.talking_bg };
        let ac = tpal.accent;
        match c.sk {
            StyleKind::Neon | StyleKind::Cyber => {
                buf.round_rect(x, y, w, h, radius, bg);
                buf.fill_rect(x, y + h - c.s(2.0), w, c.s(2.0), ac);
                buf.fill_rect(x, y, c.s(2.0), h, ac.scale_a(120.0 * talk_alpha as f64));
            }
            StyleKind::Hud | StyleKind::Lava => {
                buf.round_rect(x, y, w, h, radius, bg);
                buf.fill_rect(x, y, c.s(3.0), h, ac);
                buf.stroke_round_rect(x, y, w, h, radius, ac.scale_a(90.0 * talk_alpha as f64), 1.0);
            }
            StyleKind::Soft | StyleKind::Ice => {
                buf.round_rect(x, y, w, h, h * 0.42, bg);
                buf.stroke_round_rect(x, y, w, h, h * 0.42, tpal.talk_outline.scale_a(150.0 * talk_alpha as f64), c.s(1.5));
            }
            StyleKind::Crt | StyleKind::Retro => {
                buf.round_rect(x, y, w, h, radius, ac.scale_a(30.0 * talk_alpha as f64));
                buf.stroke_round_rect(x, y, w, h, radius, ac.scale_a(170.0 * talk_alpha as f64), 1.0);
            }
            StyleKind::Glass | StyleKind::Minimal => {
                buf.round_rect(x, y, w, h, radius, bg);
                buf.stroke_round_rect(x, y, w, h, radius, tpal.talk_outline, c.s(2.0));
            }
        }
    }
    if whisper {
        // .whisper-list .row::before — accent wash fading to 70% width
        let steps = 12;
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let a = if t < 0.7 { (46.0 * (1.0 - t / 0.7)) as u8 } else { 0 };
            if a == 0 {
                continue;
            }
            buf.fill_rect(x + w * t, y, w / steps as f32 + 1.0, h, secc.scale_a(a as f64));
        }
    }
    let av = if h < c.s(36.0) { c.s(24.0) } else { c.s(30.0) };
    let av_x = x + if whisper { c.s(11.0) } else { c.s(8.0) };
    let (cx, cy) = (av_x + av * 0.5, y + h * 0.5);

    if opts.show_avatars {
        if let Some(key) = class_key.and_then(|k| c.icons.get(k)) {
            buf.circle(cx, cy, av * 0.5, Rgba([255, 255, 255, 15]));
            buf.blit_rgba(cx - av * 0.5, cy - av * 0.5, av, av, &key.px, key.w as i32, key.h as i32, true);
        } else {
            let (g1, g2) = avatar_gradient(&u.avatar_hash, &u.nickname);
            buf.blit_rgba(cx - av * 0.5, cy - av * 0.5, av, av, &gradient_tile(g1, g2), 24, 24, true);
            let ini = initials(&u.nickname);
            let size = c.s(if h < c.s(36.0) { 10.0 } else { 11.5 });
            let tw = c.fonts.measure(&ini, size, true);
            c.fonts.draw(buf, cx - tw * 0.5, cy + size * 0.36, &ini, size, true, Rgba::rgb(255, 255, 255));
        }
    }
    if u.talking || talk_alpha < 0.999 {
        let glow_a = ((0.12 + 1.05 * pal.glow) as f32) * talk_alpha;
        buf.stroke_circle(cx, cy, av * 0.5 + c.s(1.0), tpal.accent, c.s(2.0));
        for i in 0..3 {
            let g = (i as f32 + 1.0) * c.s(2.5);
            buf.stroke_circle(
                cx,
                cy,
                av * 0.5 + c.s(1.0) + g,
                tpal.accent.scale_a((255.0 * glow_a * 0.22) as f64),
                c.s(2.2),
            );
        }
    }
    // status pip (bottom-right of the avatar)
    if opts.show_avatars {
        let pip_r = c.s(5.5);
        let pip_c = if u.talking {
            pal.accent
        } else if u.input_muted {
            pal.danger
        } else if u.away {
            pal.warn
        } else {
            pal.ok
        };
        buf.circle(cx + av * 0.36, cy + av * 0.36, pip_r + c.s(1.2), Rgba([12, 14, 24, 217]));
        buf.circle(cx + av * 0.36, cy + av * 0.36, pip_r, pip_c);
    }

    // text column
    let tx = if opts.show_avatars { av_x + av + c.s(9.0) } else { x + c.s(10.0) };
    let nick_size = c.em(1.0);
    // «Мой ник не меняет цвет» — the nick colour drives every row; mute/away
    // only dims it towards muted instead of replacing the colour.
    let nick_color = if whisper {
        secc
    } else if u.input_muted || u.away {
        mix(pal.nick, pal.muted, 0.62)
    } else {
        pal.nick
    };
    let nick = c.fonts.truncate(&u.nickname, nick_size, true, w - (tx - x) - c.s(72.0));
    let sub = if !u.away_message.is_empty() && (u.away || whisper) {
        Some(c.fonts.truncate(&u.away_message, c.em(0.76), false, w - (tx - x) - c.s(24.0)))
    } else {
        None
    };
    if let Some(sub) = &sub {
        let nb = y + c.s(11.0) + nick_size * 0.8;
        c.fonts.draw(buf, tx, nb, &nick, nick_size, true, nick_color);
        let mut chip_x = tx + c.fonts.measure(&nick, nick_size, true) + c.s(6.0);
        if u.is_self {
            chip_x = draw_you_chip(c, buf, chip_x, nb - nick_size * 0.74, you_label);
            let _ = chip_x;
        }
        c.fonts.draw(buf, tx, nb + nick_size * 1.3, sub, c.em(0.76), false, pal.muted);
    } else {
        let nb = cy + nick_size * 0.35;
        c.fonts.draw(buf, tx, nb, &nick, nick_size, true, nick_color);
        if u.is_self {
            let chip_x = tx + c.fonts.measure(&nick, nick_size, true) + c.s(6.0);
            draw_you_chip(c, buf, chip_x, nb - nick_size * 0.74, you_label);
        }
    }

    let mut right = x + w - c.s(8.0);
    if opts.show_badges {
        right = draw_badges(c, buf, right, cy, u);
    }
    if u.talking {
        draw_eq(c, buf, right - c.s(2.0), cy, &opts.eq_style, true);
    }
}

struct RowOpts {
    show_avatars: bool,
    show_badges: bool,
    eq_style: String,
}

/// Full panel frame: shadow, panel, header, rows, whisper block, feed, move handle.
pub fn render_frame(inp: &FrameInput) -> Frame {
    let a = inp.appearance;
    let k = a.scale.clamp(0.5, 3.0) as f32;
    let fs = a.font_size as f32 * k;
    let c = Ctx {
        pal: inp.pal,
        fonts: inp.fonts,
        icons: inp.icons,
        k,
        fs,
        now: inp.now_ms as f64,
        anim: a.animations,
        sk: StyleKind::parse(&a.style),
        borderless: a.borderless,
    };
    let pad = SHADOW_PAD * k;
    let panel_w = ((a.panel_width as f32) * k).clamp(200.0 * k, 600.0 * k);
    let opts = RowOpts {
        show_avatars: a.show_avatars,
        show_badges: a.show_badges,
        eq_style: a.eq_style.clone(),
    };
    let you_label = tr(inp.lang, "Вы", "you");
    // pre-wrapped feed rows (height estimate + draw share this)
    let feed_geo: Vec<FeedGeo> = inp
        .feed
        .iter()
        .map(|it| feed_wrap(&c, it, panel_w - c.s(20.0)))
        .collect();

    // ---------------- layout pass (panel coords) ----------------
    let handle_h = if inp.move_mode { c.s(44.0) } else { 0.0 };
    let header_h = if a.show_header {
        c.s(if a.compact { 44.0 } else { 52.0 })
    } else {
        0.0
    };
    let row_h = if a.compact { c.s(31.0) } else { c.s(42.0) };
    let row_gap = c.s(2.0);
    let list_pad = if a.compact { c.s(4.0) } else { c.s(6.0) };
    let whisper_head_h = if inp.whisperers.is_empty() { 0.0 } else { c.s(20.0) };
    let feed_row_h = c.s(26.0);
    let feed_gap = c.s(4.0);

    let mut y = handle_h + header_h + list_pad;
    let rows_top = y;
    for _ in &inp.users {
        y += row_h + row_gap;
    }
    let list_h = (y - rows_top - row_gap).max(0.0);
    y = rows_top + list_h + list_pad;
    if !inp.whisperers.is_empty() {
        y += whisper_head_h + c.s(2.0);
        for _ in &inp.whisperers {
            y += row_h + row_gap;
        }
        y += c.s(4.0);
    }
    if !inp.feed.is_empty() {
        y += c.s(4.0);
        for g in &feed_geo {
            y += g.row_h + feed_gap;
        }
        y += c.s(2.0);
    }
    if inp.users.is_empty() && !a.hide_empty && inp.whisperers.is_empty() {
        y += c.s(118.0);
    }
    let mut panel_h = y.max(header_h + c.s(40.0));
    // «Максимальная высота» (panelMaxHeight): 0 = auto (content height)
    let max_h = (a.panel_max_height as f32) * k;
    let mut scroll = 0.0f32;
    let mut scroll_max = 0.0f32;
    if max_h > 0.0 && panel_h > max_h {
        panel_h = max_h;
    }

    let bw = (panel_w + pad * 2.0).ceil() as i32;
    let bh = (panel_h + pad * 2.0).ceil() as i32;
    let mut buf = Buf::new(bw, bh);
    let mut hits: Vec<(Rect, Hit)> = Vec::new();

    // ---------------- draw pass ----------------
    let (px, py) = (pad, pad);
    // drop shadow + panel body (style-aware radius/tint/border)
    let pr = c.rad(c.s(inp.pal.radius as f32));
    // «Размытие» (appearance.blur) drives the shadow softness — the slider
    // must have a visible effect in the native renderer too.
    let blur_px = ((inp.appearance.blur as f32) * c.k).clamp(3.0, 44.0);
    if !c.borderless {
        buf.shadow_round_rect(px, py, panel_w, panel_h, pr, blur_px, inp.pal.shadow);
        buf.round_rect(px, py, panel_w, panel_h, pr, c.bg(inp.pal.panel_bg));
        buf.stroke_round_rect(px, py, panel_w, panel_h, pr, c.border(inp.pal.panel_border), 1.0);
    }

    let mut cy = py;
    // ---- move handle (z-index 50 in css — drawn over header) ----
    if inp.move_mode {
        let bg = mix(inp.pal.accent, Rgba::rgb(12, 14, 26).with_a(235), 0.22);
        buf.round_rect(px, cy, panel_w, handle_h, pr, bg);
        buf.stroke_round_rect(px, cy, panel_w, handle_h, pr, inp.pal.accent.scale_a(178.0), 1.0);
        // grip «⠿» as three short lines + text + ✓/✗ buttons
        let bx = px + c.s(12.0);
        for i in 0..3 {
            buf.round_rect(bx, cy + handle_h * 0.5 + (i as f32 - 1.0) * c.s(5.0) - c.s(1.0), c.s(14.0), c.s(2.0), c.s(1.0), inp.pal.accent);
        }
        let text = tr(inp.lang, "Перетащите панель", "Drag to move panel");
        let ts = c.s(12.0);
        let tw = inp.fonts.measure(&text, ts, false);
        inp.fonts.draw(&mut buf, px + (panel_w - tw) * 0.5, cy + handle_h * 0.5 + ts * 0.36, &text, ts, false, inp.pal.text);
        // ✓ / ✗ buttons (28px)
        let bs = c.s(28.0);
        let by = cy + (handle_h - bs) * 0.5;
        let bx_x = px + panel_w - c.s(12.0) - bs;
        buf.round_rect(bx_x, by, bs, bs, c.s(8.0), Rgba::rgb(22, 163, 74));
        buf.stroke_round_rect(bx_x, by, bs, bs, c.s(8.0), Rgba::rgb(34, 197, 94), 1.0);
        buf.line(bx_x + bs * 0.28, by + bs * 0.52, bx_x + bs * 0.44, by + bs * 0.68, c.s(2.6), Rgba::rgb(255, 255, 255));
        buf.line(bx_x + bs * 0.44, by + bs * 0.68, bx_x + bs * 0.74, by + bs * 0.32, c.s(2.6), Rgba::rgb(255, 255, 255));
        let nx_x = bx_x - bs - c.s(8.0);
        buf.round_rect(nx_x, by, bs, bs, c.s(8.0), Rgba::rgb(220, 38, 38));
        buf.stroke_round_rect(nx_x, by, bs, bs, c.s(8.0), Rgba::rgb(239, 68, 68), 1.0);
        buf.line(nx_x + bs * 0.32, by + bs * 0.32, nx_x + bs * 0.68, by + bs * 0.68, c.s(2.6), Rgba::rgb(255, 255, 255));
        buf.line(nx_x + bs * 0.68, by + bs * 0.32, nx_x + bs * 0.32, by + bs * 0.68, c.s(2.6), Rgba::rgb(255, 255, 255));
        hits.push((Rect { x: bx_x, y: by, w: bs, h: bs }, Hit::Save));
        hits.push((Rect { x: nx_x, y: by, w: bs, h: bs }, Hit::Discard));
        hits.push((Rect { x: px, y: cy, w: panel_w, h: handle_h }, Hit::Drag));
        cy += handle_h;
    }

    // ---- header ----
    if a.show_header {
        if !c.borderless {
        match c.sk {
            StyleKind::Glass | StyleKind::Soft | StyleKind::Ice => {
                // accent wash (linear-gradient 115deg ≈ horizontal fade)
                for i in 0..14 {
                    let t = i as f32 / 14.0;
                    let a_alpha = (30.0 * (1.0 - t) * (1.0 - t)) as u8;
                    if a_alpha == 0 {
                        continue;
                    }
                    buf.fill_rect(
                        px + panel_w * t,
                        cy,
                        panel_w / 14.0 + 1.0,
                        header_h,
                        inp.pal.accent.scale_a(a_alpha as f64),
                    );
                }
                buf.fill_rect(px, cy + header_h - 1.0, panel_w, 1.0, c.border(inp.pal.panel_border));
            }
            StyleKind::Neon | StyleKind::Cyber | StyleKind::Lava => {
                buf.fill_rect(px, cy, panel_w, header_h, Rgba::rgb(2, 3, 8).scale_a(90.0));
                buf.fill_rect(px, cy + header_h - 1.0, panel_w, 1.0, inp.pal.accent.scale_a(150.0));
            }
            StyleKind::Hud | StyleKind::Minimal => {
                buf.fill_rect(px, cy, panel_w, header_h, Rgba::rgb(0, 0, 0).scale_a(30.0));
                buf.fill_rect(px, cy + header_h - 2.0, panel_w, 1.0, c.border(inp.pal.panel_border));
                buf.fill_rect(px, cy + header_h - 1.0, panel_w, 1.0, inp.pal.accent.scale_a(90.0));
            }
            StyleKind::Crt | StyleKind::Retro => {
                let mut dx = px;
                while dx < px + panel_w - c.s(4.0) {
                    buf.fill_rect(dx, cy + header_h - 1.0, c.s(6.0), 1.0, inp.pal.accent.scale_a(120.0));
                    dx += c.s(12.0);
                }
            }
        }
        }
        // conn dot
        let dot_x = px + c.s(16.5);
        let dot_y = cy + header_h * 0.5 - c.s(8.0);
        let (dot_c, dot_glow) = match inp.status {
            "live" => (inp.pal.ok, true),
            "connecting" | "waiting-auth" => (inp.pal.warn, false),
            "error" => (inp.pal.danger, false),
            _ => (inp.pal.muted, false),
        };
        if dot_glow {
            buf.stroke_circle(dot_x, dot_y, c.s(4.5) + c.s(2.0), dot_c.scale_a(120.0), c.s(3.0));
        }
        if c.sk == StyleKind::Hud {
            buf.round_rect(dot_x - c.s(4.0), dot_y - c.s(4.0), c.s(8.0), c.s(8.0), c.s(1.0), dot_c);
        } else {
            buf.circle(dot_x, dot_y, c.s(4.5), dot_c);
        }
        // titles
        let tx = px + c.s(31.0);
        let ch_size = c.em(1.02);
        let ch = inp.fonts.truncate(inp.channel_name, ch_size, true, panel_w - c.s(31.0) - c.s(84.0));
        inp.fonts.draw(&mut buf, tx, cy + c.s(19.0) + ch_size * 0.4, &ch, ch_size, true, inp.pal.text);
        if a.show_server_name {
            let sv = inp.server_name.to_uppercase();
            let sv = inp.fonts.truncate(&sv, c.em(0.78), false, panel_w - c.s(31.0) - c.s(84.0));
            inp.fonts.draw(&mut buf, tx, cy + c.s(33.0) + c.em(0.78) * 0.6, &sv, c.em(0.78), false, inp.pal.muted);
        }
        // right cluster: user count pill (the settings button lives in the
        // tray now — «убери кнопку настроек с оверлея самого»)
        if a.show_user_count {
            let cnt = inp.user_count.to_string();
            let size = c.em(0.78);
            let tw = inp.fonts.measure(&cnt, size, true);
            let pw = tw + c.s(16.0);
            let ph = c.s(19.0);
            let pxx = px + panel_w - c.s(8.0) - pw;
            let pyy = cy + (header_h - ph) * 0.5;
            buf.round_rect(pxx, pyy, pw, ph, c.rad(ph * 0.5), inp.pal.row_hover);
            inp.fonts.draw(&mut buf, pxx + (pw - tw) * 0.5, pyy + ph * 0.68, &cnt, size, true, inp.pal.muted);
        }
        cy += header_h;
    }

    // ---- user rows ----
    cy += list_pad;
    // content viewport: roster/whispers/feed scroll inside a capped panel
    let content_top = cy;
    let content_h_total = (py + panel_h) - content_top;
    let content_total = list_h
        + (if inp.whisperers.is_empty() { 0.0 } else { whisper_head_h + c.s(2.0) + inp.whisperers.len() as f32 * (row_h + row_gap) + c.s(4.0) })
        + (if inp.feed.is_empty() { 0.0 } else { c.s(4.0) + feed_geo.iter().map(|g| g.row_h + feed_gap).sum::<f32>() + c.s(2.0) })
        + (if inp.users.is_empty() && !a.hide_empty && inp.whisperers.is_empty() { c.s(118.0) } else { 0.0 })
        + list_pad * 2.0;
    scroll_max = (content_total - content_h_total).max(0.0);
    scroll = inp.roster_scroll.clamp(0.0, scroll_max);
    let scroll_on = scroll_max > 0.5;
    if scroll_on {
        buf.set_clip(px + 1.0, content_top, panel_w - 2.0, content_h_total);
        cy -= scroll;
    }

    for (i, u) in inp.users.iter().enumerate() {
        let key = inp.class_keys.get(i).and_then(|k| k.as_deref());
        // «только говорящие»: rows fade out over `talking_fade` seconds
        let faded;
        let rc = if u.fade < 0.999 {
            faded = inp.pal.fade(u.fade);
            Ctx { pal: &faded, fonts: c.fonts, icons: c.icons, k: c.k, fs: c.fs, now: c.now, anim: c.anim, sk: c.sk, borderless: c.borderless }
        } else {
            Ctx { pal: c.pal, fonts: c.fonts, icons: c.icons, k: c.k, fs: c.fs, now: c.now, anim: c.anim, sk: c.sk, borderless: c.borderless }
        };
        draw_user_row(&rc, &mut buf, px + list_pad, cy, panel_w - list_pad * 2.0, row_h, u, key, false, inp.talk_alphas.get(i).copied().unwrap_or(1.0), &you_label, &opts);
        cy += row_h + row_gap;
    }
    cy = (cy - row_gap).max(0.0) + list_pad;

    // ---- whisper block ----
    if !inp.whisperers.is_empty() {
        let head = tr(inp.lang, "ШЁПОТ", "WHISPER");
        let hs = c.em(0.72);
        let secc = c.second();
        buf.round_rect(px + c.s(10.0), cy + whisper_head_h * 0.5 - c.s(1.0), c.s(3.0), c.s(3.0), c.s(1.5), secc);
        inp.fonts.draw(&mut buf, px + c.s(18.0), cy + whisper_head_h * 0.5 + hs * 0.36, &head, hs, true, secc);
        let tw = inp.fonts.measure(&head, hs, true);
        buf.fill_rect_vgrad(
            px + c.s(24.0) + tw,
            cy + whisper_head_h * 0.5 - c.s(0.5),
            panel_w - c.s(34.0) - tw,
            1.0,
            secc.scale_a(90.0),
            secc.scale_a(0.0),
        );
        cy += whisper_head_h + c.s(2.0);
        for u in &inp.whisperers {
            let key = inp.whisper_keys.iter().find(|(id, _)| *id == u.id).and_then(|(_, k)| k.as_deref());
            draw_user_row(&c, &mut buf, px + c.s(10.0), cy, panel_w - c.s(20.0), row_h, u, key, true, 1.0, &you_label, &opts);
            // whisperPulse: whole-row opacity 0.55 ↔ 1 (2.6s)
            if c.anim {
                let ph = ((c.now / 2600.0) % 1.0) as f32 * std::f32::consts::PI * 2.0;
                let f = 0.55 + 0.45 * (0.5 + 0.5 * ph.sin());
                buf.fade_rect(px + c.s(10.0), cy, panel_w - c.s(20.0), row_h, f, inp.pal.panel_bg);
            }
            cy += row_h + row_gap;
        }
        cy += c.s(4.0);
    }

    // ---- feed (events + chat) — long rows wrap, fading out with age ----
    if !inp.feed.is_empty() {
        cy += c.s(4.0);
        for (gi, item) in inp.feed.iter().enumerate() {
            let g = &feed_geo[gi];
            let y0 = cy;
            let x0 = px + c.s(10.0);
            let size = c.em(0.92);
            let line_h = c.s(16.0);
            let al = item.alpha.clamp(0.0, 1.0) as f64;
            if item.kind == 1 && !c.borderless {
                let fr = c.rad(c.s(10.0));
                buf.round_rect(x0, y0, panel_w - c.s(20.0), g.row_h, fr, inp.pal.accent.scale_a(26.0 * al));
                buf.stroke_round_rect(x0, y0, panel_w - c.s(20.0), g.row_h, fr, inp.pal.accent.scale_a(115.0 * al), 1.0);
            }
            let rp = inp.pal.fade(item.alpha.clamp(0.0, 1.0));
            let baseline = y0 + feed_row_h * 0.5 + size * 0.35;
            let ncol = if item.kind == 0 { rp.event_nick } else { rp.nick };
            inp.fonts.draw(&mut buf, x0 + c.s(8.0), baseline, &g.nick, size, true, ncol);
            for (li, ln) in g.lines.iter().enumerate() {
                let bl = baseline + li as f32 * line_h;
                let lx = if li == 0 { x0 + g.tx } else { x0 + c.s(8.0) };
                inp.fonts.draw(&mut buf, lx, bl, ln, size, false, rp.msg);
            }
            cy += g.row_h + feed_gap;
        }
        cy += c.s(2.0);
    }

    // ---- empty state ----
    if inp.users.is_empty() && !a.hide_empty && inp.whisperers.is_empty() {
        let icy = cy + c.s(34.0);
        let icx = px + panel_w * 0.5;
        buf.circle(icx, icy, c.s(19.0), inp.pal.row_hover);
        // simple people glyph
        buf.circle(icx - c.s(5.0), icy - c.s(3.0), c.s(4.5), inp.pal.icon.scale_a(160.0));
        buf.circle(icx + c.s(5.0), icy - c.s(3.0), c.s(4.5), inp.pal.icon.scale_a(160.0));
        buf.round_rect(icx - c.s(11.0), icy + c.s(3.0), c.s(9.0), c.s(7.0), c.s(4.5), inp.pal.icon.scale_a(160.0));
        buf.round_rect(icx + c.s(2.0), icy + c.s(3.0), c.s(9.0), c.s(7.0), c.s(4.5), inp.pal.icon.scale_a(160.0));
        let title = tr(inp.lang, "В канале никого нет", "Nobody in this channel");
        let sub = tr(inp.lang, "Дождитесь, пока кто-нибудь зайдёт на канал", "Waiting for someone to join");
        let ts = c.em(1.05);
        let tw = inp.fonts.measure(&title, ts, true);
        inp.fonts.draw(&mut buf, icx - tw * 0.5, icy + c.s(36.0), &title, ts, true, inp.pal.text.scale_a(216.0));
        let ss = c.em(0.82);
        let sw = inp.fonts.measure(&sub, ss, false);
        inp.fonts.draw(&mut buf, icx - sw * 0.5, icy + c.s(56.0), &sub, ss, false, inp.pal.muted);
    }
    if scroll_on {
        buf.clear_clip();
    }

    // ---- style texture overlays (paint-only) ----
    if !c.borderless {
    match c.sk {
        StyleKind::Crt | StyleKind::Retro => {
            // phosphor scanlines
            let mut yy = py + 1.0;
            while yy < py + panel_h - 1.0 {
                buf.fill_rect(px + 1.0, yy, panel_w - 2.0, 1.0, Rgba([0, 0, 0, 42]));
                yy += c.s(3.0);
            }
        }
        StyleKind::Hud => {
            // faint tactical grid + corner ticks
            let step = c.s(22.0);
            let mut gx = px + step;
            while gx < px + panel_w - 1.0 {
                buf.fill_rect(gx, py + 2.0, 1.0, panel_h - 4.0, inp.pal.accent.scale_a(10.0));
                gx += step;
            }
            let mut gy = py + step;
            while gy < py + panel_h - 1.0 {
                buf.fill_rect(px + 2.0, gy, panel_w - 4.0, 1.0, inp.pal.accent.scale_a(10.0));
                gy += step;
            }
            let (ins, ln, th) = (c.s(5.0), c.s(13.0), c.s(2.0));
            let col = inp.pal.accent.scale_a(160.0);
            for (cx0, cy0, sx, sy) in [
                (px + ins, py + ins, 1.0f32, 1.0),
                (px + panel_w - ins, py + ins, -1.0, 1.0),
                (px + ins, py + panel_h - ins, 1.0, -1.0),
                (px + panel_w - ins, py + panel_h - ins, -1.0, -1.0),
            ] {
                buf.fill_rect(cx0.min(cx0 + sx * ln), cy0.min(cy0 + sy * th), ln, th, col);
                buf.fill_rect(cx0.min(cx0 + sx * th), cy0.min(cy0 + sy * ln), th, ln, col);
            }
        }
        StyleKind::Neon | StyleKind::Cyber => {
            // inner glow frame
            buf.stroke_round_rect(px + 1.5, py + 1.5, panel_w - 3.0, panel_h - 3.0, pr - 1.0, inp.pal.accent.scale_a(55.0), 1.0);
        }
        StyleKind::Glass | StyleKind::Soft | StyleKind::Minimal | StyleKind::Ice | StyleKind::Lava => {}
    }
    }

    // keep the first talking row visible (auto-follow for capped panels)
    let mut scroll_to = scroll;
    if scroll_max > 0.0 {
        if let Some(i) = inp.users.iter().position(|u| u.talking) {
            let ry = rows_top + i as f32 * (row_h + row_gap);
            if ry < scroll {
                scroll_to = ry;
            } else if ry + row_h > scroll + content_h_total {
                scroll_to = (ry + row_h - content_h_total).clamp(0.0, scroll_max);
            }
        }
    }

    Frame { buf, hits, panel_w, panel_h, scroll_to }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::Icons;
    use crate::font::Fonts;
    use crate::theme::Palette;

    fn size_inp<'a>(a: &'a Appearance, pal: &'a Palette, fonts: &'a Fonts, icons: &'a Icons) -> FrameInput<'a> {
        FrameInput {
            appearance: a,
            pal,
            fonts,
            icons,
            status: "live",
            server_name: "S",
            channel_name: "C",
            user_count: 2,
            users: vec![
                { let mut u = crate::model::UserState::new("1", "A"); u.talking = true; u },
                crate::model::UserState::new("2", "B"),
            ],
            talk_alphas: vec![],
            class_keys: vec![None, None],
            whisper_keys: vec![],
            whisperers: vec![],
            feed: vec![],
            lang: "ru",
            move_mode: false,
            now_ms: 1000,
            roster_scroll: 0.0,
            work_h: 1080.0,
        }
    }

    #[test]
    fn panel_size_settings() {
        let fonts = Fonts::load();
        let icons = Icons::load();
        let pal = Palette::from_appearance(&Appearance::default());
        // panel width follows the setting
        let mut a = Appearance::default();
        a.panel_width = 400.0;
        let f = render_frame(&size_inp(&a, &pal, &fonts, &icons));
        assert_eq!(f.panel_w, 400.0);
        // max height caps the panel and auto-follow reports a scroll target
        a.panel_max_height = 120.0;
        let f2 = render_frame(&size_inp(&a, &pal, &fonts, &icons));
        assert!(f2.panel_h <= 120.0 + 1.0, "{}", f2.panel_h);
        assert!(f2.scroll_to >= 0.0);
    }

    #[test]
    fn class_matching_rules() {
        let groups = vec![(442i64, "Лучники Ордена".to_string()), (501, "Духи Крови".to_string())];
        let map = vec![("777".to_string(), "mage".to_string())];
        assert_eq!(class_key_for("442", &groups, &map).as_deref(), Some("archer"));
        assert_eq!(class_key_for("501", &groups, &map).as_deref(), Some("blood"));
        assert_eq!(class_key_for("777", &groups, &map).as_deref(), Some("mage"));
        assert_eq!(class_key_for("999", &groups, &map), None);
        // web parity: 'none' disables the MANUAL mapping but name matching
        // still applies (features.js falls through to nameMatches)
        let map2 = vec![("442".to_string(), "none".to_string())];
        assert_eq!(class_key_for("442", &groups, &map2).as_deref(), Some("archer"));
        let empty: Vec<(i64, String)> = vec![];
        assert_eq!(class_key_for("442", &empty, &map2), None, "'none' + no names = no class");
    }

    #[test]
    fn initials_rules() {
        assert_eq!(initials("Bob"), "B");
        assert_eq!(initials("Мыsha Петров"), "МП");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn frame_geometry_and_hits() {
        let a = Appearance::default();
        let pal = Palette::from_appearance(&a);
        let fonts = Fonts::load();
        let icons = Icons::load();
        let mut u1 = UserState::new("1", "Alice");
        u1.talking = true;
        u1.is_self = true;
        let mut u2 = UserState::new("2", "Bob");
        u2.input_muted = true;
        let inp = FrameInput {
            appearance: &a,
            pal: &pal,
            fonts: &fonts,
            icons: &icons,
            status: "live",
            server_name: "Eternals",
            channel_name: "General",
            user_count: 2,
            users: vec![u1, u2],
            talk_alphas: vec![],
            class_keys: vec![Some("archer".into()), None],
            whisper_keys: vec![],
            whisperers: {
                let mut w = UserState::new("3", "Мыsha");
                w.whispering = true;
                w.talking = true;
                vec![w]
            },
            feed: vec![FeedItem { nickname: "Alice".into(), text: "привет".into(), kind: 1 , alpha: 1.0 }],
            lang: "ru",
            move_mode: true,
            now_ms: 1000,
            work_h: 1080.0,
            roster_scroll: 0.0,
        };
        let frame = render_frame(&inp);
        assert_eq!(frame.panel_w, PANEL_W);
        assert!(frame.panel_h > 200.0 && frame.panel_h < 600.0, "panel_h={}", frame.panel_h);
        assert_eq!(frame.buf.w, (PANEL_W + SHADOW_PAD * 2.0) as i32);
        // move-mode hits: save + discard + drag
        let has = |h: Hit| frame.hits.iter().any(|(_, x)| *x == h);
        assert!(has(Hit::Save) && has(Hit::Discard) && has(Hit::Drag));
        // talking outline pixels exist (talk-outline green around row)
        // center of panel is opaque
        let i = ((frame.buf.h / 2 * frame.buf.w + frame.buf.w / 2) * 4) as usize;
        assert!(frame.buf.px[i + 3] > 150); // panel bg alpha (opacity 0.72)
    }

    #[test]
    fn frame_scales_with_scale_setting() {
        let mut a = Appearance::default();
        a.scale = 1.25;
        a.show_header = false;
        let pal = Palette::from_appearance(&a);
        let fonts = Fonts::load();
        let icons = Icons::load();
        let inp = FrameInput {
            appearance: &a,
            pal: &pal,
            fonts: &fonts,
            icons: &icons,
            status: "offline",
            server_name: "S",
            channel_name: "C",
            user_count: 0,
            users: vec![],
            talk_alphas: vec![],
            class_keys: vec![],
            whisper_keys: vec![],
            whisperers: vec![],
            feed: vec![],
            lang: "en",
            move_mode: false,
            now_ms: 0,
            work_h: 800.0,
            roster_scroll: 0.0,
        };
        let frame = render_frame(&inp);
        assert_eq!(frame.panel_w, PANEL_W * 1.25);
        assert!(frame.hits.is_empty());
    }
}

#[cfg(test)]
mod qa {
    use super::*;
    use crate::assets::Icons;
    use crate::font::Fonts;
    use crate::render::write_png;
    use crate::theme::Palette;

    #[test]
    fn qa_dump_frame() {
        let a = Appearance::default();
        let pal = Palette::from_appearance(&a);
        let fonts = Fonts::load();
        let icons = Icons::load();
        let mk = |id: &str, nick: &str, talk: bool, whisper: bool, mute: bool, self_: bool| {
            let mut u = UserState::new(id, nick);
            u.talking = talk;
            u.whispering = whisper;
            u.input_muted = mute;
            u.is_self = self_;
            u
        };
        let mut alice = mk("100", "Alice", true, false, false, false);
        alice.channel_commander = true;
        let mut me = mk("1", "Я", false, false, false, true);
        me.away = true;
        me.away_message = "ушёл на обед".into();
        let mut bob = mk("101", "Bob", false, false, true, false);
        bob.output_muted = true;
        bob.priority_speaker = true;
        let inp = FrameInput {
            appearance: &a,
            pal: &pal,
            fonts: &fonts,
            icons: &icons,
            status: "live",
            server_name: "Eternals",
            channel_name: "Генеральный штаб",
            user_count: 4,
            users: vec![alice, me, bob, mk("102", "Мыsha Петров", false, false, false, false)],
            talk_alphas: vec![],
            class_keys: vec![Some("archer".into()), Some("mage".into()), Some("warrior".into()), None],
            whisper_keys: vec![("103".into(), Some("ghost".into()))],
            whisperers: {
                let mut w = mk("103", "Ghost", true, true, false, false);
                w.whispering = true;
                vec![w]
            },
            feed: vec![
                FeedItem { nickname: "Мыsha".into(), text: "привет всем!".into(), kind: 1 , alpha: 1.0 },
                FeedItem { nickname: "Bob".into(), text: "вошёл в Генеральный штаб".into(), kind: 0 , alpha: 1.0 },
            ],
            lang: "ru",
            move_mode: true,
            now_ms: 1500,
            work_h: 1080.0,
            roster_scroll: 0.0,
        };
        let frame = render_frame(&inp);
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/qa");
        std::fs::create_dir_all(dir).unwrap();
        write_png(&format!("{dir}/frame.png"), &frame.buf).unwrap();

        let mut a2 = Appearance::default();
        a2.theme = "frost".into();
        a2.compact = true;
        let pal2 = Palette::from_appearance(&a2);
        let inp2 = FrameInput {
            appearance: &a2,
            pal: &pal2,
            fonts: &fonts,
            icons: &icons,
            status: "offline",
            server_name: "offline",
            channel_name: "—",
            user_count: 0,
            users: vec![],
            talk_alphas: vec![],
            class_keys: vec![],
            whisper_keys: vec![],
            whisperers: vec![],
            feed: vec![],
            lang: "en",
            move_mode: false,
            now_ms: 0,
            work_h: 1080.0,
            roster_scroll: 0.0,
        };
        let frame2 = render_frame(&inp2);
        write_png(&format!("{dir}/frame_empty_frost.png"), &frame2.buf).unwrap();

        // style variants (mockups 1..5) — same scene, per-style paint
        for (i, (st_id, _ru, _en)) in StyleKind::ALL.iter().enumerate() {
            let mut a3 = Appearance::default();
            a3.style = st_id.to_string();
            a3.accent = StyleKind::ACCENTS[i].to_string();
            let pal3 = Palette::from_appearance(&a3);
            let s_alice = {
                let mut u = mk("100", "Alice", true, false, false, false);
                u.channel_commander = true;
                u
            };
            let s_me = {
                let mut u = mk("1", "Я", false, false, false, true);
                u.away = true;
                u.away_message = "ушёл на обед".into();
                u
            };
            let s_bob = {
                let mut u = mk("101", "Bob", false, false, true, false);
                u.output_muted = true;
                u.priority_speaker = true;
                u
            };
            let inp3 = FrameInput {
                appearance: &a3,
                pal: &pal3,
                fonts: &fonts,
                icons: &icons,
                status: "live",
                server_name: "Eternals",
                channel_name: "Генеральный штаб",
                user_count: 4,
                users: vec![s_alice, s_me, s_bob, mk("102", "Мыsha Петров", false, false, false, false)],
                talk_alphas: vec![],
                class_keys: vec![Some("archer".into()), Some("mage".into()), Some("warrior".into()), None],
                whisper_keys: vec![("103".into(), Some("ghost".into()))],
                whisperers: {
                    let mut w = mk("103", "Ghost", true, true, false, false);
                    w.whispering = true;
                    vec![w]
                },
                feed: vec![
                    FeedItem { nickname: "Мыsha".into(), text: "привет всем!".into(), kind: 1 , alpha: 1.0 },
                    FeedItem { nickname: "Bob".into(), text: "вошёл в Генеральный штаб".into(), kind: 0 , alpha: 1.0 },
                ],
                lang: "ru",
                move_mode: true,
                now_ms: 1500,
                work_h: 1080.0,
                roster_scroll: 0.0,
            };
            let fr = render_frame(&inp3);
            write_png(&format!("{dir}/style-{st_id}.png"), &fr.buf).unwrap();
        }
    }
}
