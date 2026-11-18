//! Native settings window — Концепт 3 «Превью-стек» (v1: «Вид» + preview).
//! Pure cross-platform model/render/input; the window host lives in overlay.rs.

use crate::assets::Icons;
use crate::config::{Appearance, DesktopPrefs};
use crate::font::Fonts;
use crate::layout::{render_frame, FeedItem, FrameInput};
use crate::model::UserState;
use crate::render::Buf;
use crate::theme::{parse_color, Palette, Rgba, StyleKind};

pub const WIN_W: f32 = 900.0;
/// Scrollable left-pane height (title 58 + bottom margin 16).
pub const PANE_H: f32 = WIN_H - 74.0;
pub const WIN_H: f32 = 640.0;

fn tr(lang: &str, ru: &str, en: &str) -> String {
    if lang == "en" {
        en.to_string()
    } else {
        ru.to_string()
    }
}

/// settings.js PRESETS — theme + color set + blur/opacity/glow.
pub const PRESETS: [(&str, &str, &str, &str, &str, &str, f64, f64, f64); 8] = [
    ("glass", "Стекло", "Glass", "#7c6cff", "#e8ecff", "#b6b6bd", 18.0, 0.72, 0.7),
    ("midnight", "Полночь", "Midnight", "#4f7dff", "#dfe6ff", "#98a2c0", 14.0, 0.85, 0.4),
    ("frost", "Иней", "Frost", "#38bdf8", "#0b2740", "#2b5a7d", 10.0, 0.9, 0.3),
    ("neon", "Неон", "Neon", "#ff2fd6", "#39ff9c", "#c9b8ff", 22.0, 0.66, 1.0),
    ("emerald", "Изумруд", "Emerald", "#10b981", "#eafff5", "#9fd8c2", 12.0, 0.8, 0.45),
    ("carmine", "Кармин", "Carmine", "#f43f5e", "#ffe4ec", "#e79ab0", 10.0, 0.85, 0.5),
    ("amber", "Янтарь", "Amber", "#f59e0b", "#fff1d0", "#dcbb80", 8.0, 0.88, 0.4),
    ("mono", "Моно", "Mono", "#e5e5e5", "#f4f4f4", "#b3b3b3", 6.0, 0.92, 0.15),
];

/// talkOutline per preset (8th value above is glow; outlines are separate).
pub fn preset_outline(theme: &str) -> &'static str {
    match theme {
        "midnight" => "#4f7dff",
        "frost" => "#0ea5e9",
        "neon" => "#39ff9c",
        "emerald" => "#34d399",
        "carmine" => "#ff5d8f",
        "amber" => "#ffb224",
        "mono" => "#cfcfcf",
        _ => "#57e389",
    }
}

pub const ACCENT_SWATCHES: [&str; 9] = [
    "#7c6cff", "#4f7dff", "#38bdf8", "#ff2fd6", "#10b981", "#f43f5e", "#f59e0b", "#e5e5e5", "#ffffff",
];

