//! Native settings window — Концепт 3 «Превью-стек» (v1: «Вид» + preview).
//! Pure cross-platform model/render/input; the window host lives in overlay.rs.

use crate::assets::Icons;
use crate::config::{Appearance, DesktopPrefs};
use crate::font::Fonts;
use crate::layout::{render_frame, FeedItem, FrameInput};
use crate::model::UserState;
use crate::render::Buf;
use crate::theme::{parse_color, Palette, Rgba, StyleKind};

pub const WIN_W: f32 = 1020.0;
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
    Corner(u8),
    /// Window title bar drag area (moves the settings window).
    DragWindow,
    Toggle(&'static str),
    Cycle(&'static str),
    Slider(&'static str),
    /// Closed dropdown header — click toggles the option list.
    Select(&'static str),
    /// An open dropdown option: (key, option index).
    Option(&'static str, u8),
    /// Text input field: (field id). Click focuses it for typing.
    Field(&'static str),
    /// «Установить плагин» (TeamSpeak 3).
    InstallPlugin,
    /// «Переподключить» — restart the active connector.
    Reconnect,
    /// «Проверить обновления».
    CheckUpdate,
    /// «Перезапустить сейчас» — install a downloaded update and restart.
    InstallUpdate,
    /// Left-nav page switch: (page index).
    Nav(u8),
    /// Expand/collapse «Группы сервера».
    ToggleGroups,
    /// Copy the OBS browser-source URL to the clipboard.
    CopyObs,
    /// Copy the diagnostics report to the clipboard.
    CopyDiag,
}

/// Static select keys for the «Группы сервера» rows (explicit class pickers).
pub const GROUP_KEYS: [&str; 17] = [
    "grp0", "grp1", "grp2", "grp3", "grp4", "grp5", "grp6", "grp7", "grp8", "grp9", "grp10",
    "grp11", "grp12", "grp13", "grp14", "grp15", "grp16",
];

pub const GROUP_ROWS: [&str; 17] = [
    "442", "444", "445", "446", "447", "448", "449", "450", "451", "452", "453", "454", "455", "456",
    "502", "503", "506",
];

/// Class cycle order («нет» first). RU labels via class_ru().
pub const CLASS_CYCLE: [&str; 19] = [
    "", "kanglong", "blood", "werewolf", "warrior", "assassin", "gunslinger", "guardian", "priest",
    "archer", "paladin", "shaman", "bard", "mystic", "druid", "monkey", "mage", "reaper", "ghost",
];

fn class_ru(key: &str) -> &'static str {
    match key {
        "kanglong" => "Канглонг",
        "blood" => "Кровавый",
        "werewolf" => "Оборотень",
        "warrior" => "Воин",
        "assassin" => "Ассасин",
        "gunslinger" => "Стрелок",
        "guardian" => "Страж",
        "priest" => "Жрец",
        "archer" => "Лучник",
        "paladin" => "Паладин",
        "shaman" => "Шаман",
        "bard" => "Бард",
        "mystic" => "Мистик",
        "druid" => "Друид",
        "monkey" => "Обезьяна",
        "mage" => "Маг",
        "reaper" => "Жнец",
        "ghost" => "Призрак",
        _ => "нет",
    }
}

/// Actions that need `Arc<App>` — executed by run_loop after click().
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pending {
    InstallPlugin,
    Reconnect,
    CheckUpdate,
    /// Install a downloaded update and restart («Перезапустить сейчас»).
    InstallUpdate,
    CopyObs,
    CopyDiag,
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
    /// Which dropdown (if any) is currently open.
    pub open_select: Option<&'static str>,
    /// Connection draft («Связь») — persisted on every change.
    pub source: String,
    pub ts3: crate::config::TsEndpoint,
    pub ts6: crate::config::TsEndpoint,
    /// «Группы сервера»: (group id, class key or "").
    pub groups: Vec<(String, String)>,
    /// Focused text field id (typing goes there).
    pub focus: Option<&'static str>,
    /// First typed char replaces the whole value (classic select-all focus).
    pub select_all: bool,
    /// Transient status line under the «Связь» controls.
    pub conn_note: String,
    /// Status line under «Проверить обновления».
    pub update_note: String,
    /// A downloaded update is ready to install («Перезапустить сейчас»).
    pub update_ready: bool,
    /// Left-nav page: 0 Вид, 1 Элементы, 2 Связь, 3 Обновления, 4 Ещё.
    pub page: u8,
    /// «Группы сервера» expanded (hidden by default).
    pub groups_open: bool,
    /// Actions that need `Arc<App>` — performed by run_loop.
    pub pending: Option<Pending>,
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
            open_select: None,
            source: "mock".into(),
            ts3: crate::config::TsEndpoint {
                host: "127.0.0.1".into(),
                port: 25639,
                api_key: String::new(),
            },
            ts6: crate::config::TsEndpoint {
                host: "127.0.0.1".into(),
                port: 5899,
                api_key: String::new(),
            },
            groups: GROUP_ROWS.iter().map(|id| (id.to_string(), String::new())).collect(),
            focus: None,
            select_all: false,
            conn_note: String::new(),
            update_note: String::new(),
            update_ready: false,
            page: 0,
            groups_open: false,
            pending: None,
        }
    }

    /// Type into the focused field: printable chars, `\x08` backspace, pasted
    /// strings. The first char after focusing replaces the value.
    pub fn text_input(&mut self, text: &str) {
        let Some(id) = self.focus else { return };
        let mut dirty = false;
        for ch in text.chars() {
            if ch == '\x08' {
                if id.ends_with("_port") {
                    let mut v = self.field_text(id);
                    if self.select_all {
                        v.clear();
                    } else {
                        v.pop();
                    }
                    self.set_field_text(id, &v);
                } else {
                    let sel = self.select_all;
                    if let Some(f) = self.field_mut(id) {
                        if sel {
                            f.clear();
                        } else {
                            f.pop();
                        }
                    }
                }
                self.select_all = false;
                dirty = true;
            } else if ch == '\r' || ch == '\n' {
                // multi-line input is meaningless here
            } else if id.ends_with("_port") {
                if !ch.is_ascii_digit() {
                    continue;
                }
                let mut v = self.field_text(id);
                if self.select_all {
                    v.clear();
                }
                if v.len() < 5 {
                    v.push(ch);
                }
                self.set_field_text(id, &v);
                self.select_all = false;
                dirty = true;
            } else if (ch as u32) >= 0x20 {
                let sel = self.select_all;
                if let Some(f) = self.field_mut(id) {
                    if sel {
                        f.clear();
                    }
                    f.push(ch);
                }
                self.select_all = false;
                dirty = true;
            }
        }
        if dirty {
            self.touch();
        }
    }

    fn field_mut(&mut self, id: &str) -> Option<&mut String> {
        match id {
            "ts3_host" => Some(&mut self.ts3.host),
            "ts3_key" => Some(&mut self.ts3.api_key),
            "ts6_host" => Some(&mut self.ts6.host),
            "ts6_key" => Some(&mut self.ts6.api_key),
            _ => None,
        }
    }

    fn field_text(&self, id: &str) -> String {
        match id {
            "ts3_host" => self.ts3.host.clone(),
            "ts3_key" => self.ts3.api_key.clone(),
            "ts6_host" => self.ts6.host.clone(),
            "ts6_key" => self.ts6.api_key.clone(),
            "ts3_port" => self.ts3.port.to_string(),
            "ts6_port" => self.ts6.port.to_string(),
            _ => String::new(),
        }
    }

    fn set_field_text(&mut self, id: &str, v: &str) {
        match id {
            "ts3_port" => self.ts3.port = v.parse().unwrap_or(0).clamp(0, 65535),
            "ts6_port" => self.ts6.port = v.parse().unwrap_or(0).clamp(0, 65535),
            _ => {}
        }
    }

    fn touch(&mut self) {
        self.changed = true;
    }

    /// Close the open dropdown (outside click / other control).
    pub fn close_select(&mut self) {
        self.open_select = None;
    }

    pub fn click(&mut self, ctrl: Ctrl, _x: f32, _y: f32) {
        if self.open_select.is_some() && !matches!(ctrl, Ctrl::Option(..) | Ctrl::Select(_)) {
            self.open_select = None;
        }
        if !matches!(ctrl, Ctrl::Field(_)) {
            self.focus = None;
            self.select_all = false;
        }
        match ctrl {
            Ctrl::Close => self.close_requested = true,
            Ctrl::Field(id) => {
                self.focus = Some(id);
                self.select_all = true;
            }
            Ctrl::InstallPlugin => self.pending = Some(Pending::InstallPlugin),
            Ctrl::Reconnect => self.pending = Some(Pending::Reconnect),
            Ctrl::CheckUpdate => self.pending = Some(Pending::CheckUpdate),
            Ctrl::InstallUpdate => self.pending = Some(Pending::InstallUpdate),
            Ctrl::Nav(n) => {
                if self.page != n {
                    self.page = n;
                    self.scroll = 0.0;
                    self.close_select();
                }
            }
            Ctrl::ToggleGroups => self.groups_open = !self.groups_open,
            Ctrl::CopyObs => self.pending = Some(Pending::CopyObs),
            Ctrl::CopyDiag => self.pending = Some(Pending::CopyDiag),
            Ctrl::Select(key) => {
                self.open_select = if self.open_select == Some(key) { None } else { Some(key) };
            }
            Ctrl::Option(key, i) => {
                if let Some(rest) = key.strip_prefix("grp") {
                    if let Ok(r) = rest.parse::<usize>() {
                        if let Some(row) = self.groups.get_mut(r) {
                            row.1 = CLASS_CYCLE.get(i as usize).copied().unwrap_or("").to_string();
                        }
                    }
                    self.open_select = None;
                    self.touch();
                    return;
                }
                let v = match key {
                    "source" => ["mock", "ts3", "ts6"].get(i as usize).copied().unwrap_or("mock"),
                    "language" => ["auto", "ru", "en"].get(i as usize).copied().unwrap_or("auto"),
                    "eq_style" => ["bars", "ring", "dots", "none"].get(i as usize).copied().unwrap_or("bars"),
                    "overlay_layer" => ["panel", "screen", "fullscreen"].get(i as usize).copied().unwrap_or("screen"),
                    _ => ["all", "own"].get(i as usize).copied().unwrap_or("all"),
                };
                match key {
                    "source" => self.source = v.into(),
                    "language" => self.app.language = v.into(),
                    "eq_style" => self.app.eq_style = v.into(),
                    "overlay_layer" => self.desktop.overlay_layer = v.into(),
                    "ev_scope_join" => self.app.ev_scope_join = v.into(),
                    "ev_scope_leave" => self.app.ev_scope_leave = v.into(),
                    "ev_scope_move" => self.app.ev_scope_move = v.into(),
                    "ev_scope_group" => self.app.ev_scope_group = v.into(),
                    "ev_scope_sys" => self.app.ev_scope_sys = v.into(),
                    "ev_scope_chat" => self.app.ev_scope_chat = v.into(),
                    _ => {}
                }
                self.open_select = None;
                self.touch();
            }
            Ctrl::Corner(i) => {
                // «Положение» = one-click snap: the overlay knows the live panel
                // size, snaps to the corner and writes real coordinates back
                self.app.corner = ["tl", "tr", "bl", "br"].get(i as usize).copied().unwrap_or("tr").into();
                self.touch();
            }
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
                "autostart" => {
                    self.desktop.autostart = !self.desktop.autostart;
                    self.touch();
                }
                "visible" => {
                    self.desktop.visible = !self.desktop.visible;
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
            Ctrl::DragWindow => {}
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
        "chat_private" => &mut app.chat_private,
        "show_unread_badge" => &mut app.show_unread_badge,
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

/// Margin sliders travel the whole screen («координаты по всему экрану»):
/// the run loop feeds the real work-area size here; tests fall back to 4K.
/// Placement clamps (clamp_margins) keep the overlay inside the screen.
static SCREEN_DIMS: std::sync::Mutex<(f64, f64)> = std::sync::Mutex::new((3840.0, 2160.0));

pub fn set_screen_dims(w: f64, h: f64) {
    if w >= 320.0 && h >= 240.0 {
        *SCREEN_DIMS.lock().unwrap() = (w, h);
    }
}

fn slider_range(key: &str) -> (f64, f64) {
    if key == "margin_x" {
        return (0.0, SCREEN_DIMS.lock().unwrap().0);
    }
    if key == "margin_y" {
        return (0.0, SCREEN_DIMS.lock().unwrap().1);
    }
    match key {
        "scale" => (0.75, 1.5),
        "font_size" => (11.0, 18.0),
        "panel_width" => (200.0, 600.0),
        "panel_max_height" => (0.0, 1000.0),
        "opacity" => (0.15, 1.0),
        "glow" => (0.0, 1.0),
        "blur" => (0.0, 40.0),
        "radius" => (0.0, 26.0),
        "whisper_delay" => (1.0, 30.0),
        "talking_fade" => (0.0, 10.0),
        "chat_delay" => (2.0, 60.0),
        "events_delay" => (5.0, 60.0),
        "events_count" => (1.0, 10.0),
        _ => (0.0, 200.0),
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
        "margin_y" => app.margin_y = v,
        "whisper_delay" => app.whisper_delay = v.round().clamp(min, max),
        "talking_fade" => app.talking_fade = v.round().clamp(min, max),
        "chat_delay" => app.chat_delay = v.round().clamp(min, max),
        "events_delay" => app.events_delay = v.round().clamp(min, max),
        "events_count" => app.events_count = v.round().clamp(min, max),
        _ => {}
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
        "margin_y" => app.margin_y,
        "whisper_delay" => app.whisper_delay,
        "talking_fade" => app.talking_fade,
        "chat_delay" => app.chat_delay,
        "events_delay" => app.events_delay,
        "events_count" => app.events_count as f64,
        _ => 0.0,
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
        "show_ev_chat" => app.show_ev_chat,
        "chat_private" => app.chat_private,
        _ => app.show_unread_badge,
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
/// Draw wrapper that scales every coordinate/size by `k` (DPI): the buffer is
/// physical pixels while all layout math stays in logical units.
pub struct SBuf {
    b: crate::render::Buf,
    k: f32,
}

impl SBuf {
    fn s(&self, v: f32) -> f32 {
        v * self.k
    }
    pub fn text(&mut self, fonts: &crate::font::Fonts, x: f32, y: f32, t: &str, size: f32, bold: bool, col: Rgba) {
        let (x, y, size) = (self.s(x), self.s(y), self.s(size));
        fonts.draw(&mut self.b, x, y, t, size, bold, col);
    }
    pub fn clear(&mut self) {
        self.b.clear();
    }
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
        self.b.fill_rect(self.s(x), self.s(y), self.s(w), self.s(h), c);
    }
    pub fn fill_rect_vgrad(&mut self, x: f32, y: f32, w: f32, h: f32, top: Rgba, bottom: Rgba) {
        self.b.fill_rect_vgrad(self.s(x), self.s(y), self.s(w), self.s(h), top, bottom);
    }
    pub fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c: Rgba) {
        self.b.round_rect(self.s(x), self.s(y), self.s(w), self.s(h), self.s(radius), c);
    }
    pub fn stroke_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c: Rgba, thickness: f32) {
        self.b.stroke_round_rect(self.s(x), self.s(y), self.s(w), self.s(h), self.s(radius), c, self.s(thickness));
    }
    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        self.b.circle(self.s(cx), self.s(cy), self.s(r), c);
    }
    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba, thickness: f32) {
        self.b.stroke_circle(self.s(cx), self.s(cy), self.s(r), c, self.s(thickness));
    }
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, thickness: f32, c: Rgba) {
        self.b.line(self.s(x0), self.s(y0), self.s(x1), self.s(y1), self.s(thickness), c);
    }
    pub fn set_clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.b.set_clip(self.s(x), self.s(y), self.s(w), self.s(h));
    }
    pub fn clear_clip(&mut self) {
        self.b.clear_clip();
    }
    pub fn blit_rgba(&mut self, dx: f32, dy: f32, dw: f32, dh: f32, src: &[u8], sw: i32, sh: i32, circle_clip: bool) {
        self.b.blit_rgba(self.s(dx), self.s(dy), self.s(dw), self.s(dh), src, sw, sh, circle_clip);
    }
}

/// Physical/logical ratio of the current display (1.0 on non-windows).
fn ui_scale() -> f32 {
    #[cfg(any(windows, tsn_win32))]
    {
        crate::overlay::win32::dpi_scale() as f32
    }
    #[cfg(not(any(windows, tsn_win32)))]
    {
        1.0
    }
}

fn draw_icon(b: &mut SBuf, cx: f32, cy: f32, s: f32, kind: u8, col: Rgba, bg: Rgba) {
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
fn draw_tile(c: &Ctx, buf: &mut SBuf, x: f32, y: f32, w: f32, h: f32, label: &str, icon: u8, active: bool) {
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
    buf.text(&c.fonts, x + (w - tw) * 0.5, y + h - 11.0, label, 12.5, true, c.pal.text);
}

/// Accent pill button («Переместить оверлей»).
fn draw_field(
    c: &Ctx,
    buf: &mut SBuf,
    x: f32,
    y: f32,
    w: f32,
    label: &str,
    value: &str,
    focused: bool,
    select_all: bool,
) -> crate::layout::Rect {
    buf.text(&c.fonts, x, y + 12.0, label, 11.5, false, c.pal.muted);
    let by = y + 18.0;
    let bh = 28.0;
    buf.round_rect(
        x,
        by,
        w,
        bh,
        7.0,
        crate::theme::mix(c.pal.panel_bg, Rgba::rgb(255, 255, 255), 0.05),
    );
    if focused {
        buf.stroke_round_rect(x, by, w, bh, 7.0, c.pal.accent, 1.4);
        buf.stroke_round_rect(x - 2.0, by - 2.0, w + 4.0, bh + 4.0, 9.0, c.pal.accent.scale_a(50.0), 1.1);
    } else {
        buf.stroke_round_rect(x, by, w, bh, 7.0, c.pal.muted.scale_a(110.0), 1.0);
    }
    let ty = by + bh * 0.62;
    if focused && select_all {
        let tw = c.fonts.measure(value, 12.5, false);
        buf.fill_rect(x + 7.0, by + 6.0, tw + 3.0, bh - 12.0, c.pal.accent.scale_a(120.0));
    }
    buf.text(&c.fonts, x + 8.0, ty, value, 12.5, false, c.pal.text);
    if focused {
        let tw = c.fonts.measure(value, 12.5, false);
        buf.fill_rect(x + 10.0 + tw, by + 7.0, 1.6, bh - 14.0, c.pal.text);
    }
    crate::layout::Rect { x, y: by, w, h: bh }
}

fn draw_button(c: &Ctx, buf: &mut SBuf, x: f32, y: f32, w: f32, h: f32, label: &str) {
    buf.round_rect(x, y, w, h, 12.0, c.pal.accent.scale_a(235.0));
    // soft glow halo (vid-3)
    buf.stroke_round_rect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, 13.0, c.pal.accent.scale_a(85.0), 1.2);
    buf.stroke_round_rect(x - 3.0, y - 3.0, w + 6.0, h + 6.0, 15.0, c.pal.accent.scale_a(40.0), 1.1);
    let tw = c.fonts.measure(label, 13.5, true);
    buf.text(&c.fonts, x + (w - tw) * 0.5, y + h * 0.5 + 5.0, label, 13.5, true, Rgba::rgb(255, 255, 255));
}

/// Small pin glyph on the preview card.
fn draw_pin(c: &Ctx, buf: &mut SBuf, x: f32, y: f32) {
    buf.circle(x, y, 3.2, c.pal.muted);
    buf.line(x, y + 3.0, x + 5.0, y + 8.0, 1.6, c.pal.muted);
}

fn draw_toggle(c: &Ctx, buf: &mut SBuf, x: f32, y: f32, w: f32, label: &str, on: bool) {
    // «убери сами ползунки» — a plain highlightable BUTTON, a bit bigger than
    // the action buttons: lit when ON, dim but clearly readable when OFF
    let h = 36.0;
    let r = 12.0;
    if on {
        buf.round_rect(x, y, w, h, r, c.pal.accent.scale_a(235.0));
        buf.stroke_round_rect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, r + 1.0, c.pal.accent.scale_a(90.0), 1.2);
        buf.stroke_round_rect(x - 3.5, y - 3.5, w + 7.0, h + 7.0, r + 3.5, c.pal.accent.scale_a(42.0), 1.1);
        let tw = c.fonts.measure(label, 13.5, true);
        buf.text(&c.fonts, x + (w - tw) * 0.5, y + 23.0, label, 13.5, true, Rgba::rgb(255, 255, 255));
    } else {
        buf.round_rect(x, y, w, h, r, crate::theme::mix(c.pal.panel_bg, Rgba::rgb(0, 0, 0), 0.18));
        buf.stroke_round_rect(x, y, w, h, r, c.pal.muted.scale_a(175.0), 1.2);
        let tw = c.fonts.measure(label, 13.5, false);
        buf.text(&c.fonts, x + (w - tw) * 0.5, y + 23.0, label, 13.5, false, c.pal.muted);
    }
}

