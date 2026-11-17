//! Native settings window — Концепт 3 «Превью-стек» (v1: «Вид» + preview).
//! Pure cross-platform model/render/input; the window host lives in overlay.rs.

use crate::assets::Icons;
use crate::config::Appearance;
use crate::font::Fonts;
use crate::layout::{render_frame, FeedItem, FrameInput};
use crate::model::UserState;
use crate::render::Buf;
use crate::theme::{parse_color, Palette, Rgba};

pub const WIN_W: f32 = 900.0;
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
        let lang = if app.language == "en" { "en".to_string() } else { "ru".to_string() };
        SettingsModel {
            app,
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
            Ctrl::Toggle(key) => {
                let v = toggle_mut(&mut self.app, key);
                *v = !*v;
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
        let max_scroll = (content_h - (WIN_H - 130.0)).max(0.0);
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

fn round_btn(c: &Ctx, buf: &mut Buf, x: f32, y: f32, w: f32, h: f32, label: &str, active: bool, danger: bool) {
    let bg = if danger {
        Rgba::rgb(220, 38, 38).scale_a(200.0)
    } else if active {
        c.pal.accent.scale_a(60.0)
    } else {
        c.pal.row_hover
    };
    buf.round_rect(x, y, w, h, 8.0, bg);
    if active {
        buf.stroke_round_rect(x, y, w, h, 8.0, c.pal.accent, 1.4);
    }
    let tw = c.fonts.measure(label, c.fs, true);
    c.fonts.draw(buf, x + (w - tw) * 0.5, y + h * 0.5 + c.fs * 0.35, label, c.fs, true, c.pal.text);
}

fn draw_toggle(c: &Ctx, buf: &mut Buf, x: f32, y: f32, w: f32, label: &str, on: bool) {
    c.fonts.draw(buf, x, y + 15.0, label, c.fs, false, c.pal.text);
    let (tw, th) = (40.0, 22.0);
    let tx = x + w - tw;
    let ty = y + 2.0;
    let bg = if on { c.pal.accent.scale_a(200.0) } else { c.pal.row_hover };
    buf.round_rect(tx, ty, tw, th, th * 0.5, bg);
    let kx = if on { tx + tw - th * 0.5 } else { tx + th * 0.5 };
    buf.circle(kx, ty + th * 0.5, th * 0.38, Rgba::rgb(255, 255, 255));
}

fn draw_slider(c: &Ctx, buf: &mut Buf, x: f32, y: f32, w: f32, label: &str, value: f64, range: (f64, f64), unit: &str) -> crate::layout::Rect {
    let text = if unit == "px" {
        format!("{label}: {value:.0}")
    } else {
        format!("{label}: {value:.2}")
    };
    c.fonts.draw(buf, x, y + 12.0, &text, c.fs, false, c.pal.text);
    let track = crate::layout::Rect { x, y: y + 22.0, w, h: 6.0 };
    buf.round_rect(track.x, track.y, track.w, track.h, 3.0, c.pal.row_hover);
    let t = ((value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0) as f32;
    buf.round_rect(track.x, track.y, track.w * t, track.h, 3.0, c.pal.accent);
    buf.circle(track.x + track.w * t, track.y + 3.0, 7.0, Rgba::rgb(255, 255, 255));
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
        work_h: WIN_H,
    }
}

/// Render the settings window; returns the buffer + interactive regions + content height.
pub fn render(m: &SettingsModel, fonts: &Fonts, icons: &Icons) -> (Buf, Vec<Region>, f32) {
    let pal = Palette::from_appearance(&m.app);
    let c = Ctx { pal: &pal, fonts, fs: 13.0 };
    let mut buf = Buf::new(WIN_W as i32, WIN_H as i32);
    let mut regions: Vec<Region> = Vec::new();

    // window background (theme panel, opaque)
    let mut bg = pal.panel_bg;
    bg.0[3] = 255;
    buf.round_rect(0.0, 0.0, WIN_W, WIN_H, 12.0, bg);
    buf.stroke_round_rect(0.0, 0.0, WIN_W, WIN_H, 12.0, pal.panel_border, 1.0);

    // ---- title bar ----
    let title = tr(&m.lang, "Настройки", "Settings");
    c.fonts.draw(&mut buf, 20.0, 28.0, &format!("TS3-TS6 Notifier — {title}"), 15.0, true, pal.text);
    let ver = format!("v{}", m.version);
    let vw = c.fonts.measure(&ver, 12.0, false);
    c.fonts.draw(&mut buf, WIN_W - 120.0 - vw, 27.0, &ver, 12.0, false, pal.muted);
    // close ✕
    let (cx, cy, cs) = (WIN_W - 44.0, 10.0, 30.0);
    buf.round_rect(cx, cy, cs, cs, 8.0, pal.row_hover);
    buf.line(cx + 10.0, cy + 10.0, cx + 20.0, cy + 20.0, 2.2, pal.text);
    buf.line(cx + 20.0, cy + 10.0, cx + 10.0, cy + 20.0, 2.2, pal.text);
    regions.push(Region {
        rect: crate::layout::Rect { x: cx, y: cy, w: cs, h: cs },
        ctrl: Ctrl::Close,
    });
    buf.fill_rect(0.0, 44.0, WIN_W, 1.0, pal.panel_border);

    // ---- left pane (scrollable) ----
    let pane_x = 16.0;
    let pane_w = 452.0;
    let pane_y = 58.0;
    let pane_h = WIN_H - 58.0 - 70.0;
    buf.set_clip(pane_x, pane_y, pane_w, pane_h);
    let mut y = pane_y - m.scroll;

    let section = |buf: &mut Buf, c: &Ctx, y: f32, text: &str| {
        c.fonts.draw(buf, pane_x + 4.0, y + 16.0, text, 14.0, true, c.pal.accent);
        buf.fill_rect(pane_x + 4.0, y + 24.0, pane_w - 8.0, 1.0, c.pal.panel_border);
    };

    section(&mut buf, &c, y, &tr(&m.lang, "Пресеты", "Presets"));
    y += 34.0;
    for (i, p) in PRESETS.iter().enumerate() {
        let col = (i % 2) as f32;
        let row = (i / 2) as f32;
        let bx = pane_x + 4.0 + col * (pane_w * 0.5 - 6.0);
        let by = y + row * 40.0;
        let bw = pane_w * 0.5 - 14.0;
        let active = m.app.theme == p.0;
        let label = tr(&m.lang, p.1, p.2);
        round_btn(&c, &mut buf, bx, by, bw, 34.0, &label, active, false);
        regions.push(Region {
            rect: crate::layout::Rect { x: bx, y: by, w: bw, h: 34.0 },
            ctrl: Ctrl::Preset(i as u8),
        });
    }
    y += 164.0;

    section(&mut buf, &c, y, &tr(&m.lang, "Цвета", "Colors"));
    y += 34.0;
    for (i, sw) in ACCENT_SWATCHES.iter().enumerate() {
        let sx = pane_x + 4.0 + i as f32 * 34.0;
        let active = m.app.accent.eq_ignore_ascii_case(sw);
        buf.round_rect(sx, y, 26.0, 26.0, 7.0, parse_color(sw));
        if active {
            buf.stroke_round_rect(sx - 1.0, y - 1.0, 28.0, 28.0, 8.0, pal.text, 2.0);
        }
        regions.push(Region {
            rect: crate::layout::Rect { x: sx, y, w: 26.0, h: 26.0 },
            ctrl: Ctrl::Swatch(i as u8),
        });
    }
    y += 42.0;

    section(&mut buf, &c, y, &tr(&m.lang, "Геометрия и стиль", "Geometry & style"));
    y += 34.0;
    let sliders: [(&str, &str, &str); 8] = [
        ("scale", "Масштаб", "Scale"),
        ("font_size", "Шрифт", "Font size"),
        ("opacity", "Прозрачность", "Opacity"),
        ("glow", "Свечение", "Glow"),
        ("blur", "Размытие", "Blur"),
        ("radius", "Скругление", "Corner radius"),
        ("margin_x", "Отступ X", "Margin X"),
        ("margin_y", "Отступ Y", "Margin Y"),
    ];
    for (key, ru, en) in sliders {
        let label = tr(&m.lang, ru, en);
        let unit = match key {
            "font_size" | "blur" | "radius" | "margin_x" | "margin_y" => "px",
            _ => "",
        };
        let track = draw_slider(
            &c,
            &mut buf,
            pane_x + 4.0,
            y,
            pane_w - 8.0,
            &label,
            get_slider(&m.app, key),
            slider_range(key),
            unit,
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
    let content_h = y + m.scroll - pane_y;
    buf.clear_clip();
    // scroll indicator
    if content_h > pane_h {
        let frac = pane_h / content_h;
        let bar_h = (pane_h * frac).max(24.0);
        let bar_y = pane_y + (m.scroll / content_h) * (pane_h - bar_h);
        buf.round_rect(pane_x + pane_w - 4.0, bar_y, 3.0, bar_h, 1.5, pal.muted.scale_a(120.0));
    }

    // ---- right pane: pinned live preview ----
    let pv_x = pane_x + pane_w + 16.0;
    let pv_w = WIN_W - pv_x - 16.0;
    buf.round_rect(pv_x, pane_y, pv_w, pane_h, 10.0, Rgba::rgb(0, 0, 0).scale_a(60.0));
    buf.stroke_round_rect(pv_x, pane_y, pv_w, pane_h, 10.0, pal.panel_border, 1.0);
    let cap = tr(&m.lang, "Предпросмотр", "Preview");
    c.fonts.draw(&mut buf, pv_x + 12.0, pane_y + 22.0, &cap, 12.0, true, pal.muted);

    let sample = sample_input(&m.app, &pal, fonts, icons);
    let frame = render_frame(&sample);
    let avail_w = pv_w - 24.0;
    let avail_h = pane_h - 44.0;
    let fit = (avail_w / frame.panel_w).min(avail_h / frame.panel_h).min(1.0);
    let dw = frame.panel_w * fit;
    let dh = frame.panel_h * fit;
    buf.set_clip(pv_x, pane_y + 30.0, pv_w, pane_h - 30.0);
    buf.blit_rgba(
        pv_x + (pv_w - dw) * 0.5,
        pane_y + 34.0,
        dw,
        dh,
        &frame.buf.px,
        frame.buf.w,
        frame.buf.h,
        false,
    );
    buf.clear_clip();

    // ---- bottom bar ----
    let by = WIN_H - 58.0;
    buf.fill_rect(0.0, by - 6.0, WIN_W, 1.0, pal.panel_border);
    let label = tr(&m.lang, "Переместить оверлей", "Move overlay");
    round_btn(&c, &mut buf, 16.0, by, 210.0, 36.0, &label, false, false);
    regions.push(Region {
        rect: crate::layout::Rect { x: 16.0, y: by, w: 210.0, h: 36.0 },
        ctrl: Ctrl::MoveOverlay,
    });
    let hint = tr(
        &m.lang,
        "Изменения применяются сразу",
        "Changes apply immediately",
    );
    c.fonts.draw(&mut buf, WIN_W - 16.0 - c.fonts.measure(&hint, 12.0, false), by + 23.0, &hint, 12.0, false, pal.muted);

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
    #[cfg(not(windows))]
    let _ = &app;
    std::thread::Builder::new()
        .name("settings".into())
        .spawn(move || {
            #[cfg(windows)]
            crate::overlay::win32::run_settings(app.clone());
            SETTINGS_OPEN.store(false, Ordering::SeqCst);
        })
        .ok();
}

fn persist(app: &Arc<App>, a: &Appearance) {
    let patch = crate::json::obj(vec![("appearance", crate::config::appearance_to_json(a))]);
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
            persist(app, &m.app);
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
    fn frame_renders_with_regions() {
        let m = model();
        let fonts = Fonts::load();
        let icons = Icons::load();
        let (buf, regions, _) = render(&m, &fonts, &icons);
        assert_eq!((buf.w, buf.h), (WIN_W as i32, WIN_H as i32));
        assert!(regions.iter().any(|r| r.ctrl == Ctrl::Close));
        assert!(regions.iter().any(|r| r.ctrl == Ctrl::MoveOverlay));
        assert!(regions.iter().filter(|r| matches!(r.ctrl, Ctrl::Preset(_))).count() == 8);
        assert!(regions.iter().filter(|r| matches!(r.ctrl, Ctrl::Slider(_))).count() == 8);
    }
}

#[cfg(test)]
mod qa {
    use super::*;
    use crate::render::write_png;

    #[test]
    fn qa_dump_settings() {
        let m = SettingsModel::new(Appearance::default(), "0.1.0".into());
        let fonts = Fonts::load();
        let icons = Icons::load();
        let (buf, _, _) = render(&m, &fonts, &icons);
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/qa");
        std::fs::create_dir_all(dir).unwrap();
        write_png(&format!("{dir}/settings.png"), &buf).unwrap();
    }
}