/* ---------------- model ---------------- */

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ctrl {
    Close,
    MoveOverlay,
    Preset(u8),
    Swatch(u8),
    Style(u8),
    Toggle(&'static str),
    Cycle(&'static str),
    Slider(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub rect: crate::layout::Rect,
    pub ctrl: Ctrl,
}

pub struct SettingsModel {
    pub app: Appearance,
    pub desktop: DesktopPrefs,
    pub lang: String,
    pub scroll: f32,
    pub dragging: Option<(&'static str, crate::layout::Rect)>, // slider key + track rect
    pub close_requested: bool,
    pub move_requested: bool,
    pub changed: bool,
    pub version: String,
}

impl SettingsModel {
    pub fn new(app: Appearance, version: String) -> SettingsModel {
        Self::new_full(app, DesktopPrefs::default(), version)
    }

    pub fn new_full(app: Appearance, desktop: DesktopPrefs, version: String) -> SettingsModel {
        let lang = if app.language == "en" { "en".to_string() } else { "ru".to_string() };
        SettingsModel {
            app,
            desktop,
            lang,
            scroll: 0.0,
            dragging: None,
            close_requested: false,
            move_requested: false,
            changed: false,
            version,
        }
    }

    fn touch(&mut self) {
        self.changed = true;
    }

    pub fn click(&mut self, ctrl: Ctrl, _x: f32, _y: f32) {
        match ctrl {
            Ctrl::Close => self.close_requested = true,
            Ctrl::MoveOverlay => self.move_requested = true,
            Ctrl::Preset(i) => {
                if let Some(p) = PRESETS.get(i as usize) {
                    self.app.theme = p.0.to_string();
                    self.app.accent = p.3.to_string();
                    self.app.nick_color = p.4.to_string();
                    self.app.msg_color = p.5.to_string();
                    self.app.talk_outline = preset_outline(p.0).to_string();
                    self.app.blur = p.6;
                    self.app.opacity = p.7;
                    self.app.glow = p.8;
                    self.touch();
                }
            }
            Ctrl::Swatch(i) => {
                if let Some(c) = ACCENT_SWATCHES.get(i as usize) {
                    self.app.accent = c.to_string();
                    self.touch();
                }
            }
            Ctrl::Style(i) => {
                if let Some(st) = StyleKind::ALL.get(i as usize) {
                    self.app.style = st.0.to_string();
                    if let Some(acc) = StyleKind::ACCENTS.get(i as usize) {
                        self.app.accent = acc.to_string();
                    }
                    self.touch();
                }
            }
            Ctrl::Toggle(key) => match key {
                "elevate" => {
                    self.desktop.elevate = !self.desktop.elevate;
                    self.touch();
                }
                "click_through" => {
                    self.desktop.click_through = !self.desktop.click_through;
                    self.touch();
                }
                _ => {
                    let v = toggle_mut(&mut self.app, key);
                    *v = !*v;
                    self.touch();
                }
            },
            Ctrl::Cycle(key) if key == "overlay_layer" => {
                let next = match self.desktop.overlay_layer.as_str() {
                    "panel" => "screen",
                    "screen" => "fullscreen",
                    _ => "panel",
                };
                self.desktop.overlay_layer = next.to_string();
                self.touch();
            }
            Ctrl::Cycle(key) => {
                cycle_advance(&mut self.app, key);
                self.touch();
            }
            Ctrl::Slider(_) => {}
        }
    }

    /// Mouse-down on a slider track starts a capture drag and applies the value.
    pub fn drag_begin(&mut self, x: f32, y: f32, regions: &[Region]) -> bool {
        for r in regions {
            if let Ctrl::Slider(key) = r.ctrl {
                if r.rect.contains(x, y) || (y >= r.rect.y - 10.0 && y <= r.rect.y + 16.0 && x >= r.rect.x && x <= r.rect.x + r.rect.w) {
                    self.dragging = Some((key, r.rect));
                    self.drag_to(x);
                    return true;
                }
            }
        }
        false
    }

    pub fn drag_end(&mut self) {
        self.dragging = None;
    }

    /// Called on drag move while a slider is captured.
    pub fn drag_to(&mut self, x: f32) {
        if let Some((key, rect)) = self.dragging {
            let t = ((x - rect.x) / rect.w).clamp(0.0, 1.0) as f64;
            let (min, max) = slider_range(key);
            let v = min + (max - min) * t;
            set_slider(&mut self.app, key, v);
            self.touch();
        }
    }

    pub fn wheel(&mut self, dy: f32, content_h: f32) {
        let max_scroll = (content_h - PANE_H).max(0.0);
        self.scroll = (self.scroll - dy).clamp(0.0, max_scroll);
    }
}

fn toggle_mut<'a>(app: &'a mut Appearance, key: &str) -> &'a mut bool {
    match key {
        "show_header" => &mut app.show_header,
        "show_server_name" => &mut app.show_server_name,
        "show_user_count" => &mut app.show_user_count,
        "show_avatars" => &mut app.show_avatars,
        "show_badges" => &mut app.show_badges,
        "show_self" => &mut app.show_self,
        "hide_query" => &mut app.hide_query,
        "animations" => &mut app.animations,
        "talking_first" => &mut app.talking_first,
        "only_talking" => &mut app.only_talking,
        "compact" => &mut app.compact,
        "hide_empty" => &mut app.hide_empty,
        "show_whispers" => &mut app.show_whispers,
        "show_chat" => &mut app.show_chat,
        "show_events" => &mut app.show_events,
        "show_ev_join" => &mut app.show_ev_join,
        "show_ev_leave" => &mut app.show_ev_leave,
        "show_ev_move" => &mut app.show_ev_move,
        "show_ev_poke" => &mut app.show_ev_poke,
        "show_ev_chat" => &mut app.show_ev_chat,
        _ => &mut app.show_header,
    }
}

fn cycle_advance(app: &mut Appearance, key: &str) {
    match key {
        "theme" => {
            let themes = ["glass", "midnight", "frost", "neon", "emerald", "carmine", "amber", "mono"];
            let i = themes.iter().position(|t| *t == app.theme).unwrap_or(0);
            app.theme = themes[(i + 1) % themes.len()].to_string();
        }
        "corner" => {
            let corners = ["tl", "tr", "bl", "br"];
            let i = corners.iter().position(|t| *t == app.corner).unwrap_or(0);
            app.corner = corners[(i + 1) % corners.len()].to_string();
        }
        "language" => {
            app.language = match app.language.as_str() {
                "auto" => "ru".into(),
                "ru" => "en".into(),
                _ => "auto".into(),
            };
        }
        "eq_style" => {
            let styles = ["bars", "ring", "dots", "none"];
            let i = styles.iter().position(|t| *t == app.eq_style).unwrap_or(0);
            app.eq_style = styles[(i + 1) % styles.len()].to_string();
        }
        _ => {}
    }
}

fn slider_range(key: &str) -> (f64, f64) {
    match key {
        "scale" => (0.75, 1.5),
        "font_size" => (11.0, 18.0),
        "panel_width" => (200.0, 600.0),
        "panel_max_height" => (0.0, 1000.0),
        "opacity" => (0.15, 1.0),
        "glow" => (0.0, 1.0),
        "blur" => (0.0, 40.0),
        "radius" => (0.0, 26.0),
        _ => (0.0, 200.0), // margin_x / margin_y
    }
}

fn set_slider(app: &mut Appearance, key: &str, v: f64) {
    let (min, max) = slider_range(key);
    let v = v.clamp(min, max);
    match key {
        "scale" => app.scale = v,
        "font_size" => app.font_size = v.round().clamp(min, max),
        "panel_width" => app.panel_width = v.round().clamp(min, max),
        "panel_max_height" => app.panel_max_height = v.round().clamp(min, max),
        "opacity" => app.opacity = v,
        "glow" => app.glow = v,
        "blur" => app.blur = v,
        "radius" => app.radius = v,
        "margin_x" => app.margin_x = v,
        _ => app.margin_y = v,
    }
}

fn get_slider(app: &Appearance, key: &str) -> f64 {
    match key {
        "scale" => app.scale,
        "font_size" => app.font_size,
        "panel_width" => app.panel_width,
        "panel_max_height" => app.panel_max_height,
        "opacity" => app.opacity,
        "glow" => app.glow,
        "blur" => app.blur,
        "radius" => app.radius,
        "margin_x" => app.margin_x,
        _ => app.margin_y,
    }
}

fn get_toggle(app: &Appearance, key: &str) -> bool {
    match key {
        "show_header" => app.show_header,
        "show_server_name" => app.show_server_name,
        "show_user_count" => app.show_user_count,
        "show_avatars" => app.show_avatars,
        "show_badges" => app.show_badges,
        "show_self" => app.show_self,
        "hide_query" => app.hide_query,
        "animations" => app.animations,
        "talking_first" => app.talking_first,
        "only_talking" => app.only_talking,
        "compact" => app.compact,
        "hide_empty" => app.hide_empty,
        "show_whispers" => app.show_whispers,
        "show_chat" => app.show_chat,
        "show_events" => app.show_events,
        "show_ev_join" => app.show_ev_join,
        "show_ev_leave" => app.show_ev_leave,
        "show_ev_move" => app.show_ev_move,
        "show_ev_poke" => app.show_ev_poke,
        _ => app.show_ev_chat,
    }
}

/* ---------------- rendering ---------------- */

struct Ctx<'a> {
    pal: &'a Palette,
    fonts: &'a Fonts,
    fs: f32,
}

/// Split text into lines that fit `max_w` (word wrap).
fn wrap_text(fonts: &crate::font::Fonts, text: &str, size: f32, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if fonts.measure(&candidate, size, false) <= max_w || cur.is_empty() {
            cur = candidate;
        } else {
            lines.push(cur);
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Glass mockup icon glyph (kind 0..7), centered at (cx, cy).
fn draw_icon(b: &mut Buf, cx: f32, cy: f32, s: f32, kind: u8, col: Rgba, bg: Rgba) {
    match kind {
        0 => b.stroke_circle(cx, cy, s, col, 2.0),                    // glass ring
        1 => {                                                        // crescent
            b.circle(cx, cy, s, col);
            b.circle(cx + s * 0.5, cy - s * 0.3, s * 0.85, bg);
        }
        2 => {                                                        // snowflake
            b.line(cx - s, cy, cx + s, cy, 1.8, col);
            b.line(cx - s * 0.5, cy - s * 0.87, cx + s * 0.5, cy + s * 0.87, 1.8, col);
            b.line(cx - s * 0.5, cy + s * 0.87, cx + s * 0.5, cy - s * 0.87, 1.8, col);
        }
        3 => {                                                        // bolt
            b.line(cx + s * 0.35, cy - s, cx - s * 0.35, cy + s * 0.15, 2.0, col);
            b.line(cx - s * 0.35, cy + s * 0.15, cx + s * 0.25, cy + s * 0.15, 2.0, col);
            b.line(cx + s * 0.25, cy + s * 0.15, cx - s * 0.2, cy + s, 2.0, col);
        }
        4 => {                                                        // leaf / diamond
            b.line(cx, cy - s, cx + s, cy, 2.0, col);
            b.line(cx + s, cy, cx, cy + s, 2.0, col);
            b.line(cx, cy + s, cx - s, cy, 2.0, col);
            b.line(cx - s, cy, cx, cy - s, 2.0, col);
        }
        5 => {                                                        // drop
            b.circle(cx, cy + s * 0.35, s * 0.62, col);
            b.line(cx, cy - s, cx - s * 0.55, cy + s * 0.25, 2.0, col);
            b.line(cx, cy - s, cx + s * 0.55, cy + s * 0.25, 2.0, col);
        }
        6 => {                                                        // sun
            b.stroke_circle(cx, cy, s * 0.55, col, 2.0);
            b.line(cx - s, cy, cx + s, cy, 1.8, col);
            b.line(cx, cy - s, cx, cy + s, 1.8, col);
            b.line(cx - s * 0.7, cy - s * 0.7, cx + s * 0.7, cy + s * 0.7, 1.6, col);
            b.line(cx - s * 0.7, cy + s * 0.7, cx + s * 0.7, cy - s * 0.7, 1.6, col);
        }
        _ => b.stroke_round_rect(cx - s, cy - s, s * 2.0, s * 2.0, 2.0, col, 2.0), // mono square
    }
}

/// Preset tile (icon above label), Glass mockup.
fn draw_tile(c: &Ctx, buf: &mut Buf, x: f32, y: f32, w: f32, h: f32, label: &str, icon: u8, active: bool) {
    let bg = if active {
        c.pal.accent.scale_a(36.0)
    } else {
        crate::theme::mix(c.pal.panel_bg, Rgba::rgb(255, 255, 255), 0.05)
    };
    buf.round_rect(x, y, w, h, 12.0, bg);
    let border = if active { c.pal.accent } else { c.pal.panel_border };
    buf.stroke_round_rect(x, y, w, h, 12.0, border, if active { 1.8 } else { 1.0 });
    let icol = if active { c.pal.accent } else { c.pal.icon };
    draw_icon(buf, x + w * 0.5, y + 18.0, 6.5, icon, icol, bg);
    let tw = c.fonts.measure(label, 12.5, true);
    c.fonts.draw(buf, x + (w - tw) * 0.5, y + h - 11.0, label, 12.5, true, c.pal.text);
}

/// Accent pill button («Переместить оверлей»).
fn draw_button(c: &Ctx, buf: &mut Buf, x: f32, y: f32, w: f32, h: f32, label: &str) {
    buf.round_rect(x, y, w, h, 12.0, c.pal.accent.scale_a(235.0));
    let tw = c.fonts.measure(label, 13.5, true);
    c.fonts.draw(buf, x + (w - tw) * 0.5, y + h * 0.5 + 5.0, label, 13.5, true, Rgba::rgb(255, 255, 255));
}

/// Small pin glyph on the preview card.
fn draw_pin(c: &Ctx, buf: &mut Buf, x: f32, y: f32) {
    buf.circle(x, y, 3.2, c.pal.muted);
    buf.line(x, y + 3.0, x + 5.0, y + 8.0, 1.6, c.pal.muted);
}

fn draw_toggle(c: &Ctx, buf: &mut Buf, x: f32, y: f32, w: f32, label: &str, on: bool) {
    c.fonts.draw(buf, x, y + 15.0, label, c.fs, false, c.pal.text);
    let (tw, th) = (42.0, 24.0);
    let tx = x + w - tw;
    let ty = y + 1.0;
    let bg = if on {
        c.pal.accent.scale_a(220.0)
    } else {
        crate::theme::mix(c.pal.panel_bg, Rgba::rgb(0, 0, 0), 0.35)
    };
    buf.round_rect(tx, ty, tw, th, th * 0.5, bg);
    let kx = if on { tx + tw - th * 0.5 } else { tx + th * 0.5 };
    buf.circle(kx, ty + th * 0.5, th * 0.38, Rgba::rgb(255, 255, 255));
}

/// Slider row: label left, value right (accent), track + in-bounds knob below.
fn draw_slider(c: &Ctx, buf: &mut Buf, x: f32, y: f32, w: f32, label: &str, value_str: &str, value: f64, range: (f64, f64)) -> crate::layout::Rect {
    c.fonts.draw(buf, x, y + 12.0, label, c.fs, false, c.pal.text);
    let vw = c.fonts.measure(value_str, c.fs, true);
    c.fonts.draw(buf, x + w - vw, y + 12.0, value_str, c.fs, true, c.pal.accent);
    let track = crate::layout::Rect { x, y: y + 24.0, w, h: 8.0 };
    buf.round_rect(track.x, track.y, track.w, track.h, 4.0, c.pal.row_hover);
    let t = ((value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0) as f32;
    let kx = track.x + 8.0 + (track.w - 16.0) * t;
    buf.round_rect(track.x, track.y, (kx - track.x).max(4.0), track.h, 4.0, c.pal.accent);
    buf.circle(kx, track.y + 4.0, 8.0, Rgba::rgb(255, 255, 255));
    buf.stroke_circle(kx, track.y + 4.0, 8.0, c.pal.accent.scale_a(120.0), 1.2);
    track
}

fn draw_cycle(c: &Ctx, buf: &mut Buf, x: f32, y: f32, w: f32, label: &str, value: &str) {
    c.fonts.draw(buf, x, y + 15.0, label, c.fs, false, c.pal.text);
    let bw = 140.0;
    let bx = x + w - bw;
    buf.round_rect(bx, y, bw, 24.0, 8.0, c.pal.row_hover);
    let tw = c.fonts.measure(value, c.fs, true);
    c.fonts.draw(buf, bx + (bw - tw) * 0.5, y + 17.0, value, c.fs, true, c.pal.text);
}

/// Preview sample — shows the design regardless of live state.
fn sample_input<'a>(
    a: &'a Appearance,
    pal: &'a Palette,
    fonts: &'a Fonts,
    icons: &'a Icons,
) -> FrameInput<'a> {
    let mut alice = UserState::new("100", "Alice");
    alice.talking = true;
    alice.channel_commander = true;
    let mut me = UserState::new("1", "Я");
    me.is_self = true;
    me.away = true;
    me.away_message = "ушёл на обед".into();
    let mut bob = UserState::new("101", "Bob");
    bob.input_muted = true;
    bob.priority_speaker = true;
    let mut ghost = UserState::new("103", "Ghost");
    ghost.whispering = true;
    ghost.talking = true;
    ghost.whisper_channel = Some("Другая".into());
    ghost.away = true;
    ghost.away_message = "Шёпот из канала «Другая»".into();
    FrameInput {
        appearance: a,
        pal,
        fonts,
        icons,
        status: "live",
        server_name: "Eternals",
        channel_name: "Генеральный штаб",
        user_count: 4,
        users: vec![alice, me, bob],
        class_keys: vec![Some("archer".into()), Some("mage".into()), Some("warrior".into())],
        whisper_keys: vec![("103".into(), Some("ghost".into()))],
        whisperers: vec![ghost],
        feed: vec![FeedItem { nickname: "Мыsha".into(), text: "привет!".into(), kind: 1 }],
        lang: "ru",
        move_mode: false,
        now_ms: 1500,
        roster_scroll: 0.0,
        work_h: WIN_H,
    }
}

/// Render the settings window; returns the buffer + interactive regions + content height.
pub fn render(m: &SettingsModel, fonts: &Fonts, icons: &Icons) -> (Buf, Vec<Region>, f32) {
    let pal = Palette::from_appearance(&m.app);
    let c = Ctx { pal: &pal, fonts, fs: 13.0 };
    let mut buf = Buf::new(WIN_W as i32, WIN_H as i32);
    let mut regions: Vec<Region> = Vec::new();

    // window background — Glass: subtle vertical gradient + border
    let mut bg = pal.panel_bg;
    bg.0[3] = 255;
    let top = crate::theme::mix(bg, Rgba::rgb(0, 0, 0), 0.12);
    let bottom = crate::theme::mix(bg, Rgba::rgb(255, 255, 255), 0.035);
    buf.fill_rect_vgrad(0.0, 0.0, WIN_W, WIN_H, top, bottom);
    buf.stroke_round_rect(0.0, 0.0, WIN_W, WIN_H, 12.0, pal.panel_border, 1.0);

    // ---- title bar (Glass: version pill next to the title, round close) ----
    let title = tr(&m.lang, "Настройки", "Settings");
    let full = format!("TS3-TS6 Notifier — {title}");
    c.fonts.draw(&mut buf, 20.0, 28.0, &full, 15.0, true, pal.text);
    let ver = format!("v{}", m.version);
    let tw_full = c.fonts.measure(&full, 15.0, true);
    let vw = c.fonts.measure(&ver, 11.0, true);
    let vpx = 20.0 + tw_full + 10.0;
    buf.round_rect(vpx, 13.0, vw + 16.0, 20.0, 10.0, pal.row_hover);
    c.fonts.draw(&mut buf, vpx + 8.0, 27.0, &ver, 11.0, true, pal.muted);
    let (cx, cy) = (WIN_W - 42.0, 25.0);
    buf.circle(cx, cy, 15.0, pal.row_hover);
    buf.line(cx - 5.0, cy - 5.0, cx + 5.0, cy + 5.0, 2.2, pal.text);
    buf.line(cx + 5.0, cy - 5.0, cx - 5.0, cy + 5.0, 2.2, pal.text);
    regions.push(Region {
        rect: crate::layout::Rect { x: cx - 15.0, y: cy - 15.0, w: 30.0, h: 30.0 },
        ctrl: Ctrl::Close,
    });
    buf.fill_rect(0.0, 44.0, WIN_W, 1.0, pal.panel_border);

    // ---- left pane (scrollable) ----
    let pane_x = 16.0;
    let pane_w = 452.0;
    let pane_y = 58.0;
    let pane_h = PANE_H;
    buf.set_clip(pane_x, pane_y, pane_w, pane_h);
    let mut y = pane_y - m.scroll;

    let section = |buf: &mut Buf, c: &Ctx, y: f32, text: &str| {
        c.fonts.draw(buf, pane_x + 4.0, y + 16.0, text, 14.0, true, c.pal.accent);
        buf.fill_rect(pane_x + 4.0, y + 24.0, pane_w - 8.0, 1.0, c.pal.panel_border);
    };

    section(&mut buf, &c, y, &tr(&m.lang, "Стиль панели", "Panel style"));
    y += 34.0;
    {
        let icons = [0u8, 3, 7, 5, 6];
        let n = StyleKind::ALL.len() as f32;
        let tile_w = (pane_w - 8.0 - (n - 1.0) * 8.0) / n;
        for (i, st) in StyleKind::ALL.iter().enumerate() {
            let bx = pane_x + 4.0 + i as f32 * (tile_w + 8.0);
            let active = m.app.style == st.0;
            let label = tr(&m.lang, st.1, st.2);
            draw_tile(&c, &mut buf, bx, y, tile_w, 56.0, &label, icons[i], active);
            regions.push(Region {
                rect: crate::layout::Rect { x: bx, y, w: tile_w, h: 56.0 },
                ctrl: Ctrl::Style(i as u8),
            });
        }
        y += 62.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "Пресеты", "Presets"));
    y += 34.0;
    for (i, p) in PRESETS.iter().enumerate() {
        let col = (i % 2) as f32;
        let row = (i / 2) as f32;
        let bx = pane_x + 4.0 + col * (pane_w * 0.5 - 6.0);
        let by = y + row * 62.0;
        let bw = pane_w * 0.5 - 14.0;
        let active = m.app.theme == p.0;
        let label = tr(&m.lang, p.1, p.2);
        draw_tile(&c, &mut buf, bx, by, bw, 56.0, &label, i as u8, active);
        regions.push(Region {
            rect: crate::layout::Rect { x: bx, y: by, w: bw, h: 56.0 },
            ctrl: Ctrl::Preset(i as u8),
        });
    }
    y += 254.0;

    section(&mut buf, &c, y, &tr(&m.lang, "Цвета", "Colors"));
    y += 34.0;
    for (i, sw) in ACCENT_SWATCHES.iter().enumerate() {
        let sx = pane_x + 4.0 + i as f32 * 34.0;
        let active = m.app.accent.eq_ignore_ascii_case(sw);
        buf.round_rect(sx, y, 26.0, 26.0, 13.0, parse_color(sw));
        if active {
            buf.stroke_round_rect(sx - 2.0, y - 2.0, 30.0, 30.0, 15.0, pal.text.scale_a(230.0), 2.0);
        }
        regions.push(Region {
            rect: crate::layout::Rect { x: sx, y, w: 26.0, h: 26.0 },
            ctrl: Ctrl::Swatch(i as u8),
        });
    }
    y += 36.0;
    {
        let names_ru = ["Фиолетовый", "Синий", "Голубой", "Пурпурный", "Изумрудный", "Розовый", "Янтарный", "Серебряный", "Белый"];
        let names_en = ["Violet", "Blue", "Sky", "Magenta", "Emerald", "Rose", "Amber", "Silver", "White"];
        let idx = ACCENT_SWATCHES
            .iter()
            .position(|s2| m.app.accent.eq_ignore_ascii_case(s2))
            .unwrap_or(0);
        let cname = tr(&m.lang, names_ru[idx], names_en[idx]);
        let cap = format!("{}: {} ({})", tr(&m.lang, "Цвет", "Color"), cname, m.app.accent);
        c.fonts.draw(&mut buf, pane_x + 4.0, y + 12.0, &cap, 11.5, false, pal.muted);
        y += 26.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "Геометрия и стиль", "Geometry & style"));
    y += 34.0;
    let sliders: [(&str, &str, &str); 10] = [
        ("scale", "Масштаб", "Scale"),
        ("font_size", "Шрифт", "Font size"),
        ("panel_width", "Ширина панели", "Panel width"),
        ("panel_max_height", "Максимальная высота", "Max height"),
        ("opacity", "Прозрачность", "Opacity"),
        ("glow", "Свечение", "Glow"),
        ("blur", "Размытие", "Blur"),
        ("radius", "Скругление", "Corner radius"),
        ("margin_x", "Отступ X", "Margin X"),
        ("margin_y", "Отступ Y", "Margin Y"),
    ];
    for (key, ru, en) in sliders {
        let label = tr(&m.lang, ru, en);
        let value = get_slider(&m.app, key);
        let int_unit = match key {
            "font_size" | "blur" | "radius" | "margin_x" | "margin_y" | "panel_width" | "panel_max_height" => true,
            _ => false,
        };
        let vstr = if key == "panel_max_height" && value <= 0.0 {
            tr(&m.lang, "авто", "auto")
        } else if int_unit {
            format!("{value:.0}")
        } else {
            format!("{value:.2}")
        };
        let track = draw_slider(
            &c,
            &mut buf,
            pane_x + 4.0,
            y,
            pane_w - 8.0,
            &label,
            &vstr,
            value,
            slider_range(key),
        );
        regions.push(Region { rect: track, ctrl: Ctrl::Slider(key) });
        y += 52.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "Расположение и вид", "Placement & look"));
    y += 34.0;
    for (key, ru, en) in [
        ("theme", "Тема", "Theme"),
        ("corner", "Угол", "Corner"),
        ("language", "Язык", "Language"),
        ("eq_style", "Индикатор", "Talking indicator"),
    ] {
        let label = tr(&m.lang, ru, en);
        let value = match key {
            "theme" => m.app.theme.as_str(),
            "corner" => m.app.corner.as_str(),
            "language" => m.app.language.as_str(),
            _ => m.app.eq_style.as_str(),
        };
        draw_cycle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, value);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + pane_w - 144.0, y, w: 140.0, h: 24.0 },
            ctrl: Ctrl::Cycle(key),
        });
        y += 36.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "Показывать", "Show"));
    y += 34.0;
    let toggles: [(&str, &str, &str); 20] = [
        ("show_header", "Шапка", "Header"),
        ("show_server_name", "Имя сервера", "Server name"),
        ("show_user_count", "Счётчик", "User count"),
        ("show_avatars", "Аватарки", "Avatars"),
        ("show_badges", "Бейджи", "Badges"),
        ("show_self", "Себя", "Self"),
        ("hide_query", "Скрыть query", "Hide query"),
        ("animations", "Анимации", "Animations"),
        ("talking_first", "Говорящие первыми", "Talking first"),
        ("only_talking", "Только говорящие", "Only talking"),
        ("compact", "Компактный", "Compact"),
        ("hide_empty", "Скрыть пустой", "Hide empty"),
        ("show_whispers", "Шёпот", "Whispers"),
        ("show_chat", "Чат", "Chat"),
        ("show_events", "События", "Events"),
        ("show_ev_join", "События: входы", "Events: joins"),
        ("show_ev_leave", "События: выходы", "Events: leaves"),
        ("show_ev_move", "События: переходы", "Events: moves"),
        ("show_ev_poke", "События: поке", "Events: pokes"),
        ("show_ev_chat", "События: чат", "Events: chat"),
    ];
    for (key, ru, en) in toggles {
        let label = tr(&m.lang, ru, en);
        let on = get_toggle(&m.app, key);
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 26.0 },
            ctrl: Ctrl::Toggle(key),
        });
        y += 30.0;
    }
    // «Система» — desktop prefs (layer + UAC + click-through)
    y += 16.0;
    {
        let label = tr(&m.lang, "Система", "System");
        c.fonts.draw(&mut buf, pane_x + 4.0, y, &label, 13.0, true, pal.accent);
        y += 26.0;
    }
    {
        let label = tr(&m.lang, "Поверх всех окон", "Over all windows");
        let value = match m.desktop.overlay_layer.as_str() {
            "panel" => tr(&m.lang, "Панель", "Panel"),
            "screen" => tr(&m.lang, "Во весь экран", "Full screen"),
            _ => tr(&m.lang, "Полноэкранное", "Fullscreen"),
        };
        draw_cycle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &value);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + pane_w - 144.0, y, w: 140.0, h: 24.0 },
            ctrl: Ctrl::Cycle("overlay_layer"),
        });
        y += 36.0;
    }
    let sys_rows: [(&str, &str, &str); 2] = [
        ("elevate", "Запуск от имени администратора", "Run as administrator"),
        ("click_through", "Мышь проходит сквозь оверлей", "Mouse passes through the overlay"),
    ];
    for (key, ru, en) in sys_rows {
        let label = tr(&m.lang, ru, en);
        let on = if key == "elevate" { m.desktop.elevate } else { m.desktop.click_through };
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 26.0 },
            ctrl: Ctrl::Toggle(key),
        });
        y += 30.0;
    }
    let content_h = y + m.scroll - pane_y;
    buf.clear_clip();
    // scroll indicator
    if content_h > pane_h {
        let frac = pane_h / content_h;
        let bar_h = (pane_h * frac).max(24.0);
        let bar_y = pane_y + (m.scroll / content_h) * (pane_h - bar_h);
        buf.round_rect(pane_x + pane_w - 4.0, bar_y, 3.0, bar_h, 1.5, pal.muted.scale_a(120.0));
    }

    // ---- right pane: pinned live preview (Glass) ----
    let pv_x = pane_x + pane_w + 16.0;
    let pv_w = WIN_W - pv_x - 16.0;
    let btn_y = WIN_H - 100.0;
    let card_h = btn_y - 16.0 - pane_y;
    buf.round_rect(pv_x, pane_y, pv_w, card_h, 12.0, crate::theme::mix(pal.panel_bg, Rgba::rgb(255, 255, 255), 0.05));
    buf.stroke_round_rect(pv_x, pane_y, pv_w, card_h, 12.0, pal.panel_border, 1.0);
    let cap = tr(&m.lang, "Предпросмотр", "Preview");
    c.fonts.draw(&mut buf, pv_x + 14.0, pane_y + 23.0, &cap, 13.0, true, pal.text);
    draw_pin(&c, &mut buf, pv_x + pv_w - 22.0, pane_y + 18.0);

    let sample = sample_input(&m.app, &pal, fonts, icons);
    let frame = render_frame(&sample);
    let avail_w = pv_w - 28.0;
    let avail_h = card_h - 52.0;
    let fit = (avail_w / frame.panel_w).min(avail_h / frame.panel_h).min(1.0);
    let dw = frame.panel_w * fit;
    let dh = frame.panel_h * fit;
    buf.set_clip(pv_x + 1.0, pane_y + 32.0, pv_w - 2.0, card_h - 34.0);
    buf.blit_rgba(
        pv_x + (pv_w - dw) * 0.5,
        pane_y + 38.0,
        dw,
        dh,
        &frame.buf.px,
        frame.buf.w,
        frame.buf.h,
        false,
    );
    buf.clear_clip();

    // move button + hints under the card (mockup-1 layout, no bottom bar)
    let label = tr(&m.lang, "Переместить оверлей", "Move overlay");
    draw_button(&c, &mut buf, pv_x, btn_y, pv_w, 46.0, &label);
    regions.push(Region {
        rect: crate::layout::Rect { x: pv_x, y: btn_y, w: pv_w, h: 46.0 },
        ctrl: Ctrl::MoveOverlay,
    });
    let hint1 = tr(&m.lang, "Перетащите оверлей по экрану.", "Drag the overlay across the screen.");
    let hint2 = tr(&m.lang, "Изменения применяются сразу", "Changes apply immediately");
    let h1w = c.fonts.measure(&hint1, 11.5, false);
    let h2w = c.fonts.measure(&hint2, 11.5, false);
    c.fonts.draw(&mut buf, pv_x + (pv_w - h1w) * 0.5, btn_y + 64.0, &hint1, 11.5, false, pal.muted);
    c.fonts.draw(&mut buf, pv_x + (pv_w - h2w) * 0.5, btn_y + 82.0, &hint2, 11.5, false, pal.muted);

    (buf, regions, content_h)
}