/// Slider row: label left, value right (accent), track + in-bounds knob below.
fn draw_slider(c: &Ctx, buf: &mut SBuf, x: f32, y: f32, w: f32, label: &str, value_str: &str, value: f64, range: (f64, f64)) -> crate::layout::Rect {
    buf.text(&c.fonts, x, y + 12.0, label, c.fs, false, c.pal.text);
    let vw = c.fonts.measure(value_str, c.fs, true);
    buf.text(&c.fonts, x + w - vw, y + 12.0, value_str, c.fs, true, c.pal.accent);
    let track = crate::layout::Rect { x, y: y + 24.0, w, h: 8.0 };
    buf.round_rect(
        track.x,
        track.y,
        track.w,
        track.h,
        4.0,
        crate::theme::mix(c.pal.panel_bg, Rgba::rgb(0, 0, 0), 0.22),
    );
    buf.stroke_round_rect(track.x, track.y, track.w, track.h, 4.0, c.pal.muted.scale_a(90.0), 1.0);
    let t = ((value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0) as f32;
    let kx = track.x + 8.0 + (track.w - 16.0) * t;
    buf.round_rect(track.x, track.y, (kx - track.x).max(4.0), track.h, 4.0, c.pal.accent);
    buf.circle(kx, track.y + 4.0, 11.0, c.pal.accent.scale_a(45.0));
    buf.circle(kx, track.y + 4.0, 8.0, Rgba::rgb(255, 255, 255));
    buf.stroke_circle(kx, track.y + 4.0, 8.0, c.pal.accent.scale_a(150.0), 1.2);
    track
}

fn select_options(key: &str, lang: &str) -> Vec<(String, u8)> {
    if let Some(rest) = key.strip_prefix("grp") {
        if rest.chars().all(|c| c.is_ascii_digit()) {
            let mut v = vec![(tr(lang, "— не сопоставлено", "— unmapped"), 0u8)];
            for (i, c) in CLASS_CYCLE.iter().enumerate().skip(1) {
                v.push((class_ru(c).to_string(), i as u8));
            }
            return v;
        }
    }
    if key == "source" {
        return vec![
            (tr(lang, "Демо-данные", "Mock data"), 0),
            ("TeamSpeak 3".into(), 1),
            ("TeamSpeak 6".into(), 2),
        ];
    }
    match key {
        "language" => vec![
            (tr(lang, "Авто", "Auto"), 0),
            (tr(lang, "Русский", "Russian"), 1),
            ("English".to_string(), 2),
        ],
        "eq_style" => vec![
            (tr(lang, "Столбики", "Bars"), 0),
            (tr(lang, "Кольцо", "Ring"), 1),
            (tr(lang, "Точки", "Dots"), 2),
            (tr(lang, "Скрыть", "Hidden"), 3),
        ],
        "overlay_layer" => vec![
            (tr(lang, "По размеру панели", "Panel-sized"), 0),
            (tr(lang, "Во весь экран", "Full screen"), 1),
            (tr(lang, "Полноэкранное окно", "Fullscreen window"), 2),
        ],
        _ => vec![
            (tr(lang, "Весь сервер", "Whole server"), 0),
            (tr(lang, "Мой канал", "My channel"), 1),
        ],
    }
}

fn select_index(key: &str, m: &SettingsModel) -> u8 {
    if let Some(rest) = key.strip_prefix("grp") {
        if let Ok(i) = rest.parse::<usize>() {
            if let Some((_, cls)) = m.groups.get(i) {
                return CLASS_CYCLE
                    .iter()
                    .position(|c| *c == cls.as_str())
                    .unwrap_or(0) as u8;
            }
        }
        return 0;
    }
    if key == "source" {
        return match m.source.as_str() {
            "ts3" => 1,
            "ts6" => 2,
            _ => 0,
        };
    }
    let (app, desktop) = (&m.app, &m.desktop);
    let v = match key {
        "language" => app.language.as_str(),
        "eq_style" => app.eq_style.as_str(),
        "overlay_layer" => desktop.overlay_layer.as_str(),
        "ev_scope_join" => app.ev_scope_join.as_str(),
        "ev_scope_leave" => app.ev_scope_leave.as_str(),
        "ev_scope_move" => app.ev_scope_move.as_str(),
        "ev_scope_group" => app.ev_scope_group.as_str(),
        "ev_scope_sys" => app.ev_scope_sys.as_str(),
        _ => app.ev_scope_chat.as_str(),
    };
    match (key, v) {
        ("language", "auto") => 0, ("language", "ru") => 1, ("language", "en") => 2,
        ("eq_style", "bars") => 0, ("eq_style", "ring") => 1, ("eq_style", "dots") => 2, ("eq_style", _) => 3,
        ("overlay_layer", "panel") => 0, ("overlay_layer", "screen") => 1, ("overlay_layer", _) => 2,
        (_, "own") => 1,
        _ => 0,
    }
}

/// Dropdown row: label left, value box + chevron right. Returns the value box rect.
fn draw_select(
    c: &Ctx,
    buf: &mut SBuf,
    x: f32,
    y: f32,
    w: f32,
    label: &str,
    value: &str,
    open: bool,
) -> crate::layout::Rect {
    let pal = &c.pal;
    let bx = x + w - 194.0;
    buf.round_rect(
        bx,
        y - 2.0,
        190.0,
        28.0,
        7.0,
        crate::theme::mix(pal.panel_bg, Rgba::rgb(255, 255, 255), 0.05),
    );
    if open {
        buf.stroke_round_rect(bx, y - 2.0, 190.0, 28.0, 7.0, pal.accent, 1.3);
        buf.stroke_round_rect(bx - 2.0, y - 4.0, 194.0, 32.0, 9.0, pal.accent.scale_a(50.0), 1.1);
    } else {
        buf.stroke_round_rect(bx, y - 2.0, 190.0, 28.0, 7.0, pal.muted.scale_a(110.0), 1.0);
    }
    buf.text(&c.fonts, bx + 12.0, y + 16.0, value, 12.0, false, pal.text);
    // chevron
    let cx = bx + 172.0;
    let cy = y + 11.0;
    if open {
        buf.line(cx - 4.0, cy + 3.0, cx, cy - 3.0, 1.6, pal.accent);
        buf.line(cx, cy - 3.0, cx + 4.0, cy + 3.0, 1.6, pal.accent);
    } else {
        buf.line(cx - 4.0, cy - 3.0, cx, cy + 3.0, 1.6, pal.muted);
        buf.line(cx, cy + 3.0, cx + 4.0, cy - 3.0, 1.6, pal.muted);
    }
    buf.text(&c.fonts, x + 2.0, y + 16.0, label, 12.0, false, pal.text);
    crate::layout::Rect { x: bx, y: y - 2.0, w: 190.0, h: 28.0 }
}

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
        talk_alphas: vec![],
        class_keys: vec![Some("archer".into()), Some("mage".into()), Some("warrior".into())],
        whisper_keys: vec![("103".into(), Some("ghost".into()))],
        whisperers: vec![ghost],
        feed: vec![FeedItem { nickname: "Мыsha".into(), text: "привет!".into(), kind: 1, alpha: 1.0 }],
        lang: "ru",
        move_mode: false,
        now_ms: 1500,
        roster_scroll: 0.0,
        work_h: WIN_H,
    }
}

