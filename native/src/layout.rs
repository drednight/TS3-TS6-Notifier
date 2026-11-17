//! Overlay panel layout — mirrors `public/css/overlay.css` + `public/js/app.js`
//! row rules (v1.4.7 event rows, v1.4.9 talking-row outline, whisper linger).

use crate::assets::Icons;
use crate::config::Appearance;
use crate::font::Fonts;
use crate::model::UserState;
use crate::render::Buf;
use crate::theme::{hash_hue, hsl, mix, Palette, Rgba};

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
    Gear,
    Save,
    Discard,
}

/// One feed (events/chat) line.
pub struct FeedItem {
    pub nickname: String,
    pub text: String,
    pub kind: u8, // 0 = system event row, 1 = private chat, 2 = channel/server chat
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
}

impl<'a> Ctx<'a> {
    fn s(&self, v: f32) -> f32 {
        v * self.k
    }
    fn em(&self, em: f32) -> f32 {
        self.fs * em
    }
}

fn draw_you_chip(c: &Ctx, buf: &mut Buf, x: f32, y: f32, label: &str) -> f32 {
    let size = c.em(0.72);
    let tw = c.fonts.measure(label, size, true);
    let pad = c.s(5.0);
    let h = size * 1.8;
    buf.round_rect(x, y, tw + pad * 2.0, h, c.s(6.0), c.pal.accent.scale_a(41.0));
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
    you_label: &str,
    opts: &RowOpts,
) {
    let pal = c.pal;
    let radius = pal.radius as f32 * 0.5 * c.k;
    if u.talking && !whisper {
        buf.round_rect(x, y, w, h, radius, pal.talking_bg);
        // v1.4.9 «Обводка говорящего» — 2px outline of the whole row strip
        buf.stroke_round_rect(x, y, w, h, radius, pal.talk_outline, c.s(2.0));
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
            buf.fill_rect(x + w * t, y, w / steps as f32 + 1.0, h, pal.accent.scale_a(a as f64));
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
    if u.talking {
        let glow_a = (0.25 + 0.5 * pal.glow) as f32;
        buf.stroke_circle(cx, cy, av * 0.5 + c.s(1.0), pal.accent, c.s(2.0));
        for i in 0..3 {
            let g = (i as f32 + 1.0) * c.s(2.5);
            buf.stroke_circle(
                cx,
                cy,
                av * 0.5 + c.s(1.0) + g,
                pal.accent.scale_a((255.0 * glow_a * 0.16) as f64),
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
    let nick_color = if whisper {
        pal.accent
    } else if u.input_muted || u.away {
        pal.muted
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
    };
    let pad = SHADOW_PAD * k;
    let panel_w = ((a.panel_width as f32) * k).clamp(200.0 * k, 600.0 * k);
    let opts = RowOpts {
        show_avatars: a.show_avatars,
        show_badges: a.show_badges,
        eq_style: a.eq_style.clone(),
    };
    let you_label = tr(inp.lang, "Вы", "you");

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
        for _ in &inp.feed {
            y += feed_row_h + feed_gap;
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
    // drop shadow + panel body
    buf.shadow_round_rect(px, py, panel_w, panel_h, c.s(inp.pal.radius as f32), c.s(18.0), inp.pal.shadow);
    buf.round_rect(px, py, panel_w, panel_h, c.s(inp.pal.radius as f32), inp.pal.panel_bg);
    buf.stroke_round_rect(px, py, panel_w, panel_h, c.s(inp.pal.radius as f32), inp.pal.panel_border, 1.0);

    let mut cy = py;
    // ---- move handle (z-index 50 in css — drawn over header) ----
    if inp.move_mode {
        let bg = mix(inp.pal.accent, Rgba::rgb(12, 14, 26).with_a(235), 0.22);
        buf.round_rect(px, cy, panel_w, handle_h, c.s(inp.pal.radius as f32), bg);
        buf.stroke_round_rect(px, cy, panel_w, handle_h, c.s(inp.pal.radius as f32), inp.pal.accent.scale_a(178.0), 1.0);
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
        // accent wash (linear-gradient 115deg ≈ horizontal)
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
        buf.fill_rect(px, cy + header_h - 1.0, panel_w, 1.0, inp.pal.panel_border);
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
        buf.circle(dot_x, dot_y, c.s(4.5), dot_c);
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
        // right cluster: user count pill + gear
        let gear_s = c.s(28.0);
        let gx = px + panel_w - gear_s - c.s(8.0);
        let gy = cy + (header_h - gear_s) * 0.5;
        // gear pictogram
        let (gcx, gcy) = (gx + gear_s * 0.5, gy + gear_s * 0.5);
        buf.stroke_circle(gcx, gcy, c.s(4.6), inp.pal.icon, c.s(2.4));
        buf.stroke_circle(gcx, gcy, c.s(1.4), inp.pal.icon, c.s(1.6));
        for i in 0..8 {
            let ang = i as f32 * std::f32::consts::PI / 4.0;
            let (sn, cs) = (ang.sin(), ang.cos());
            buf.line(gcx + cs * c.s(5.6), gcy + sn * c.s(5.6), gcx + cs * c.s(6.9), gcy + sn * c.s(6.9), c.s(2.8), inp.pal.icon);
        }
        hits.push((Rect { x: gx, y: gy, w: gear_s, h: gear_s }, Hit::Gear));
        if a.show_user_count {
            let cnt = inp.user_count.to_string();
            let size = c.em(0.78);
            let tw = inp.fonts.measure(&cnt, size, true);
            let pw = tw + c.s(16.0);
            let ph = c.s(19.0);
            let pxx = gx - c.s(6.0) - pw;
            let pyy = cy + (header_h - ph) * 0.5;
            buf.round_rect(pxx, pyy, pw, ph, ph * 0.5, inp.pal.row_hover);
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
        + (if inp.feed.is_empty() { 0.0 } else { c.s(4.0) + inp.feed.len() as f32 * (feed_row_h + feed_gap) + c.s(2.0) })
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
        draw_user_row(&c, &mut buf, px + list_pad, cy, panel_w - list_pad * 2.0, row_h, u, key, false, &you_label, &opts);
        cy += row_h + row_gap;
    }
    cy = (cy - row_gap).max(0.0) + list_pad;

    // ---- whisper block ----
    if !inp.whisperers.is_empty() {
        let head = tr(inp.lang, "ШЁПОТ", "WHISPER");
        let hs = c.em(0.72);
        buf.round_rect(px + c.s(10.0), cy + whisper_head_h * 0.5 - c.s(1.0), c.s(3.0), c.s(3.0), c.s(1.5), inp.pal.accent);
        inp.fonts.draw(&mut buf, px + c.s(18.0), cy + whisper_head_h * 0.5 + hs * 0.36, &head, hs, true, inp.pal.accent);
        let tw = inp.fonts.measure(&head, hs, true);
        buf.fill_rect_vgrad(
            px + c.s(24.0) + tw,
            cy + whisper_head_h * 0.5 - c.s(0.5),
            panel_w - c.s(34.0) - tw,
            1.0,
            inp.pal.accent.scale_a(90.0),
            inp.pal.accent.scale_a(0.0),
        );
        cy += whisper_head_h + c.s(2.0);
        for u in &inp.whisperers {
            let key = inp.whisper_keys.iter().find(|(id, _)| *id == u.id).and_then(|(_, k)| k.as_deref());
            draw_user_row(&c, &mut buf, px + c.s(10.0), cy, panel_w - c.s(20.0), row_h, u, key, true, &you_label, &opts);
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

    // ---- feed (events + chat) ----
    if !inp.feed.is_empty() {
        cy += c.s(4.0);
        for item in &inp.feed {
            let y0 = cy;
            let x0 = px + c.s(10.0);
            let w0 = panel_w - c.s(20.0);
            if item.kind == 1 {
                buf.round_rect(x0, y0, w0, feed_row_h, c.s(10.0), inp.pal.accent.scale_a(26.0));
                buf.stroke_round_rect(x0, y0, w0, feed_row_h, c.s(10.0), inp.pal.accent.scale_a(115.0), 1.0);
            }
            let size = c.em(0.92);
            let baseline = y0 + feed_row_h * 0.5 + size * 0.35;
            let nick_w = inp.fonts.measure(&item.nickname, size, true);
            let nick = inp.fonts.truncate(&item.nickname, size, true, w0 * 0.42);
            inp.fonts.draw(&mut buf, x0 + c.s(8.0), baseline, &nick, size, true, inp.pal.nick);
            let tx = x0 + c.s(8.0) + nick_w + c.s(6.0);
            if item.kind == 0 {
                // minimal action text after the colored nick
                let rest = inp.fonts.truncate(&item.text, size, false, x0 + w0 - c.s(8.0) - tx);
                inp.fonts.draw(&mut buf, tx, baseline, &rest, size, false, inp.pal.msg);
            } else {
                let rest = inp.fonts.truncate(&format!("— {}", item.text), size, false, x0 + w0 - c.s(8.0) - tx);
                inp.fonts.draw(&mut buf, tx, baseline, &rest, size, false, inp.pal.msg);
            }
            cy += feed_row_h + feed_gap;
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
            class_keys: vec![Some("archer".into()), None],
            whisper_keys: vec![],
            whisperers: {
                let mut w = UserState::new("3", "Мыsha");
                w.whispering = true;
                w.talking = true;
                vec![w]
            },
            feed: vec![FeedItem { nickname: "Alice".into(), text: "привет".into(), kind: 1 }],
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
        assert!(has(Hit::Save) && has(Hit::Discard) && has(Hit::Drag) && has(Hit::Gear));
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
        assert!(frame.hits.iter().all(|(_, h)| *h == Hit::Gear) || frame.hits.is_empty());
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
            class_keys: vec![Some("archer".into()), Some("mage".into()), Some("warrior".into()), None],
            whisper_keys: vec![("103".into(), Some("ghost".into()))],
            whisperers: {
                let mut w = mk("103", "Ghost", true, true, false, false);
                w.whispering = true;
                vec![w]
            },
            feed: vec![
                FeedItem { nickname: "Мыsha".into(), text: "привет всем!".into(), kind: 1 },
                FeedItem { nickname: "Bob".into(), text: "вошёл в Генеральный штаб".into(), kind: 0 },
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
    }
}