/* ---------------- window runner ---------------- */

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::http::App;
use crate::overlay::{Ev, Host};

static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);

/// Open the settings window at most once («окно настроек открывается один раз»).
pub fn spawn_once(app: Arc<App>, port: u16) {
    if SETTINGS_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = port;
    #[cfg(not(any(windows, tsn_win32)))]
    let _ = &app;
    std::thread::Builder::new()
        .name("settings".into())
        .spawn(move || {
            #[cfg(any(windows, tsn_win32))]
            crate::overlay::win32::run_settings(app.clone());
            SETTINGS_OPEN.store(false, Ordering::SeqCst);
        })
        .ok();
}

fn persist(app: &Arc<App>, a: &Appearance, d: &DesktopPrefs) {
    let patch = crate::json::obj(vec![
        ("appearance", crate::config::appearance_to_json(a)),
        (
            "desktop",
            crate::json::obj(vec![
                ("elevate", crate::json::Json::b(d.elevate)),
                ("clickThrough", crate::json::Json::b(d.click_through)),
                ("overlayLayer", crate::json::Json::s(d.overlay_layer.clone())),
            ]),
        ),
    ]);
    app.store.lock().unwrap().update(&patch);
    app.broadcast(&crate::json::obj(vec![
        ("type", crate::json::Json::s("config")),
        ("config", app.config_json()),
    ]));
}