/// Render the settings window; returns the buffer + interactive regions + content height.
pub fn render(m: &SettingsModel, fonts: &Fonts, icons: &Icons, k: f32) -> (Buf, Vec<Region>, f32) {
    let pal = Palette::from_appearance(&m.app);
    let c = Ctx { pal: &pal, fonts, fs: 13.0 };
    let mut buf = SBuf {
        b: Buf::new(((WIN_W as f32) * k).round() as i32, ((WIN_H as f32) * k).round() as i32),
        k,
    };
    let mut regions: Vec<Region> = Vec::new();
    let mut open_sel_at: Option<(f32, f32)> = None;

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
    buf.text(&c.fonts, 20.0, 28.0, &full, 15.0, true, pal.text);
    let ver = format!("v{}", m.version);
    let tw_full = c.fonts.measure(&full, 15.0, true);
    let vw = c.fonts.measure(&ver, 11.0, true);
    let vpx = 20.0 + tw_full + 10.0;
    buf.round_rect(vpx, 13.0, vw + 16.0, 20.0, 10.0, pal.row_hover);
    buf.text(&c.fonts, vpx + 8.0, 27.0, &ver, 11.0, true, pal.muted);
    let (cx, cy) = (WIN_W - 42.0, 25.0);
    buf.circle(cx, cy, 15.0, pal.row_hover);
    buf.line(cx - 5.0, cy - 5.0, cx + 5.0, cy + 5.0, 2.2, pal.text);
    buf.line(cx + 5.0, cy - 5.0, cx - 5.0, cy + 5.0, 2.2, pal.text);
    regions.push(Region {
        rect: crate::layout::Rect { x: cx - 15.0, y: cy - 15.0, w: 30.0, h: 30.0 },
        ctrl: Ctrl::Close,
    });
    // dragging the title strip moves the window (like a normal app window)
    regions.push(Region {
        rect: crate::layout::Rect { x: 0.0, y: 0.0, w: WIN_W, h: 44.0 },
        ctrl: Ctrl::DragWindow,
    });
    buf.fill_rect(0.0, 44.0, WIN_W, 1.0, pal.panel_border);

    // ---- left pane (scrollable) ----
    let pane_x = 182.0;
    let pane_w = 452.0;
    let pane_y = 58.0;
    let pane_h = PANE_H;
    let mut y = pane_y - m.scroll;

    let section = |buf: &mut SBuf, c: &Ctx, y: f32, text: &str| {
        buf.text(&c.fonts, pane_x + 4.0, y + 16.0, text, 14.0, true, c.pal.accent);
        buf.fill_rect(pane_x + 4.0, y + 23.5, pane_w - 8.0, 1.6, c.pal.accent.scale_a(180.0));
        buf.fill_rect(pane_x + 4.0, y + 25.5, pane_w - 8.0, 1.2, c.pal.accent.scale_a(60.0));
    };

    // ---- left nav: pages (big highlightable buttons) ----
    {
        let pages = [
            (0u8, tr(&m.lang, "Вид", "Appearance")),
            (1, tr(&m.lang, "Элементы", "Elements")),
            (2, tr(&m.lang, "Связь", "Connection")),
            (3, tr(&m.lang, "Обновления", "Updates")),
            (4, tr(&m.lang, "Ещё", "More")),
        ];
        let (bx, bw, bh) = (12.0f32, 156.0f32, 48.0f32);
        let mut ny = pane_y;
        for (n, label) in pages {
            let active = m.page == n;
            if active {
                buf.round_rect(bx, ny, bw, bh, 16.0, pal.accent.scale_a(235.0));
                buf.stroke_round_rect(
                    bx - 1.0,
                    ny - 1.0,
                    bw + 2.0,
                    bh + 2.0,
                    17.0,
                    pal.accent.scale_a(95.0),
                    1.4,
                );
                // soft glow halo (vid-3)
                buf.stroke_round_rect(
                    bx - 3.5,
                    ny - 3.5,
                    bw + 7.0,
                    bh + 7.0,
                    19.5,
                    pal.accent.scale_a(42.0),
                    1.2,
                );
                let tw = c.fonts.measure(&label, 14.0, true);
                buf.text(
                    &c.fonts,
                    bx + (bw - tw) * 0.5,
                    ny + 30.0,
                    &label,
                    14.0,
                    true,
                    Rgba::rgb(255, 255, 255),
                );
            } else {
                // glass pill (vid-3) with generous radius (vid-2)
                buf.round_rect(
                    bx,
                    ny,
                    bw,
                    bh,
                    16.0,
                    crate::theme::mix(pal.panel_bg, Rgba::rgb(255, 255, 255), 0.05),
                );
                buf.stroke_round_rect(bx, ny, bw, bh, 16.0, pal.muted.scale_a(135.0), 1.1);
                let tw = c.fonts.measure(&label, 13.5, false);
                buf.text(&c.fonts, bx + (bw - tw) * 0.5, ny + 29.5, &label, 13.5, false, pal.muted);
            }
            regions.push(Region {
                rect: crate::layout::Rect { x: bx, y: ny, w: bw, h: bh },
                ctrl: Ctrl::Nav(n),
            });
            ny += bh + 12.0;
        }
    }

    // frosted glass content column with a soft accent glow (vid-3)
    {
        let (gx, gy, gw, gh) = (pane_x - 7.0, pane_y - 7.0, pane_w + 14.0, pane_h + 14.0);
        buf.round_rect(
            gx,
            gy,
            gw,
            gh,
            17.0,
            crate::theme::mix(pal.panel_bg, Rgba::rgb(255, 255, 255), 0.05).scale_a(232.0),
        );
        buf.stroke_round_rect(gx, gy, gw, gh, 17.0, pal.accent.scale_a(70.0), 1.2);
        buf.stroke_round_rect(
            gx - 2.5,
            gy - 2.5,
            gw + 5.0,
            gh + 5.0,
            19.5,
            pal.accent.scale_a(32.0),
            1.1,
        );
    }
    buf.set_clip(pane_x, pane_y, pane_w, pane_h);
    // ---- page content ----
    if m.page == 0 {
        // Вид: стиль, пресеты, цвета, размеры и эффекты, панель, положение
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
        buf.text(&c.fonts, pane_x + 4.0, y + 12.0, &cap, 11.5, false, pal.muted);
        y += 26.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "Размеры и эффекты", "Size & effects"));
    y += 34.0;
    let sliders: [(&str, &str, &str); 8] = [
        ("scale", "Масштаб", "Scale"),
        ("font_size", "Шрифт", "Font size"),
        ("panel_width", "Ширина панели", "Panel width"),
        ("panel_max_height", "Максимальная высота", "Max height"),
        ("opacity", "Прозрачность", "Opacity"),
        ("glow", "Свечение", "Glow"),
        ("blur", "Размытие", "Blur"),
        ("radius", "Скругление", "Corner radius"),
    ];
    for (key, ru, en) in sliders {
        let label = tr(&m.lang, ru, en);
        let value = get_slider(&m.app, key);
        let int_unit = match key {
            "font_size" | "blur" | "radius" | "panel_width" | "panel_max_height" => true,
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

    section(&mut buf, &c, y, &tr(&m.lang, "Панель", "Panel"));
    y += 34.0;
    for (key, ru, en) in [
        ("show_header", "Шапка", "Header"),
        ("show_server_name", "Имя сервера", "Server name"),
        ("show_user_count", "Счётчик", "User count"),
        ("compact", "Компактный", "Compact"),
        ("animations", "Анимации", "Animations"),
    ] {
        let label = tr(&m.lang, ru, en);
        let on = get_toggle(&m.app, key);
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 36.0 },
            ctrl: Ctrl::Toggle(key),
        });
        y += 40.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "Положение на экране", "Screen corner"));
    y += 34.0;
    // corner grid 2x2 (same control as the web main)
    {
        let gx = pane_x + 4.0;
        let gw = 240.0;
        let gh = 128.0;
        buf.round_rect(gx, y, gw, gh, 10.0, pal.row_hover);
        buf.stroke_round_rect(gx, y, gw, gh, 10.0, pal.muted.scale_a(80.0), 1.0);
        let cell = gw / 2.0;
        let chh = gh / 2.0;
        for (i, key) in ["tl", "tr", "bl", "br"].iter().enumerate() {
            let cx = gx + (i % 2) as f32 * cell;
            let cy = y + (i / 2) as f32 * chh;
            let active = m.app.corner == *key;
            if active {
                buf.round_rect(cx + 3.0, cy + 3.0, cell - 6.0, chh - 6.0, 8.0, pal.accent.scale_a(46.0));
                buf.stroke_round_rect(cx + 3.0, cy + 3.0, cell - 6.0, chh - 6.0, 8.0, pal.accent, 1.4);
            }
            let mw = 26.0f32;
            let mh = 16.0f32;
            let (mx, my) = match *key {
                "tl" => (cx + 12.0, cy + 11.0),
                "tr" => (cx + cell - 12.0 - mw, cy + 11.0),
                "bl" => (cx + 12.0, cy + chh - 11.0 - mh),
                _ => (cx + cell - 12.0 - mw, cy + chh - 11.0 - mh),
            };
            buf.round_rect(mx, my, mw, mh, 3.0, if active { pal.accent } else { pal.muted.scale_a(170.0) });
            regions.push(Region {
                rect: crate::layout::Rect { x: cx, y: cy, w: cell, h: chh },
                ctrl: Ctrl::Corner(i as u8),
            });
        }
        y += gh + 12.0;
    }
    for (key, ru, en) in [("margin_x", "Координата X", "X coordinate"), ("margin_y", "Координата Y", "Y coordinate")] {
        let label = tr(&m.lang, ru, en);
        let value = get_slider(&m.app, key);
        let track = draw_slider(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &format!("{value:.0}"), value, slider_range(key));
        regions.push(Region { rect: track, ctrl: Ctrl::Slider(key) });
        y += 52.0;
    }

    {
        let label = tr(&m.lang, "Слой оверлея", "Overlay layer");
        let cur = select_index("overlay_layer", m);
        let opts = select_options("overlay_layer", &m.lang);
        let value = opts.iter().find(|(_, i)| *i == cur).map(|(l, _)| l.clone()).unwrap_or_default();
        let open = m.open_select == Some("overlay_layer");
        let boxrect = draw_select(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &value, open);
        if open {
            open_sel_at = Some((boxrect.x, boxrect.y));
        }
        regions.push(Region { rect: boxrect, ctrl: Ctrl::Select("overlay_layer") });
        y += 36.0;
    }
    {
        let label = tr(&m.lang, "Показывать оверлей", "Show overlay");
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, m.desktop.visible);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 36.0 },
            ctrl: Ctrl::Toggle("visible"),
        });
        y += 40.0;
    }
    } else if m.page == 1 {
        // Элементы: состав канала, речь и шёпот, чат, события
    section(&mut buf, &c, y, &tr(&m.lang, "Состав канала", "Channel roster"));
    y += 34.0;
    for (key, ru, en) in [
        ("show_avatars", "Аватарки", "Avatars"),
        ("show_badges", "Бейджи", "Badges"),
        ("show_self", "Себя", "Self"),
        ("hide_query", "Скрыть query", "Hide query"),
        ("hide_empty", "Скрыть пустой", "Hide empty"),
    ] {
        let label = tr(&m.lang, ru, en);
        let on = get_toggle(&m.app, key);
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 36.0 },
            ctrl: Ctrl::Toggle(key),
        });
        y += 40.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "Речь и шёпот", "Voice & whispers"));
    y += 34.0;
    for (key, ru, en) in [
        ("talking_first", "Говорящие первыми", "Talking first"),
        ("only_talking", "Только говорящие", "Only talking"),
        ("show_whispers", "Шёпот", "Whispers"),
    ] {
        let label = tr(&m.lang, ru, en);
        let on = get_toggle(&m.app, key);
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 36.0 },
            ctrl: Ctrl::Toggle(key),
        });
        y += 40.0;
    }
    {
        let label = tr(&m.lang, "Индикатор речи", "Talking indicator");
        let cur = select_index("eq_style", m);
        let opts = select_options("eq_style", &m.lang);
        let value = opts.iter().find(|(_, i)| *i == cur).map(|(l, _)| l.clone()).unwrap_or_default();
        let open = m.open_select == Some("eq_style");
        let boxrect = draw_select(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &value, open);
        if open {
            open_sel_at = Some((boxrect.x, boxrect.y));
        }
        regions.push(Region { rect: boxrect, ctrl: Ctrl::Select("eq_style") });
        y += 36.0;
    }
    {
        // unit lives in the value, not the label («убери ", с"»)
        let label = tr(&m.lang, "Задержка шёпота", "Whisper delay");
        let value = get_slider(&m.app, "whisper_delay");
        let vstr = format!("{value:.0} {}", tr(&m.lang, "с", "s"));
        let track = draw_slider(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &vstr, value, slider_range("whisper_delay"));
        regions.push(Region { rect: track, ctrl: Ctrl::Slider("whisper_delay") });
        y += 52.0;
    }
    {
        let label = tr(&m.lang, "Задержка исчезновения говорящих", "Talking fade delay");
        let value = get_slider(&m.app, "talking_fade");
        let vstr = if value <= 0.0 {
            tr(&m.lang, "выкл", "off")
        } else {
            format!("{value:.0} {}", tr(&m.lang, "с", "s"))
        };
        let track = draw_slider(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &vstr, value, slider_range("talking_fade"));
        regions.push(Region { rect: track, ctrl: Ctrl::Slider("talking_fade") });
        y += 52.0;
        // what the delay is tied to («не понятно к чему привязана»)
        let hint = tr(
            &m.lang,
            "Подсветка говорящего плавно гаснет через это время после речи",
            "The talking highlight fades out this many seconds after speech",
        );
        buf.text(&c.fonts, pane_x + 8.0, y + 2.0, &hint, c.fs * 0.85, false, c.pal.text.scale_a(150.0));
        y += 22.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "Чат и уведомления", "Chat & notifications"));
    y += 34.0;
    for (key, ru, en) in [
        ("show_chat", "Чат", "Chat"),
        ("chat_private", "Только личные", "Private only"),
        ("show_unread_badge", "Бейдж непрочитанных", "Unread badge"),
    ] {
        let label = tr(&m.lang, ru, en);
        let on = get_toggle(&m.app, key);
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 36.0 },
            ctrl: Ctrl::Toggle(key),
        });
        y += 40.0;
    }
    {
        // «Чат(панель событий) пропадает через определённое время» — chat rows
        // live `chat_delay` seconds and fade out at the end
        let label = tr(&m.lang, "Задержка чата", "Chat delay");
        let value = get_slider(&m.app, "chat_delay");
        let vstr = format!("{value:.0} {}", tr(&m.lang, "с", "s"));
        let track = draw_slider(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &vstr, value, slider_range("chat_delay"));
        regions.push(Region { rect: track, ctrl: Ctrl::Slider("chat_delay") });
        y += 52.0;
    }

    section(&mut buf, &c, y, &tr(&m.lang, "События сервера", "Server events"));
    y += 34.0;
    {
        let label = tr(&m.lang, "Показывать события", "Show events");
        let on = get_toggle(&m.app, "show_events");
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 36.0 },
            ctrl: Ctrl::Toggle("show_events"),
        });
        y += 40.0;
    }
    {
        let label = tr(&m.lang, "Количество событий", "Event count");
        let value = get_slider(&m.app, "events_count");
        let track = draw_slider(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &format!("{value:.0}"), value, slider_range("events_count"));
        regions.push(Region { rect: track, ctrl: Ctrl::Slider("events_count") });
        y += 52.0;
    }
    {
        let label = tr(&m.lang, "Задержка событий", "Event delay");
        let value = get_slider(&m.app, "events_delay");
        let vstr = format!("{value:.0} {}", tr(&m.lang, "с", "s"));
        let track = draw_slider(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &vstr, value, slider_range("events_delay"));
        regions.push(Region { rect: track, ctrl: Ctrl::Slider("events_delay") });
        y += 52.0;
    }
    for (tkey, skey, ru, en) in [
        ("show_ev_join", "ev_scope_join", "Входы", "Joins"),
        ("show_ev_leave", "ev_scope_leave", "Выходы", "Leaves"),
        ("show_ev_move", "ev_scope_move", "Переходы", "Moves"),
        ("show_ev_poke", "ev_scope_group", "Поке", "Pokes"),
        ("show_ev_chat", "ev_scope_chat", "Чат", "Chat"),
    ] {
        let label = tr(&m.lang, ru, en);
        let on = get_toggle(&m.app, tkey);
        let tw = pane_w - 8.0 - 200.0;
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, tw, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: tw, h: 36.0 },
            ctrl: Ctrl::Toggle(tkey),
        });
        let cur = select_index(skey, m);
        let opts = select_options(skey, &m.lang);
        let value = opts.iter().find(|(_, i)| *i == cur).map(|(l, _)| l.clone()).unwrap_or_default();
        let open = m.open_select == Some(skey);
        let boxrect = draw_select(&c, &mut buf, pane_x + 4.0, y + 5.0, pane_w - 8.0, "", &value, open);
        if open {
            open_sel_at = Some((boxrect.x, boxrect.y));
        }
        regions.push(Region { rect: boxrect, ctrl: Ctrl::Select(skey) });
        y += 44.0;
    }

    } else if m.page == 2 {
        // Связь
    section(&mut buf, &c, y, &tr(&m.lang, "Связь", "Connection"));
    y += 34.0;
    {
        let label = tr(&m.lang, "Источник данных", "Data source");
        let cur = select_index("source", m);
        let opts = select_options("source", &m.lang);
        let value = opts.iter().find(|(_, i)| *i == cur).map(|(l, _)| l.clone()).unwrap_or_default();
        let open = m.open_select == Some("source");
        let boxrect = draw_select(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &value, open);
        if open {
            open_sel_at = Some((boxrect.x, boxrect.y));
        }
        regions.push(Region { rect: boxrect, ctrl: Ctrl::Select("source") });
        y += 36.0;
    }
    {
        buf.text(&c.fonts, pane_x + 4.0, y + 13.0, "TeamSpeak 3", 12.5, true, pal.text);
        y += 24.0;
    }
    for (id, ru, en) in [
        ("ts3_host", "Хост", "Host"),
        ("ts3_port", "Порт", "Port"),
        ("ts3_key", "API ключ", "API key"),
    ] {
        let label = tr(&m.lang, ru, en);
        let focused = m.focus == Some(id);
        let rect = draw_field(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &m.field_text(id), focused, focused && m.select_all);
        regions.push(Region { rect, ctrl: Ctrl::Field(id) });
        y += 56.0;
    }
    {
        let label = tr(&m.lang, "Установить плагин", "Install plugin");
        draw_button(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, 34.0, &label);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 34.0 },
            ctrl: Ctrl::InstallPlugin,
        });
        y += 42.0;
    }
    {
        buf.text(&c.fonts, pane_x + 4.0, y + 13.0, "TeamSpeak 6", 12.5, true, pal.text);
        y += 24.0;
    }
    for (id, ru, en) in [
        ("ts6_host", "Хост", "Host"),
        ("ts6_port", "Порт", "Port"),
        ("ts6_key", "API ключ", "API key"),
    ] {
        let label = tr(&m.lang, ru, en);
        let focused = m.focus == Some(id);
        let rect = draw_field(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &m.field_text(id), focused, focused && m.select_all);
        regions.push(Region { rect, ctrl: Ctrl::Field(id) });
        y += 56.0;
    }
    {
        let label = tr(&m.lang, "Переподключить", "Reconnect");
        draw_button(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, 34.0, &label);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 34.0 },
            ctrl: Ctrl::Reconnect,
        });
        y += 42.0;
    }
    if !m.conn_note.is_empty() {
        buf.text(&c.fonts, pane_x + 4.0, y + 12.0, &m.conn_note, 11.5, false, pal.muted);
        y += 24.0;
    }
    {
        // «Группы сервера» — скрыты изначально, раскрываются по клику
        let t = tr(&m.lang, "Группы сервера", "Server groups");
        let (chx, chy) = (pane_x + 12.0, y + 11.0);
        if m.groups_open {
            buf.line(chx - 4.0, chy + 3.0, chx, chy - 3.0, 1.6, pal.accent);
            buf.line(chx, chy - 3.0, chx + 4.0, chy + 3.0, 1.6, pal.accent);
        } else {
            buf.line(chx - 4.0, chy - 3.0, chx, chy + 3.0, 1.6, pal.muted);
            buf.line(chx, chy + 3.0, chx + 4.0, chy - 3.0, 1.6, pal.muted);
        }
        buf.text(&c.fonts, pane_x + 26.0, y + 13.0, &t, 12.5, true, pal.text);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 24.0 },
            ctrl: Ctrl::ToggleGroups,
        });
        y += 24.0;
    }
    if m.groups_open {
    for (i, (gid, _cls)) in m.groups.iter().enumerate() {
        // «должно быть явно» — каждая группа это выпадающий выбор класса:
        // слева номер группы, справа явный список классов
        let key = GROUP_KEYS[i];
        let cur = select_index(key, m);
        let opts = select_options(key, &m.lang);
        let value = opts
            .iter()
            .find(|(_, idx)| *idx == cur)
            .map(|(l, _)| l.clone())
            .unwrap_or_default();
        let open = m.open_select == Some(key);
        let label = format!("#{}", gid);
        let boxrect = draw_select(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &value, open);
        if open {
            open_sel_at = Some((boxrect.x, boxrect.y));
        }
        regions.push(Region { rect: boxrect, ctrl: Ctrl::Select(key) });
        y += 38.0;
    }
    }

    } else if m.page == 3 {
        // Обновления
    section(&mut buf, &c, y, &tr(&m.lang, "Обновления", "Updates"));
    y += 34.0;
    {
        let ver = format!("TS3-TS6 Notifier v{}", m.version);
        buf.text(&c.fonts, pane_x + 4.0, y + 13.0, &ver, 12.0, false, pal.muted);
        y += 26.0;
    }
    {
        let label = tr(&m.lang, "Проверить обновления", "Check for updates");
        draw_button(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, 34.0, &label);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 34.0 },
            ctrl: Ctrl::CheckUpdate,
        });
        y += 42.0;
    }
    if !m.update_note.is_empty() {
        buf.text(&c.fonts, pane_x + 4.0, y + 12.0, &m.update_note, 11.5, false, pal.muted);
        y += 24.0;
    }
    if m.update_ready {
        // «автообновление»: installer is downloaded — offer the restart
        let label = tr(&m.lang, "Перезапустить сейчас", "Restart now");
        draw_button(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, 34.0, &label);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 34.0 },
            ctrl: Ctrl::InstallUpdate,
        });
        y += 42.0;
    }
    } else {
        // Ещё: система + инструменты
    section(&mut buf, &c, y, &tr(&m.lang, "Система", "System"));
    y += 34.0;
    {
        let label = tr(&m.lang, "Язык", "Language");
        let cur = select_index("language", m);
        let opts = select_options("language", &m.lang);
        let value = opts.iter().find(|(_, i)| *i == cur).map(|(l, _)| l.clone()).unwrap_or_default();
        let open = m.open_select == Some("language");
        let boxrect = draw_select(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, &value, open);
        if open {
            open_sel_at = Some((boxrect.x, boxrect.y));
        }
        regions.push(Region { rect: boxrect, ctrl: Ctrl::Select("language") });
        y += 36.0;
    }
    for (key, ru, en) in [
        ("elevate", "Запуск от имени администратора", "Run as administrator"),
        ("autostart", "Запуск с системой", "Launch at startup"),
        ("click_through", "Мышь проходит сквозь оверлей", "Mouse passes through the overlay"),
    ] {
        let label = tr(&m.lang, ru, en);
        let on = match key {
            "elevate" => m.desktop.elevate,
            "click_through" => m.desktop.click_through,
            _ => m.desktop.autostart,
        };
        draw_toggle(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, &label, on);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 36.0 },
            ctrl: Ctrl::Toggle(key),
        });
        y += 40.0;
    }
    // ---- Связь (Connection): source + TS3 plugin + TS6 ----
    section(&mut buf, &c, y, &tr(&m.lang, "Инструменты", "Tools"));
    y += 34.0;
    {
        let label = tr(&m.lang, "Скопировать ссылку для OBS", "Copy OBS browser-source URL");
        draw_button(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, 34.0, &label);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 34.0 },
            ctrl: Ctrl::CopyObs,
        });
        y += 42.0;
    }
    {
        let label = tr(&m.lang, "Скопировать диагностику", "Copy diagnostics report");
        draw_button(&c, &mut buf, pane_x + 4.0, y, pane_w - 8.0, 34.0, &label);
        regions.push(Region {
            rect: crate::layout::Rect { x: pane_x + 4.0, y, w: pane_w - 8.0, h: 34.0 },
            ctrl: Ctrl::CopyDiag,
        });
        y += 42.0;
    }
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
    buf.round_rect(pv_x, pane_y, pv_w, card_h, 14.0, crate::theme::mix(pal.panel_bg, Rgba::rgb(255, 255, 255), 0.055));
    buf.stroke_round_rect(pv_x, pane_y, pv_w, card_h, 14.0, pal.accent.scale_a(70.0), 1.2);
    buf.stroke_round_rect(
        pv_x - 2.5,
        pane_y - 2.5,
        pv_w + 5.0,
        card_h + 5.0,
        16.5,
        pal.accent.scale_a(32.0),
        1.1,
    );
    let cap = tr(&m.lang, "Предпросмотр", "Preview");
    buf.text(&c.fonts, pv_x + 14.0, pane_y + 23.0, &cap, 13.0, true, pal.text);
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
    buf.text(&c.fonts, pv_x + (pv_w - h1w) * 0.5, btn_y + 64.0, &hint1, 11.5, false, pal.muted);
    buf.text(&c.fonts, pv_x + (pv_w - h2w) * 0.5, btn_y + 82.0, &hint2, 11.5, false, pal.muted);

    // ---- dropdown popup (topmost layer) ----
    if let (Some(key), Some((px, py))) = (m.open_select, open_sel_at) {
        let opts = select_options(key, &m.lang);
        let cur = select_index(key, m);
        let row_h = 30.0f32;
        let pop_h = opts.len() as f32 * row_h + 10.0;
        let mut pop_y = py + 32.0;
        if pop_y + pop_h > WIN_H - 8.0 {
            pop_y = (py - pop_h - 2.0).max(8.0);
        }
        buf.round_rect(px, pop_y, 190.0, pop_h, 8.0, crate::theme::mix(pal.panel_bg, Rgba::rgb(0, 0, 0), 0.35).scale_a(252.0));
        buf.stroke_round_rect(px, pop_y, 190.0, pop_h, 8.0, pal.accent.scale_a(170.0), 1.2);
        for (i, (lbl, idx)) in opts.iter().enumerate() {
            let ry = pop_y + 5.0 + i as f32 * row_h;
            if *idx == cur {
                buf.round_rect(px + 4.0, ry, 182.0, row_h - 4.0, 6.0, pal.accent.scale_a(52.0));
            }
            buf.text(&c.fonts, px + 12.0, ry + 17.5, lbl, 12.0, false, pal.text);
            regions.push(Region {
                rect: crate::layout::Rect { x: px, y: ry, w: 190.0, h: row_h - 2.0 },
                ctrl: Ctrl::Option(key, *idx),
            });
        }
    }
    // dropdown options win clicks over content underneath
    let (mut front, back): (Vec<Region>, Vec<Region>) =
        regions.into_iter().partition(|r| matches!(r.ctrl, Ctrl::Option(..)));
    front.extend(back);
    let regions = front;

    (buf.b, regions, content_h)
}

/* ---------------- window runner ---------------- */

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::http::App;
use crate::overlay::{Ev, Host};

static SETTINGS_RECT: std::sync::Mutex<Option<(f64, f64, f64, f64)>> = std::sync::Mutex::new(None);

static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);

static CLOSE_REQ: AtomicBool = AtomicBool::new(false);

/// WM_CLOSE / Alt+F4 on the settings window — run_loop exits and destroys it.
pub fn request_close() {
    CLOSE_REQ.store(true, Ordering::SeqCst);
}

/// True while the settings window is open (the overlay leaves topmost z-order
/// alone then, so the separate settings app stays usable above it).
pub fn is_open() -> bool {
    SETTINGS_OPEN.load(Ordering::SeqCst)
}

pub fn set_window_rect(x: f64, y: f64, w: f64, h: f64) {
    *SETTINGS_RECT.lock().unwrap() = Some((x, y, w, h));
}

/// Screen-space hit test used to route wheel input to the settings window first.
pub fn rect_contains(px: f64, py: f64) -> bool {
    match *SETTINGS_RECT.lock().unwrap() {
        Some((x, y, w, h)) => px >= x && px <= x + w && py >= y && py <= y + h,
        None => false,
    }
}

/// Open the settings as a SEPARATE app in the taskbar («отдельным приложением
/// в панели задач»). At most one window at a time; reopening is allowed after
/// the window closes.
pub fn spawn_once(app: Arc<App>, _port: u16) {
    if SETTINGS_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    struct OpenGuard;
    impl Drop for OpenGuard {
        fn drop(&mut self) {
            SETTINGS_OPEN.store(false, Ordering::SeqCst);
        }
    }
    std::thread::Builder::new()
        .name("settings".into())
        .spawn(move || {
            // RAII: even a panic inside the settings UI must release the
            // "already open" flag — otherwise settings can never open again
            let _guard = OpenGuard;
            #[cfg(any(windows, tsn_win32))]
            crate::overlay::win32::run_settings(app.clone());
            #[cfg(not(any(windows, tsn_win32)))]
            let _ = &app;
        })
        .ok();
}