/// Settings event loop (cross-platform; the host presents + pumps input).
pub fn run_loop(m: &mut SettingsModel, app: &Arc<App>, fonts: &Fonts, icons: &Icons, host: &mut dyn Host) {
    let work = host.work_area();
    let x = work.0 + (work.2 - WIN_W as f64) * 0.5;
    let y = work.1 + (work.3 - WIN_H as f64) * 0.5;
    loop {
        let (buf, regions, content_h) = render(m, fonts, icons);
        host.present(&buf, WIN_W, WIN_H, x, y);
        host.set_interactive(true);
        for ev in host.pump() {
            match ev {
                Ev::Click(ex, ey) => {
                    for r in &regions {
                        if r.rect.contains(ex, ey) {
                            if matches!(r.ctrl, Ctrl::Slider(_)) {
                                continue;
                            }
                            m.click(r.ctrl, ex, ey);
                            break;
                        }
                    }
                }
                Ev::DragStart(ex, ey) => {
                    m.drag_begin(ex, ey, &regions);
                }
                Ev::DragMove(ex, _) => m.drag_to(ex),
                Ev::DragEnd => m.drag_end(),
                Ev::Wheel(dy) => m.wheel(dy, content_h),
            }
        }
        if m.move_requested {
            m.move_requested = false;
            app.move_mode.store(true, Ordering::SeqCst);
            app.broadcast(&crate::json::obj(vec![
                ("type", crate::json::Json::s("move-mode")),
                ("active", crate::json::Json::b(true)),
            ]));
        }
        if m.changed {
            m.changed = false;
            persist(app, &m.app, &m.desktop);
        }
        if m.close_requested {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> SettingsModel {
        SettingsModel::new(Appearance::default(), "0.1.0".into())
    }

    #[test]
    fn preset_apply_matches_settings_js() {
        let mut m = model();
        m.click(Ctrl::Preset(4), 0.0, 0.0); // emerald
        assert_eq!(m.app.theme, "emerald");
        assert_eq!(m.app.accent, "#10b981");
        assert_eq!(m.app.opacity, 0.8);
        assert_eq!(m.app.glow, 0.45);
        assert_eq!(m.app.talk_outline, "#34d399");
        assert!(m.changed);
    }

    #[test]
    fn toggle_and_cycle() {
        let mut m = model();
        let before = m.app.show_header;
        m.click(Ctrl::Toggle("show_header"), 0.0, 0.0);
        assert_eq!(m.app.show_header, !before);
        m.click(Ctrl::Cycle("language"), 0.0, 0.0);
        assert_eq!(m.app.language, "ru");
        m.click(Ctrl::Cycle("language"), 0.0, 0.0);
        assert_eq!(m.app.language, "en");
        m.click(Ctrl::Cycle("language"), 0.0, 0.0);
        assert_eq!(m.app.language, "auto");
    }

    #[test]
    fn slider_drag_clamps() {
        let mut m = model();
        m.dragging = Some((
            "scale",
            crate::layout::Rect { x: 100.0, y: 0.0, w: 200.0, h: 6.0 },
        ));
        m.drag_to(160.0); // t=0.3 → 0.75 + 0.75*0.3
        assert!((m.app.scale - 0.975).abs() < 1e-5, "{}", m.app.scale);
        m.drag_to(9999.0);
        assert_eq!(m.app.scale, 1.5, "clamped to max");
        m.drag_to(-50.0);
        assert_eq!(m.app.scale, 0.75, "clamped to min");
    }

    #[test]
    fn scroll_clamps() {
        let mut m = model();
        m.wheel(-200.0, 2000.0); // wheel down → scroll grows
        assert!(m.scroll > 0.0, "{}", m.scroll);
        m.wheel(5000.0, 2000.0); // wheel up → clamped at 0
        assert_eq!(m.scroll, 0.0);
    }

    #[test]
    fn desktop_toggles_persist_defaults() {
        let mut m = model();
        // Run-as-admin default ON (требование: по умолчанию включено)
        assert!(m.desktop.elevate);
        m.click(Ctrl::Toggle("elevate"), 0.0, 0.0);
        assert!(!m.desktop.elevate);
        assert!(m.changed);
        m.click(Ctrl::Toggle("elevate"), 0.0, 0.0);
        assert!(m.desktop.elevate);
        m.click(Ctrl::Toggle("click_through"), 0.0, 0.0);
        assert!(!m.desktop.click_through);
    }

    #[test]
    fn frame_renders_with_regions() {
        let m = model();
        let fonts = Fonts::load();
        let icons = Icons::load();
        let (buf, regions, _) = render(&m, &fonts, &icons);
        assert_eq!((buf.w, buf.h), (WIN_W as i32, WIN_H as i32));
        assert!(regions.iter().any(|r| r.ctrl == Ctrl::Close));
        assert!(regions.iter().any(|r| r.ctrl == Ctrl::MoveOverlay));
        assert!(regions.iter().filter(|r| matches!(r.ctrl, Ctrl::Preset(_))).count() == 8);
        assert!(regions.iter().filter(|r| matches!(r.ctrl, Ctrl::Slider(_))).count() == 10);
    }
}

#[cfg(test)]
mod qa {
    use super::*;
    use crate::render::write_png;

    #[test]
    fn qa_dump_settings() {
        let mut m = SettingsModel::new(Appearance::default(), "0.1.0".into());
        let fonts = Fonts::load();
        let icons = Icons::load();
        let (buf, _, ch) = render(&m, &fonts, &icons);
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/qa");
        std::fs::create_dir_all(dir).unwrap();
        write_png(&format!("{dir}/settings.png"), &buf).unwrap();
        // scrolled to the bottom — «Система» section
        m.wheel(-100000.0, ch);
        let (buf2, _, _) = render(&m, &fonts, &icons);
        write_png(&format!("{dir}/settings-bottom.png"), &buf2).unwrap();
    }
}