/* ---------------- update check ---------------- */

/// Human-readable update status shown in «Ещё» (also mirrored over WS).
pub static UPDATE_NOTE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// "v1.2.3"-style tag → numeric parts (missing parts count as 0).
fn ver_parts(tag: &str) -> Vec<u64> {
    tag.trim()
        .trim_start_matches('v')
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

/// True when `remote` names a strictly newer version than `current`.
fn ver_newer(remote: &str, current: &str) -> bool {
    let (a, b) = (ver_parts(remote), ver_parts(current));
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// Installer download target — fixed release asset name (release plan).
pub const UPDATE_SETUP_URL: &str =
    "https://github.com/drednight/TS3-TS6-Notifier/releases/latest/download/TS3-TS6-Notifier-Setup.exe";

fn update_setup_path() -> std::path::PathBuf {
    std::env::temp_dir().join("tsn-update-setup.exe")
}

fn update_marker_path() -> std::path::PathBuf {
    std::env::temp_dir().join("tsn-update-pending.txt")
}

/// Translate an update-check mailbox code into the «Ещё» status line.
pub fn update_note_text(code: &str, lang: &str) -> String {
    let ru = lang == "ru";
    if code == "checking" {
        tr(lang, "Проверка обновлений…", "Checking for updates…")
    } else if code == "latest" {
        tr(lang, "У вас последняя версия ✓", "You are up to date ✓")
    } else if code == "error" {
        tr(lang, "Не удалось проверить обновления", "Update check failed")
    } else if let Some(v) = code.strip_prefix("available ") {
        if ru {
            format!("Новая версия: {v} — скачиваем…")
        } else {
            format!("New version: {v} — downloading…")
        }
    } else if let Some(v) = code.strip_prefix("downloaded ") {
        if ru {
            format!("Обновление {v} скачано — установится при выходе из приложения")
        } else {
            format!("Update {v} downloaded — installs when you quit the app")
        }
    } else if let Some(v) = code.strip_prefix("dlfail ") {
        if ru {
            format!("Новая версия: {v} — не удалось скачать автоматически")
        } else {
            format!("New version: {v} — automatic download failed")
        }
    } else {
        code.to_string()
    }
}

/// Launch a downloaded update installer (if any), consuming the marker so the
/// installer runs exactly once. True when the installer was started.
pub fn try_launch_pending_update() -> bool {
    let marker = update_marker_path();
    let target = match std::fs::read_to_string(&marker) {
        Ok(t) => t.trim().to_string(),
        Err(_) => return false,
    };
    let _ = std::fs::remove_file(&marker);
    if target.is_empty() || !std::path::Path::new(&target).exists() {
        return false;
    }
    #[cfg(any(windows, tsn_win32))]
    {
        crate::overlay::win32::shell_open(&target);
        true
    }
    #[cfg(not(any(windows, tsn_win32)))]
    {
        let _ = target;
        false
    }
}

/// Download the release installer to a temp file («автообновление» step 2).
#[cfg(any(windows, tsn_win32))]
fn download_update_setup() -> bool {
    crate::overlay::win32::fetch_url_to_file(UPDATE_SETUP_URL, &update_setup_path())
}
#[cfg(not(any(windows, tsn_win32)))]
fn download_update_setup() -> bool {
    false
}

/// Check GitHub Releases for a newer version (background thread). Broadcasts
/// `update-status` messages the web UI already understands, stores the human
/// line in [`UPDATE_NOTE`] for the settings window, and downloads the update
/// installer so it installs on quit / «Перезапустить сейчас».
pub fn start_update_check(app: Arc<App>) {
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            *UPDATE_NOTE.lock().unwrap() = "checking".into();
            app.broadcast(&crate::json::obj(vec![
                ("type", crate::json::Json::s("update-status")),
                ("status", crate::json::Json::s("checking")),
            ]));
            let tag = fetch_release_tag();
            match tag {
                Some(tag) if ver_newer(&tag, &app.version) => {
                    *UPDATE_NOTE.lock().unwrap() = format!("available {tag}");
                    app.broadcast(&crate::json::obj(vec![
                        ("type", crate::json::Json::s("update-status")),
                        ("status", crate::json::Json::s("available")),
                        ("version", crate::json::Json::s(tag.clone())),
                    ]));
                    // «автообновление»: fetch the installer right away — it then
                    // installs on quit or via «Перезапустить сейчас»
                    if download_update_setup() {
                        let _ = std::fs::write(
                            update_marker_path(),
                            update_setup_path().to_string_lossy().as_bytes(),
                        );
                        *UPDATE_NOTE.lock().unwrap() = format!("downloaded {tag}");
                        app.broadcast(&crate::json::obj(vec![
                            ("type", crate::json::Json::s("update-status")),
                            ("status", crate::json::Json::s("downloaded")),
                            ("version", crate::json::Json::s(tag.clone())),
                        ]));
                        // «предлагает обновиться и перезапустить» — tray prompt
                        #[cfg(any(windows, tsn_win32))]
                        {
                            let ru = app.store.lock().unwrap().get().appearance.language != "en";
                            let msg = if ru {
                                format!("Обновление {tag} скачано — перезапустите приложение, чтобы установить")
                            } else {
                                format!("Update {tag} downloaded — restart the app to install")
                            };
                            crate::overlay::win32::tray_balloon("TS3-TS6 Notifier", &msg);
                        }
                    } else {
                        *UPDATE_NOTE.lock().unwrap() = format!("dlfail {tag}");
                    }
                }
                Some(_) => {
                    *UPDATE_NOTE.lock().unwrap() = "latest".into();
                    app.broadcast(&crate::json::obj(vec![
                        ("type", crate::json::Json::s("update-status")),
                        ("status", crate::json::Json::s("latest")),
                    ]));
                }
                None => {
                    *UPDATE_NOTE.lock().unwrap() = "error".into();
                    app.broadcast(&crate::json::obj(vec![
                        ("type", crate::json::Json::s("update-status")),
                        ("status", crate::json::Json::s("error")),
                    ]));
                }
            }
        })
        .ok();
}

/// Latest release tag from the GitHub API (""-free Option), or None.
fn fetch_release_tag() -> Option<String> {
    #[cfg(any(windows, tsn_win32))]
    {
        let url = "https://api.github.com/repos/drednight/TS3-TS6-Notifier/releases/latest";
        let tmp = std::env::temp_dir().join("tsn-latest-release.json");
        if !crate::overlay::win32::fetch_url_to_file(url, &tmp) {
            return None;
        }
        let text = std::fs::read_to_string(&tmp).ok()?;
        let json = crate::json::Json::parse(&text).ok()?;
        let tag = json.get("tag_name")?.as_str()?.to_string();
        if tag.is_empty() {
            None
        } else {
            Some(tag)
        }
    }
    #[cfg(not(any(windows, tsn_win32)))]
    {
        // dev/test hosts have no WinINet — report "no update info"
        None
    }
}

/// The desktop prefs object written by persist() — ALL toggles included
/// (visible/autostart were silently dropped once).
fn desktop_patch(d: &crate::config::DesktopPrefs) -> Vec<(&'static str, crate::json::Json)> {
    vec![
        ("elevate", crate::json::Json::b(d.elevate)),
        ("clickThrough", crate::json::Json::b(d.click_through)),
        ("overlayLayer", crate::json::Json::s(d.overlay_layer.clone())),
        ("autostart", crate::json::Json::b(d.autostart)),
        ("visible", crate::json::Json::b(d.visible)),
    ]
}

fn persist(app: &Arc<App>, m: &SettingsModel) -> bool {
    // «Запуск с системой» — keep the HKCU Run entry in sync with the toggle
    #[cfg(any(windows, tsn_win32))]
    crate::overlay::win32::set_autostart(m.desktop.autostart);

    let d = &m.desktop;
    let ts_json = |t: &crate::config::TsEndpoint| {
        crate::json::obj(vec![
            ("host", crate::json::Json::s(t.host.clone())),
            ("port", crate::json::Json::n(t.port as f64)),
            ("apiKey", crate::json::Json::s(t.api_key.clone())),
        ])
    };
    let patch = crate::json::obj(vec![
        ("appearance", crate::config::appearance_to_json(&m.app)),
        ("desktop", crate::json::obj(desktop_patch(d))),
        ("source", crate::json::Json::s(m.source.clone())),
        ("ts3", ts_json(&m.ts3)),
        ("ts6", ts_json(&m.ts6)),
        (
            "classes",
            crate::json::obj(vec![
                ("enabled", crate::json::Json::b(true)),
                (
                    "map",
                    crate::json::obj(
                        m.groups
                            .iter()
                            .map(|(id, cls)| {
                                (
                                    id.as_str(),
                                    crate::json::Json::s(if cls.is_empty() { "none" } else { cls.as_str() }),
                                )
                            })
                            .collect::<Vec<_>>(),
                    ),
                ),
            ]),
        ),
    ]);
    let restart = app.store.lock().unwrap().update(&patch);
    if restart {
        (app.restart_connector)();
    }
    app.broadcast(&crate::json::obj(vec![
        ("type", crate::json::Json::s("config")),
        ("config", app.config_json()),
    ]));
    restart
}

/// Settings event loop (cross-platform; the host presents + pumps input).
pub fn run_loop(m: &mut SettingsModel, app: &Arc<App>, fonts: &Fonts, icons: &Icons, host: &mut dyn Host) {
    // the window lives in PHYSICAL pixels (DPI-aware thread); all region and
    // model math stays in logical units, so event coords are divided by `k`
    let k = ui_scale();
    let kw = (WIN_W as f32) * k;
    let kh = (WIN_H as f32) * k;
    let work = host.work_area();
    // «Отступ X/Y задают координаты по всему экрану» — sliders span the
    // whole work area (this thread reports PHYSICAL px; margins are logical)
    set_screen_dims(work.2 / k as f64, work.3 / k as f64);
    let cx = work.0 + (work.2 - kw as f64) * 0.5;
    let cy = work.1 + (work.3 - kh as f64) * 0.5;
    let mut pos: Option<(f64, f64)> = None;
    let mut activated = false;
    loop {
        if m.close_requested || CLOSE_REQ.swap(false, Ordering::SeqCst) {
            break;
        }
        let (x, y) = pos.unwrap_or((cx, cy));
        let (buf, regions, content_h) = render(m, fonts, icons, k);
        host.present(&buf, x, y, (x, y, kw as f64, kh as f64));
        pos = Some(host.window_pos());
        host.set_interactive(true);
        if !activated {
            activated = true;
            host.activate();
        }
        // publish our rect (PHYSICAL px) for the overlay's wheel routing
        if let Some((wx, wy)) = pos {
            set_window_rect(wx, wy, kw as f64, kh as f64);
        }
        // pump() reports SCREEN coordinates — the model works window-local
        for ev in host.pump() {
            match ev {
                Ev::Click(ex, ey) => {
                    let (ex, ey) = ((ex - x as f32) / k, (ey - y as f32) / k);
                    let mut hit = false;
                    for r in &regions {
                        if r.rect.contains(ex, ey) {
                            if matches!(r.ctrl, Ctrl::Slider(_)) {
                                continue;
                            }
                            m.click(r.ctrl, ex, ey);
                            hit = true;
                            break;
                        }
                    }
                    if !hit {
                        // click outside any control closes the open dropdown
                        m.close_select();
                    }
                }
                Ev::DragStart(ex, ey) => {
                    let (ex, ey) = ((ex - x as f32) / k, (ey - y as f32) / k);
                    // dragging the window header moves the whole window — but a
                    // press on the × must become a click: the native drag loop
                    // would swallow the mouse-up and Close would never fire
                    let over_close = regions.iter().any(|r| {
                        matches!(r.ctrl, Ctrl::Close) && r.rect.contains(ex, ey)
                    });
                    let on_header = !over_close && regions.iter().any(|r| {
                        matches!(r.ctrl, Ctrl::DragWindow) && r.rect.contains(ex, ey)
                    });
                    if on_header {
                        m.close_select();
                        host.begin_window_drag();
                        pos = Some(host.window_pos());
                    } else {
                        m.drag_begin(ex, ey, &regions);
                    }
                }
                Ev::DragMove(ex, _) => m.drag_to((ex - x as f32) / k),
                Ev::DragEnd => m.drag_end(),
                Ev::Wheel(dy) => m.wheel(dy, content_h),
                Ev::Text(t) => m.text_input(&t),
            }
        }
        {
            // consume the update-check mailbox (status codes from the thread)
            let mut note = UPDATE_NOTE.lock().unwrap();
            if !note.is_empty() {
                let code = std::mem::take(&mut *note);
                m.update_ready = code.starts_with("downloaded ");
                m.update_note = update_note_text(&code, &m.lang);
            }
        }
        if let Some(p) = m.pending.take() {
            match p {
                Pending::InstallPlugin => {
                    let res = crate::http::plugin_install(&app.plugin_dll);
                    let ok = matches!(res.get("ok"), Some(crate::json::Json::Bool(true)));
                    m.conn_note = if ok {
                        tr(&m.lang, "Плагин установлен ✓", "Plugin installed ✓")
                    } else {
                        tr(&m.lang, "Не удалось установить плагин", "Plugin install failed")
                    };
                }
                Pending::Reconnect => {
                    (app.restart_connector)();
                    m.conn_note = tr(&m.lang, "Переподключено ✓", "Reconnected ✓");
                }
                Pending::CheckUpdate => {
                    m.update_note = tr(&m.lang, "Проверка обновлений…", "Checking for updates…");
                    start_update_check(app.clone());
                }
                Pending::InstallUpdate => {
                    if try_launch_pending_update() {
                        std::process::exit(0);
                    }
                    m.update_note = tr(&m.lang, "Не удалось запустить установщик", "Failed to start the installer");
                }
                Pending::CopyObs => {
                    let url = format!("http://127.0.0.1:{}/?clean=1", crate::http::listen_port());
                    #[cfg(any(windows, tsn_win32))]
                    crate::overlay::win32::set_clipboard_text(&url);
                    #[cfg(not(any(windows, tsn_win32)))]
                    let _ = &url;
                    m.update_note = tr(&m.lang, "Ссылка скопирована ✓", "URL copied ✓");
                }
                Pending::CopyDiag => {
                    let report = crate::http::diagnostics(&app);
                    #[cfg(any(windows, tsn_win32))]
                    crate::overlay::win32::set_clipboard_text(&report);
                    #[cfg(not(any(windows, tsn_win32)))]
                    let _ = &report;
                    m.update_note = tr(&m.lang, "Отчёт скопирован ✓", "Report copied ✓");
                }
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
        if !m.changed {
            // tray «Скрыть/Показывать оверлей» must survive later persists of
            // our model — adopt external desktop changes while idle
            if let Ok(g) = app.store.try_lock() {
                let cfg = g.get();
                let (v, a) = (cfg.desktop.visible, cfg.desktop.autostart);
                m.desktop.visible = v;
                m.desktop.autostart = a;
                // corner snap / overlay drag write real coordinates back
                m.app.margin_x = cfg.appearance.margin_x;
                m.app.margin_y = cfg.appearance.margin_y;
                m.app.corner = cfg.appearance.corner.clone();
            }
        }
        if m.changed {
            m.changed = false;
            if persist(app, m) {
                m.conn_note = tr(&m.lang, "Переподключено ✓", "Reconnected ✓");
            }
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
    fn desktop_patch_keeps_visibility_and_autostart() {
        let mut d = crate::config::DesktopPrefs::default();
        d.visible = false;
        d.autostart = true;
        d.elevate = false;
        let patch = crate::json::obj(vec![("desktop", crate::json::obj(desktop_patch(&d)))]);
        let mut cfg = crate::config::AppConfig::default();
        cfg.update_from(&patch); // returns "reconnect needed", not "changed"
        assert!(!cfg.desktop.visible, "visible must persist");
        assert!(cfg.desktop.autostart, "autostart must persist");
        assert!(!cfg.desktop.elevate, "elevate must persist");
    }

    #[test]
    fn group_rows_use_explicit_class_selects() {
        let mut m = model();
        m.page = 2;
        m.groups_open = true;
        // pick «Лучник» for the first group via the select popup
        m.click(Ctrl::Select(GROUP_KEYS[0]), 0.0, 0.0);
        assert_eq!(m.open_select, Some(GROUP_KEYS[0]));
        let archer_idx = CLASS_CYCLE.iter().position(|c| *c == "archer").unwrap() as u8;
        m.click(Ctrl::Option(GROUP_KEYS[0], archer_idx), 0.0, 0.0);
        assert_eq!(m.groups[0].1, "archer");
        assert_eq!(m.open_select, None);
        // unmapped pick clears it
        m.click(Ctrl::Select(GROUP_KEYS[0]), 0.0, 0.0);
        m.click(Ctrl::Option(GROUP_KEYS[0], 0), 0.0, 0.0);
        assert_eq!(m.groups[0].1, "");
        // the picker renders as a select row
        let fonts = Fonts::load();
        let icons = Icons::load();
        let (_, regions, _) = render(&m, &fonts, &icons, 1.0);
        assert!(regions.iter().any(|r| r.ctrl == Ctrl::Select(GROUP_KEYS[0])));
    }

    #[test]
    fn margin_sliders_span_the_full_screen() {
        set_screen_dims(1920.0, 1080.0);
        let (x0, x1) = slider_range("margin_x");
        let (y0, y1) = slider_range("margin_y");
        assert_eq!((x0, x1), (0.0, 1920.0), "«координаты по всему экрану»");
        assert_eq!((y0, y1), (0.0, 1080.0));
        set_screen_dims(3840.0, 2160.0); // restore shared default for other tests
    }

    #[test]
    fn close_not_eaten_by_header_drag() {
        // the × region sits inside the header drag strip; the drag handler
        // must yield to Close (see run_loop Ev::DragStart)
        let m = model();
        let fonts = Fonts::load();
        let icons = Icons::load();
        let (_, regions, _) = render(&m, &fonts, &icons, 1.0);
        let close = regions.iter().find(|r| r.ctrl == Ctrl::Close).expect("close region");
        let drag = regions.iter().find(|r| r.ctrl == Ctrl::DragWindow).expect("drag region");
        assert!(
            drag.rect.contains(close.rect.x + 1.0, close.rect.y + 1.0),
            "test precondition: close is inside the drag strip"
        );
    }

    #[test]
    fn text_fields_edit_and_replace() {
        let mut m = model();
        m.click(Ctrl::Field("ts3_host"), 0.0, 0.0);
        assert_eq!(m.focus, Some("ts3_host"));
        m.text_input("ex");
        assert_eq!(m.ts3.host, "ex"); // select-all focus replaced the default
        m.text_input("ample");
        assert_eq!(m.ts3.host, "example");
        m.text_input("\x08\x08");
        assert_eq!(m.ts3.host, "examp");
        // ports accept digits only, max 5
        m.click(Ctrl::Field("ts3_port"), 0.0, 0.0);
        m.text_input("1a2b3");
        assert_eq!(m.ts3.port, 123);
        // api keys stay free-form
        m.click(Ctrl::Field("ts6_key"), 0.0, 0.0);
        m.text_input("k-9!x");
        assert_eq!(m.ts6.api_key, "k-9!x");
    }

    #[test]
    fn source_select_switches_endpoint() {
        let mut m = model();
        m.click(Ctrl::Select("source"), 0.0, 0.0);
        assert_eq!(m.open_select, Some("source"));
        m.click(Ctrl::Option("source", 1), 0.0, 0.0);
        assert_eq!(m.source, "ts3");
        assert_eq!(m.open_select, None);
    }

    #[test]
    fn pending_actions_queue_for_the_host() {
        let mut m = model();
        m.click(Ctrl::CheckUpdate, 0.0, 0.0);
        assert_eq!(m.pending, Some(Pending::CheckUpdate));
        m.click(Ctrl::InstallPlugin, 0.0, 0.0);
        assert_eq!(m.pending, Some(Pending::InstallPlugin));
    }

    #[test]
    fn group_rows_cover_all_ids() {
        let m = model();
        assert_eq!(m.groups.len(), 17);
        assert_eq!(GROUP_KEYS.len(), 17);
    }

    #[test]
    fn version_compare_semver() {
        assert!(ver_newer("v1.5.1", "1.5.0"));
        assert!(ver_newer("1.4.13", "0.1.0"));
        assert!(!ver_newer("v1.4.13", "1.5.0"));
        assert!(!ver_newer("1.5.0", "1.5.0"));
        assert!(ver_newer("2.0", "1.9.9"));
    }

    #[test]
    fn ver_same_version_is_not_newer() {
        assert!(!ver_newer("v2.0.0", "2.0.0"));
        assert!(ver_newer("v2.0.1", "2.0.0"));
    }

    #[test]
    fn update_note_codes_translate() {
        assert_eq!(update_note_text("checking", "ru"), "Проверка обновлений…");
        assert_eq!(update_note_text("latest", "ru"), "У вас последняя версия ✓");
        assert!(update_note_text("downloaded v2.0.0", "ru").contains("v2.0.0"));
        assert!(update_note_text("available v2.0.0", "ru").contains("v2.0.0"));
        assert!(update_note_text("dlfail v2.0.0", "en").contains("v2.0.0"));
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
        let mut m = model();
        let fonts = Fonts::load();
        let icons = Icons::load();
        let (buf, regions, _) = render(&m, &fonts, &icons, 1.0);
        assert_eq!((buf.w, buf.h), (WIN_W as i32, WIN_H as i32));
        assert!(regions.iter().any(|r| r.ctrl == Ctrl::Close));
        assert!(regions.iter().any(|r| r.ctrl == Ctrl::MoveOverlay));
        // left nav: five pages
        for n in 0..5u8 {
            assert!(regions.iter().any(|r| r.ctrl == Ctrl::Nav(n)), "nav {n}");
        }
        // controls are spread across the pages — sum them up
        let (mut presets, mut sliders, mut selects, mut corners) = (0, 0, 0, 0);
        for page in 0..5u8 {
            m.page = page;
            let (_, regs, _) = render(&m, &fonts, &icons, 1.0);
            presets += regs.iter().filter(|r| matches!(r.ctrl, Ctrl::Preset(_))).count();
            sliders += regs.iter().filter(|r| matches!(r.ctrl, Ctrl::Slider(_))).count();
            selects += regs.iter().filter(|r| matches!(r.ctrl, Ctrl::Select(_))).count();
            corners += regs.iter().filter(|r| matches!(r.ctrl, Ctrl::Corner(_))).count();
        }
        assert_eq!(presets, 8);
        assert_eq!(sliders, 15);
        assert_eq!(selects, 9);
        assert_eq!(corners, 4);
    }

    #[test]
    fn groups_hidden_by_default() {
        let mut m = model();
        assert!(!m.groups_open, "«Группы сервера» скрыты изначально");
        m.click(Ctrl::ToggleGroups, 0.0, 0.0);
        assert!(m.groups_open);
    }

    #[test]
    fn select_dropdown_pick_and_close() {
        let mut m = model();
        m.click(Ctrl::Select("language"), 0.0, 0.0);
        assert_eq!(m.open_select, Some("language"));
        m.click(Ctrl::Option("language", 1), 0.0, 0.0);
        assert_eq!(m.app.language, "ru");
        assert_eq!(m.open_select, None);
        assert!(m.changed);
        m.changed = false;
        m.click(Ctrl::Select("eq_style"), 0.0, 0.0);
        assert_eq!(m.open_select, Some("eq_style"));
        m.click(Ctrl::Toggle("show_chat"), 0.0, 0.0);
        assert_eq!(m.open_select, None);
        m.click(Ctrl::Option("ev_scope_join", 1), 0.0, 0.0);
        assert_eq!(m.app.ev_scope_join, "own");
        m.click(Ctrl::Option("overlay_layer", 0), 0.0, 0.0);
        assert_eq!(m.desktop.overlay_layer, "panel");
    }

    #[test]
    fn corner_grid_requests_snap() {
        let mut m = model();
        let before = (m.app.margin_x, m.app.margin_y);
        m.click(Ctrl::Corner(2), 0.0, 0.0);
        assert_eq!(m.app.corner, "bl");
        // the overlay snaps to the corner with its LIVE panel size and writes
        // the real coordinates back — the model must not guess them here
        assert_eq!((m.app.margin_x, m.app.margin_y), before);
        m.click(Ctrl::Corner(1), 0.0, 0.0);
        assert_eq!(m.app.corner, "tr");
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
        let (buf, _, ch) = render(&m, &fonts, &icons, 1.0);
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/qa");
        std::fs::create_dir_all(dir).unwrap();
        write_png(&format!("{dir}/settings.png"), &buf).unwrap();
        // middle — corner grid + roster sections
        m.scroll = 620.0;
        let (bufm, _, _) = render(&m, &fonts, &icons, 1.0);
        write_png(&format!("{dir}/settings-middle.png"), &bufm).unwrap();
        // scrolled to the bottom — «Система» section
        m.wheel(-100000.0, ch);
        let (buf2, _, _) = render(&m, &fonts, &icons, 1.0);
        write_png(&format!("{dir}/settings-bottom.png"), &buf2).unwrap();
        // dropdown open («Язык»)
        m.click(Ctrl::Select("language"), 0.0, 0.0);
        let (buf3, _, _) = render(&m, &fonts, &icons, 1.0);
        write_png(&format!("{dir}/settings-select.png"), &buf3).unwrap();
    }
}
