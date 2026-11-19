//! Native overlay window: frame loop + move-mode drag + whisper linger.
//! Windows: layered topmost window (win32). Non-windows/tests: headless host.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::assets::Icons;
use crate::font::Fonts;
use crate::http::App;
use crate::layout::{class_key_for, render_frame, FeedItem, FrameInput, Hit, PANEL_W, SHADOW_PAD};
use crate::model::{ChatKind, OverlayState, UserState};
use crate::theme::Palette;

/* ---------------- placement math (pure, tested) ---------------- */

/// Panel top-left within the work area, from corner + margins (css px).
/// «Координата X/Y» — ABSOLUTE panel top-left inside the work area. The panel
/// fills downward from this fixed top edge; clamp_coords keeps it on screen.
pub fn panel_pos(work: (f64, f64, f64, f64), x: f64, y: f64) -> (f64, f64) {
    (work.0 + x, work.1 + y)
}

/// Inverse: work-area coordinates from a panel top-left position.
pub fn coords_from_pos(work: (f64, f64, f64, f64), px: f64, py: f64) -> (f64, f64) {
    (px - work.0, py - work.1)
}

/// Clamp so the panel always stays fully visible («оверлей всегда на экране»).
pub fn clamp_coords(work: (f64, f64, f64, f64), pw: f64, ph: f64, x: f64, y: f64) -> (f64, f64) {
    let max_x = (work.2 - pw).max(0.0);
    let max_y = (work.3 - ph).max(0.0);
    (x.clamp(0.0, max_x), y.clamp(0.0, max_y))
}

/// Corner preset («Положение» grid): snap coordinates for that corner using
/// the live panel size — the settings window cannot know it.
pub fn corner_coords(work: (f64, f64, f64, f64), corner: &str, pw: f64, ph: f64) -> (f64, f64) {
    // 9 anchors: "tl".."br" (row-major: t/m/b rows, l/c/r columns)
    let (ww, wh) = (work.2.max(pw), work.3.max(ph));
    let x = match corner.as_bytes().get(1).copied().unwrap_or(b'l') {
        b'l' => 0.0,
        b'r' => ww - pw,
        _ => (ww - pw) * 0.5,
    };
    let y = match corner.as_bytes().first().copied().unwrap_or(b't') {
        b't' => 0.0,
        b'b' => wh - ph,
        _ => (wh - ph) * 0.5,
    };
    (x.max(0.0), y.max(0.0))
}

/* ---------------- feed / whisper helpers (pure, tested) ---------------- */

fn event_action(ev: &str, detail: Option<&str>, lang: &str) -> String {
    let d = detail.unwrap_or("");
    let en = lang == "en";
    match ev {
        "join" => {
            if d.is_empty() {
                if en {
                    "joined the server".into()
                } else {
                    "зашёл(ла) на сервер".into()
                }
            } else if en {
                format!("joined channel {d}")
            } else {
                format!("зашёл(ла) в канал {d}")
            }
        }
        "leave" => {
            if d.is_empty() {
                if en {
                    "left the server".into()
                } else {
                    "вышел(ла) с сервера".into()
                }
            } else if en {
                format!("left channel {d}")
            } else {
                format!("вышел(ла) из канала {d}")
            }
        }
        "move" => {
            if en {
                format!("moved to channel {d}")
            } else {
                format!("перешёл(ла) в канал {d}")
            }
        }
        "group" => {
            let g = if d.is_empty() || d == "—" { "—".to_string() } else { d.to_string() };
            if en {
                format!("new group: {g}")
            } else {
                format!("новая группа: {g}")
            }
        }
        "poke" => {
            if en {
                "poked".into()
            } else {
                "покнул(а)".into()
            }
        }
        _ => d.to_string(),
    }
}

/// Build feed rows from state.chat, applying showEv*/scope filters (newest
/// last). Rows expire after `events_delay` / `chat_delay` seconds and fade out
/// over the last third of their life («панель событий пропадает и затухает»).
pub fn build_feed(
    state: &OverlayState,
    a: &crate::config::Appearance,
    lang: &str,
    now: u64,
) -> Vec<FeedItem> {
    let mut out: Vec<FeedItem> = Vec::new();
    for m in state.chat.iter().rev() {
        let lifetime = if m.event.is_some() {
            (a.events_delay.max(1.0) * 1000.0) as u64
        } else {
            (a.chat_delay.max(1.0) * 1000.0) as u64
        };
        let age = now.saturating_sub(m.at); // `at: 0` (unstamped) counts as expired
        if age >= lifetime {
            continue;
        }
        let fade_span = (lifetime / 3).max(400);
        let alpha = if age + fade_span >= lifetime {
            ((lifetime - age) as f32 / fade_span as f32).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if let Some(ev) = m.event {
            if !a.show_events {
                continue;
            }
            // showEvGroup/showEvSys live only in settings.js defaults (true) —
            // the server model drops them, so group/sys are always shown here.
            let (show, scope_own) = match ev {
                "join" => (a.show_ev_join, a.ev_scope_join == "own"),
                "leave" => (a.show_ev_leave, a.ev_scope_leave == "own"),
                "move" => (a.show_ev_move, a.ev_scope_move == "own"),
                "group" => (true, a.ev_scope_group == "own"),
                "poke" => (a.show_ev_poke, false),
                "chat" => (a.show_ev_chat, a.ev_scope_chat == "own"),
                _ => (true, a.ev_scope_sys == "own"),
            };
            if !show || (scope_own && m.own != Some(true)) {
                continue;
            }
            out.push(FeedItem {
                nickname: m.nickname.clone(),
                text: event_action(ev, m.detail.as_deref(), lang),
                kind: 0,
                alpha,
            });
        } else {
            if !a.show_chat {
                continue;
            }
            let is_private = matches!(m.kind, ChatKind::Private);
            if is_private && !a.chat_private {
                continue;
            }
            out.push(FeedItem {
                nickname: m.nickname.clone(),
                text: m.text.clone(),
                kind: if is_private { 1 } else { 2 },
                alpha,
            });
        }
        if out.len() >= a.events_count.max(1.0) as usize {
            break;
        }
    }
    out.reverse(); // oldest first → newest at the bottom of the block
    out
}

/// Whisper-section users with linger (v1.4.9 whisperDelay). Rows move to the
/// whisper block AND OUT of the roster while active (app.js containerFor), and
/// return to the roster when the linger expires.
pub fn whisper_now(u: &UserState, seen: &mut HashMap<String, u64>, now: u64, linger_ms: u64) -> bool {
    if u.whispering {
        seen.insert(u.id.clone(), now);
    }
    let last = seen.get(&u.id).copied().unwrap_or(0);
    u.whispering || (last > 0 && now.saturating_sub(last) < linger_ms)
}

/// Split sorted users into (roster, whisper-block) preserving order.
/// «Ограничение на показ одновременно до 10 человек на канале» — the cap
/// applies to the roster only; the whisper section stays complete.
pub(crate) const MAX_ROSTER: usize = 10;

pub(crate) fn cap_roster(mut rows: Vec<UserState>) -> Vec<UserState> {
    rows.truncate(MAX_ROSTER);
    rows
}

pub fn split_whisper(
    users: Vec<UserState>,
    seen: &mut HashMap<String, u64>,
    now: u64,
    linger_ms: u64,
) -> (Vec<UserState>, Vec<UserState>) {
    let mut roster = Vec::new();
    let mut whisper = Vec::new();
    for u in users {
        if whisper_now(&u, seen, now, linger_ms) {
            whisper.push(u);
        } else {
            roster.push(u);
        }
    }
    (roster, whisper)
}

/// Visible roster rows: showSelf/hideQuery/onlyTalking/talkingFirst (app.js rules).
pub fn roster_rows(users: &[UserState], a: &crate::config::Appearance) -> Vec<UserState> {
    let mut rows: Vec<UserState> = users
        .iter()
        .filter(|u| {
            if !a.show_self && u.is_self {
                return false;
            }
            if a.hide_query && u.is_query {
                return false;
            }
            if a.only_talking && !u.talking {
                return false;
            }
            true
        })
        .cloned()
        .collect();
    if a.talking_first {
        rows.sort_by(|x, y| y.talking.cmp(&x.talking));
    }
    rows
}

/* ---------------- host abstraction ---------------- */

pub enum Ev {
    Click(f32, f32),
    DragStart(f32, f32),
    DragMove(f32, f32),
    DragEnd,
    Wheel(f32),
    /// Keyboard text input for the settings window: printable chars, \x08
    /// backspace, pasted strings.
    Text(String),
}

pub trait Host {
    /// Present `buf` as a window at (win_x, win_y). `hit` is the SCREEN rect
    /// where wheel events belong to this window (the panel, not the whole
    /// monitor-sized window in screen/fullscreen layer mode).
    fn present(&mut self, buf: &crate::render::Buf, win_x: f64, win_y: f64, hit: (f64, f64, f64, f64));
    fn pump(&mut self) -> Vec<Ev>;
    fn set_interactive(&mut self, on: bool);
    /// Hide/show the overlay window («Показывать оверлей») without exiting.
    fn set_visible(&mut self, _on: bool) {}
    fn open_url(&self, url: &str);
    /// Bring the window to the foreground (settings window on open).
    fn activate(&mut self) {}
    /// Current top-left of the window in screen coords (survives user drags).
    fn window_pos(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
    /// Native move loop for a borderless window (caption-drag trick).
    fn begin_window_drag(&mut self) {}
    fn work_area(&self) -> (f64, f64, f64, f64);
    /// Full bounds of the target monitor ('screen'/'fullscreen' layer modes).
    fn screen_area(&self) -> (f64, f64, f64, f64);
}

/// Headless host — tests and non-windows builds.
pub struct HeadlessHost {
    pub last_w: i32,
    pub last_h: i32,
    pub interactive: bool,
    pub last_pos: (f64, f64),
}

impl HeadlessHost {
    pub fn new() -> HeadlessHost {
        HeadlessHost { last_w: 0, last_h: 0, interactive: false, last_pos: (0.0, 0.0) }
    }
}

impl Host for HeadlessHost {
    fn present(&mut self, buf: &crate::render::Buf, x: f64, y: f64, _hit: (f64, f64, f64, f64)) {
        self.last_w = buf.w;
        self.last_h = buf.h;
        self.last_pos = (x, y);
    }
    fn window_pos(&self) -> (f64, f64) {
        self.last_pos
    }
    fn pump(&mut self) -> Vec<Ev> {
        Vec::new()
    }
    fn set_interactive(&mut self, on: bool) {
        self.interactive = on;
    }
    fn open_url(&self, _url: &str) {}
    fn work_area(&self) -> (f64, f64, f64, f64) {
        (0.0, 0.0, 1920.0, 1080.0)
    }
    fn screen_area(&self) -> (f64, f64, f64, f64) {
        (0.0, 0.0, 1920.0, 1080.0)
    }
}

/* ---------------- window loop ---------------- */

pub struct Overlay {
    app: Arc<App>,
    pub port: u16,
    fonts: Fonts,
    icons: Icons,
    whisper_seen: HashMap<String, u64>,
    /// v1.4.12: outside whisperers (another channel) with their own linger.
    outside_whisper: HashMap<String, (UserState, u64)>,
    /// «Только говорящие»: stopped talkers linger here, fading out over
    /// `appearance.talking_fade` seconds before leaving the roster.
    talk_fade: HashMap<String, (UserState, u64)>,
    /// last tick a user was seen talking (highlight linger, «задержка исчезновения говорящих»)
    talk_seen: HashMap<String, u64>,
    /// roster scroll inside a height-capped panel (panelMaxHeight)
    roster_scroll: f32,
    pending: Option<(f64, f64)>,
    /// last applied «Положение» corner — a change snaps the panel there
    last_corner: Option<String>,
    saved_margins: Option<(f64, f64)>,
    dragging: bool,
    drag_off: (f64, f64),
    last_size: (f64, f64),
    /// Reusable fullscreen scratch buffer (screen/fullscreen layer) — the
    /// per-frame alloc of a monitor-sized buffer was a system-wide freeze.
    scratch: Option<crate::render::Buf>,
    /// Dirty tracking: skip render+present when nothing changed.
    last_key: Option<String>,
    last_present_ms: u64,
    /// Cached hit geometry for input between re-renders.
    last_hits: Vec<(crate::layout::Rect, crate::layout::Hit)>,
    last_geom: (f64, f64, f64, f64, f64), // px, py, pad, pw, ph
    last_origin: (f64, f64),
    /// Lock-free fallbacks: tick() must NEVER block (the message pump has to
    /// keep running or the system-wide mouse hook starves).
    last_cfg: crate::config::AppConfig,
    last_state: OverlayState,
}

impl Overlay {
    pub fn new(app: Arc<App>, port: u16) -> Overlay {
        let cfg0 = app.store.lock().unwrap().get().clone();
        let st0 = app.latest.read().unwrap().clone();
        Overlay {
            app,
            port,
            fonts: Fonts::load(),
            icons: Icons::load(),
            whisper_seen: HashMap::new(),
            outside_whisper: HashMap::new(),
            talk_fade: HashMap::new(),
            talk_seen: HashMap::new(),
            roster_scroll: 0.0,
            pending: None,
            last_corner: None,
            saved_margins: None,
            dragging: false,
            drag_off: (0.0, 0.0),
            last_size: (PANEL_W as f64, 200.0),
            scratch: None,
            last_key: None,
            last_present_ms: 0,
            last_hits: Vec::new(),
            last_geom: (0.0, 0.0, 0.0, PANEL_W as f64, 200.0),
            last_origin: (0.0, 0.0),
            last_cfg: cfg0,
            last_state: st0,
        }
    }



    /// One iteration: build the frame, present, process input.
    pub fn tick(&mut self, host: &mut dyn Host) {
        // Never block: a stalled lock holder must not stop the pump.
        let state = match self.app.latest.try_read() {
            Ok(g) => {
                self.last_state = g.clone();
                self.last_state.clone()
            }
            Err(_) => self.last_state.clone(),
        };
        let now_ms = crate::model::now_ms();
        let cfg = match self.app.store.try_lock() {
            Ok(g) => {
                self.last_cfg = g.get().clone();
                self.last_cfg.clone()
            }
            Err(_) => self.last_cfg.clone(),
        };
        // «Показывать оверлей» (tray toggle / Вид): hide the overlay window
        // without exiting the app
        host.set_visible(cfg.desktop.visible);
        if !cfg.desktop.visible {
            // Hidden: still pump Win32 messages — the tray window is created on
            // this thread and is driven by this pump; skipping it freezes the
            // tray («после скрытия в трее не дает раскрыть ничего»).
            let _ = host.pump();
            return;
        }
        let a = cfg.appearance.clone();
        let pal = Palette::from_appearance(&a);
        let lang = match a.language.as_str() {
            "en" => "en",
            "ru" => "ru",
            _ => "auto-or",
        };
        let lang = if lang == "auto-or" {
            // auto: follow UI language default ru (the app ships ru-first)
            "ru"
        } else {
            lang
        };

        // «Задержка исчезновения говорящих» — the talking highlight lingers and
        // fades out over `talking_fade` seconds after speech stops (every mode).
        let talk_fade_ms = (a.talking_fade.max(0.0) * 1000.0) as u64;
        for u in &state.users {
            if u.talking {
                self.talk_seen.insert(u.id.clone(), now_ms);
            }
        }
        if talk_fade_ms == 0 {
            self.talk_seen.clear();
        } else {
            self.talk_seen.retain(|_, t| now_ms.saturating_sub(*t) <= talk_fade_ms);
        }
        let mut all = roster_rows(&state.users, &a);
        // «Только говорящие»: recent talkers linger and fade out of the roster
        if a.only_talking && a.talking_fade > 0.0 {
            let fade_ms = (a.talking_fade * 1000.0) as u64;
            for u in &state.users {
                if u.talking {
                    self.talk_fade.insert(u.id.clone(), (u.clone(), now_ms));
                }
            }
            self.talk_fade.retain(|_id, (_u, t)| now_ms.saturating_sub(*t) < fade_ms);
            let mut extra: Vec<UserState> = Vec::new();
            for (id, (u, t)) in self.talk_fade.iter() {
                if !all.iter().any(|r| &r.id == id) {
                    let age = now_ms.saturating_sub(*t) as f32;
                    let mut u = u.clone();
                    u.talking = false;
                    u.fade = (1.0 - age / fade_ms as f32).clamp(0.0, 1.0);
                    extra.push(u);
                }
            }
            all.extend(extra);
        } else {
            self.talk_fade.clear();
        }
        let linger = (a.whisper_delay.max(0.0) * 1000.0) as u64;
        let (mut rows, mut whisperers) = if a.show_whispers {
            split_whisper(all, &mut self.whisper_seen, now_ms, linger)
        } else {
            (all, Vec::new())
        };
        // «до 10 человек на канале» — roster only; whisperers stay complete
        rows = cap_roster(rows);
        // per-row talking-highlight alpha: 1 while speaking, ramping to 0 over
        // `talking_fade` after speech stops, 0 = no highlight
        let talk_alphas: Vec<f32> = rows
            .iter()
            .map(|u| {
                if u.talking {
                    1.0
                } else if talk_fade_ms == 0 {
                    0.0
                } else {
                    match self.talk_seen.get(&u.id) {
                        Some(t) => {
                            let age = now_ms.saturating_sub(*t) as f32;
                            (1.0 - age / talk_fade_ms as f32).clamp(0.0, 1.0)
                        }
                        None => 0.0,
                    }
                }
            })
            .collect();
        if a.show_whispers {
            // v1.4.12: people whispering from ANOTHER channel never reach the
            // roster — merge them into the whisper section with the same linger.
            for u in state.whisperers.iter() {
                self.outside_whisper.insert(u.id.clone(), (u.clone(), now_ms));
            }
            self.outside_whisper
                .retain(|_, (_, at)| now_ms.saturating_sub(*at) <= linger.max(crate::model::WHISPER_GRACE_MS));
            let mut outsiders: Vec<UserState> = self.outside_whisper.values().map(|(u, _)| u.clone()).collect();
            outsiders.sort_by(|a, b| a.nickname.cmp(&b.nickname));
            for u in outsiders.iter_mut() {
                // sub-line: where the whisper comes from (web shows a tooltip)
                if let Some(ch) = u.whisper_channel.clone() {
                    if !ch.is_empty() {
                        u.away_message = if lang == "en" {
                            format!("Whisper from «{}»", ch)
                        } else {
                            format!("Шёпот из канала «{}»", ch)
                        };
                        u.away = true; // sub row styling
                    }
                }
            }
            whisperers.extend(outsiders);
        } else {
            self.outside_whisper.clear();
        }
        let groups: Vec<(i64, String)> = state
            .groups
            .iter()
            .map(|g| (g.id, g.name.clone()))
            .collect();
        let class_keys: Vec<Option<String>> = rows
            .iter()
            .map(|u| class_key_for(&u.server_groups, &groups, &cfg.classes.map))
            .collect();
        let whisper_keys: Vec<(String, Option<String>)> = whisperers
            .iter()
            .map(|u| (u.id.clone(), class_key_for(&u.server_groups, &groups, &cfg.classes.map)))
            .collect();
        let feed = build_feed(&state, &a, lang, now_ms);

        let move_mode = self.app.move_mode.load(Ordering::SeqCst);

        // ---- dirty check: an unchanged frame skips render + present ----
        let style_anim = a.style == "plasma" || a.style == "radiance";
        let anim_active =
            a.animations && (style_anim || rows.iter().any(|u| u.talking) || !whisperers.is_empty());
        let time_bucket = if anim_active {
            now_ms / 16
        } else if !feed.is_empty() {
            now_ms / 500 // feed lines expire over time
        } else {
            0
        };
        // margins live against the FULL screen («по всему размеру экрана»)
        let work = host.screen_area();
        let layer = cfg.desktop.overlay_layer.clone();
        let key = format!(
            "{}|{:?}|{:?}|{:?}|{:?}|{:.2}|{}|{:?}|{:?}|{}",
            cfg.to_json().to_string(),
            state,
            rows,
            whisperers,
            feed,
            self.roster_scroll,
            move_mode as u8,
            self.pending,
            (work.0, work.1, work.2, work.3),
            time_bucket
        );
        let interactive_wanted = move_mode || self.dragging || !cfg.desktop.click_through;
        let min_gap: u64 = 15; // up to ~60fps (dirty-check + 2s force keep idle cheap)
        // `|| now_ms < self.last_present_ms` — страховка от любых скачков
        // часов назад: кадр обязан перерисоваться, а не ждать force-таймера
        let due = now_ms.saturating_sub(self.last_present_ms) >= min_gap || now_ms < self.last_present_ms;
        let force = now_ms.saturating_sub(self.last_present_ms) >= 2000;
        let dirty = force || (due && self.last_key.as_deref() != Some(key.as_str()));

        if dirty {
        self.last_key = Some(key);
        self.last_present_ms = now_ms;
        let inp = FrameInput {
            appearance: &a,
            pal: &pal,
            fonts: &self.fonts,
            icons: &self.icons,
            status: state.status,
            server_name: &state.server_name,
            channel_name: &state.channel_name,
            user_count: rows.len() + whisperers.len(),
            users: rows,
            talk_alphas,
            class_keys,
            whisper_keys,
            whisperers,
            feed,
            lang,
            move_mode,
            now_ms,
            roster_scroll: self.roster_scroll,
            work_h: host.work_area().3 as f32,
        };
        let frame = render_frame(&inp);
        // auto-follow the first talking row inside a capped panel
        self.roster_scroll = frame.scroll_to;

        // placement: «Координата X/Y» are ABSOLUTE work-area coordinates
        let (mut cx, mut cy) = self.pending.unwrap_or((a.margin_x, a.margin_y));
        let pw = frame.panel_w as f64;
        let ph = frame.panel_h as f64;
        // «Положение» grid = one-click snap to that corner (live panel size),
        // then the real coordinates go back to the config so the sliders stay
        // truthful
        if self.last_corner.is_none() {
            self.last_corner = Some(a.corner.clone());
        } else if self.last_corner.as_deref() != Some(a.corner.as_str()) {
            self.last_corner = Some(a.corner.clone());
            let (sx, sy) = corner_coords(work, &a.corner, pw, ph);
            let patch = crate::json::obj(vec![(
                "appearance",
                crate::json::obj(vec![
                    ("marginX", crate::json::Json::n(sx)),
                    ("marginY", crate::json::Json::n(sy)),
                ]),
            )]);
            self.app.store.lock().unwrap().update(&patch);
            self.pending = Some((sx, sy));
            cx = sx;
            cy = sy;
        }
        let (cx, cy) = clamp_coords(work, pw, ph, cx, cy);
        let (px, py) = panel_pos(work, cx, cy);
        self.last_size = (pw, ph);
        // window rect includes the shadow margin
        let pad = (SHADOW_PAD * a.scale.clamp(0.5, 3.0) as f32) as f64;
        let origin = (px - pad, py - pad);
        // wheel/hit rect: the PANEL (frame buf incl. shadow pad), in screen px
        let hit = (origin.0, origin.1, frame.buf.w as f64, frame.buf.h as f64);
        if layer == "panel" {
            // v1.4.5 memory saver: the window hugs the panel
            host.present(&frame.buf, origin.0, origin.1, hit);
        } else {
            // 'screen'/'fullscreen': the window covers the WHOLE monitor, so
            // Windows keeps it above borderless/fullscreen games (v1.4.13).
            // The monitor-sized scratch buffer is REUSED across frames.
            let (ax, ay, aw, ah) = host.screen_area();
            let need = self
                .scratch
                .as_ref()
                .map_or(true, |b| b.w != aw as i32 || b.h != ah as i32);
            if need {
                self.scratch = Some(crate::render::Buf::new(aw as i32, ah as i32));
            }
            let big = self.scratch.as_mut().unwrap();
            big.clear();
            big.blit_buf((origin.0 - ax).round() as i32, (origin.1 - ay).round() as i32, &frame.buf);
            host.present(big, ax, ay, hit);
        }
        self.last_hits = frame.hits;
        self.last_geom = (px, py, pad, pw, ph);
        self.last_origin = origin;
        }

        // input (cached geometry — identical across skipped frames)
        let (px, py, pad, pw, ph) = self.last_geom;
        let origin = self.last_origin;
        let work = host.screen_area();
        let hits = self.last_hits.clone();
        let events = host.pump();
        host.set_interactive(interactive_wanted);
        for ev in events {
            match ev {
                Ev::Click(x, y) => {
                    let lx = x as f64 - origin.0;
                    let ly = y as f64 - origin.1;
                    for (rect, hit) in &hits {
                        if rect.contains(lx as f32, ly as f32) {
                            match hit {
                                Hit::Save => self.confirm_move(),
                                Hit::Discard => self.cancel_move(),
                                _ => {}
                            }
                        }
                    }
                }
                Ev::DragStart(x, y) if move_mode => {
                    self.dragging = true;
                    self.drag_off = (x as f64 - (px - pad), y as f64 - (py - pad));
                    if self.saved_margins.is_none() {
                        self.saved_margins = Some((a.margin_x, a.margin_y));
                    }
                }
                Ev::DragMove(x, y) if self.dragging => {
                    let npx = x as f64 - self.drag_off.0 + pad;
                    let npy = y as f64 - self.drag_off.1 + pad;
                    let _ = (px, py);
                    let (mmx, mmy) = coords_from_pos(work, npx, npy);
                    let (mmx, mmy) = clamp_coords(work, pw, ph, mmx, mmy);
                    self.pending = Some((mmx, mmy));
                }
                Ev::Wheel(dy) => {
                    self.roster_scroll = (self.roster_scroll - dy).max(0.0);
                }
                Ev::DragEnd => {
                    if self.dragging {
                        self.dragging = false;
                        if let Some((mmx, mmy)) = self.pending {
                            // move-pending for other views (web overlay parity)
                            self.app.broadcast(&crate::json::obj(vec![
                                ("type", crate::json::Json::s("move-pending")),
                                ("marginX", crate::json::Json::n(mmx)),
                                ("marginY", crate::json::Json::n(mmy)),
                            ]));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn confirm_move(&mut self) {
        if let Some((mx, my)) = self.pending.take() {
            let patch = crate::json::obj(vec![(
                "appearance",
                crate::json::obj(vec![
                    ("marginX", crate::json::Json::n(mx)),
                    ("marginY", crate::json::Json::n(my)),
                ]),
            )]);
            self.app.store.lock().unwrap().update(&patch);
            self.app.broadcast(&crate::json::obj(vec![
                ("type", crate::json::Json::s("config")),
                ("config", self.app.config_json()),
            ]));
            self.saved_margins = None;
        }
        self.app.move_mode.store(false, Ordering::SeqCst);
        self.app.broadcast(&crate::json::obj(vec![("type", crate::json::Json::s("move-confirm"))]));
    }

    fn cancel_move(&mut self) {
        if let Some(saved) = self.saved_margins.take() {
            self.pending = None;
            let patch = crate::json::obj(vec![(
                "appearance",
                crate::json::obj(vec![
                    ("marginX", crate::json::Json::n(saved.0)),
                    ("marginY", crate::json::Json::n(saved.1)),
                ]),
            )]);
            self.app.store.lock().unwrap().update(&patch);
        } else {
            self.pending = None;
        }
        self.app.move_mode.store(false, Ordering::SeqCst);
        self.app.broadcast(&crate::json::obj(vec![("type", crate::json::Json::s("move-cancel"))]));
    }
}

/// Spawn the overlay loop on its own thread (window on windows, headless elsewhere).
pub fn spawn(app: Arc<App>, port: u16) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("overlay".into())
        .spawn(move || {
            #[cfg(any(windows, tsn_win32))]
            win32::spawn_tray(app.clone(), port);
            let mut ov = Overlay::new(app, port);
            #[cfg(any(windows, tsn_win32))]
            {
                win32::run(&mut ov);
            }
            #[cfg(not(any(windows, tsn_win32)))]
            {
                let mut host = HeadlessHost::new();
                loop {
                    ov.tick(&mut host);
                    std::thread::sleep(Duration::from_millis(16));
                }
            }
        })
        .expect("overlay thread")
}

/* ---------------- win32 host (windows only) ---------------- */

/// win32 backend — always compiled so it type-checks on every platform;
/// the windows import libs are only linked when targeting windows.
/// Open a URL in the user's default browser (used for the web settings menu).
pub fn open_url_external(url: &str) {
    #[cfg(any(windows, tsn_win32))]
    win32::open_url_external(url);
    #[cfg(not(any(windows, tsn_win32)))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

pub(crate) mod win32 {
    use super::*;
    use crate::render::Buf;
    use std::ffi::c_void;

    #[allow(non_snake_case)]
    type WndProc = unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct WNDCLASSEXW {
        cbSize: u32,
        style: u32,
        lpfnWndProc: WndProc,
        cbClsExtra: i32,
        cbWndExtra: i32,
        hInstance: *mut c_void,
        hIcon: *mut c_void,
        hCursor: *mut c_void,
        hbrBackground: *mut c_void,
        lpszMenuName: *const u16,
        lpszClassName: *const u16,
        hIconSm: *mut c_void,
    }

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct BLENDFUNCTION {
        BlendOp: u8,
        BlendFlags: u8,
        SourceConstantAlpha: u8,
        AlphaFormat: u8,
    }

    #[repr(C)]
    struct BITMAPINFOHEADER {
        biSize: u32,
        biWidth: i32,
        biHeight: i32,
        biPlanes: u16,
        biBitCount: u16,
        biCompression: u32,
        biSizeImage: u32,
        biXPelsPerMeter: i32,
        biYPelsPerMeter: i32,
        biClrUsed: u32,
        biClrImportant: u32,
    }

    #[repr(C)]
    struct BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER,
        bmiColors: [u32; 3],
    }

    #[repr(C)]
    struct MSG {
        hwnd: *mut c_void,
        message: u32,
        wParam: usize,
        lParam: isize,
        time: u32,
        pt: POINT,
        lPrivate: u32,
    }

    #[cfg_attr(windows, link(name = "user32"))]
    extern "system" {
        fn RegisterClassExW(c: *const WNDCLASSEXW) -> u16;
        fn CreateWindowExW(
            ex: u32,
            cls: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            inst: *mut c_void,
            param: *mut c_void,
        ) -> *mut c_void;
        fn DefWindowProcW(w: *mut c_void, m: u32, wp: usize, lp: isize) -> isize;
        fn DestroyWindow(w: *mut c_void) -> i32;
        fn PeekMessageW(m: *mut MSG, w: *mut c_void, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(m: *const MSG) -> i32;
        fn DispatchMessageW(m: *const MSG) -> isize;
        fn PostQuitMessage(c: i32);
        fn LoadCursorW(inst: *mut c_void, name: *const u16) -> *mut c_void;
        fn SetCapture(w: *mut c_void) -> *mut c_void;
        fn ReleaseCapture() -> i32;
        fn GetCursorPos(p: *mut POINT) -> i32;
        fn ScreenToClient(w: *mut c_void, p: *mut POINT) -> i32;
        fn SetForegroundWindow(w: *mut c_void) -> i32;
        fn GetWindowRect(w: *mut c_void, r: *mut RECT) -> i32;
        fn GetForegroundWindow() -> *mut c_void;
        fn ShowWindow(w: *mut c_void, cmd: i32) -> i32;
        fn GetWindowThreadProcessId(w: *mut c_void, pid: *mut u32) -> u32;
        fn SendMessageW(w: *mut c_void, msg: u32, wp: usize, lp: isize) -> isize;
        fn SetWindowTextW(w: *mut c_void, text: *const u16) -> i32;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
        fn SetWindowPos(
            w: *mut c_void,
            after: *mut c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        fn GetWindowLongW(w: *mut c_void, idx: i32) -> i32;
        fn SetWindowLongW(w: *mut c_void, idx: i32, v: i32) -> i32;
        fn UpdateLayeredWindow(
            w: *mut c_void,
            dst: *mut c_void,
            ppt: *const POINT,
            psz: *const POINT,
            dc: *mut c_void,
            pptsrc: *const POINT,
            key: u32,
            blend: *const BLENDFUNCTION,
            flags: u32,
        ) -> i32;
        fn GetDC(w: *mut c_void) -> *mut c_void;
        fn ReleaseDC(w: *mut c_void, dc: *mut c_void) -> i32;
        fn SystemParametersInfoW(a: u32, u: u32, p: *mut c_void, w: u32) -> i32;
        fn ClientToScreen(w: *mut c_void, p: *mut POINT) -> i32;
        fn MonitorFromWindow(w: *mut c_void, flags: u32) -> *mut c_void;
        fn SetWindowsHookExW(id: i32, proc: *mut c_void, inst: *mut c_void, tid: u32) -> *mut c_void;
        fn CallNextHookEx(h: *mut c_void, code: i32, w: usize, l: isize) -> isize;
        fn UnhookWindowsHookEx(h: *mut c_void) -> i32;
        fn GetMonitorInfoW(m: *mut c_void, info: *mut MONITORINFO) -> i32;
        fn GetMessageW(m: *mut MSG, w: *mut c_void, min: u32, max: u32) -> i32;
        fn LoadIconW(inst: *mut c_void, name: *const u16) -> *mut c_void;
        fn LoadImageW(inst: *mut c_void, name: *const u16, ty: u32, cx: i32, cy: i32, flags: u32) -> *mut c_void;
        fn CreateIconIndirect(info: *const ICONINFO) -> *mut c_void;
        fn SetTimer(w: *mut c_void, id: usize, ms: u32, proc: *mut c_void) -> usize;
        fn CreatePopupMenu() -> *mut c_void;
        fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, label: *const u16) -> i32;
        fn TrackPopupMenu(menu: *mut c_void, flags: u32, x: i32, y: i32, res: i32, w: *mut c_void, rect: *const c_void) -> u32;
        fn DestroyMenu(menu: *mut c_void) -> i32;
    }

    const HWND_MESSAGE: *mut c_void = -3isize as *mut c_void;
    const MONITOR_DEFAULTTOPRIMARY: u32 = 2;
    const WH_MOUSE_LL: i32 = 14;
    const WM_MOUSEWHEEL: u32 = 0x020A;

    #[repr(C)]
    struct MONITORINFO {
        cb_size: u32,
        rc_monitor: RECT,
        rc_work: RECT,
        dw_flags: u32,
    }

    #[cfg_attr(windows, link(name = "kernel32"))]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CreateMutexW(attr: *mut c_void, owner: i32, name: *const u16) -> *mut c_void;
        fn GetLastError() -> u32;
        fn CloseHandle(h: *mut c_void) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetModuleFileNameW(h: *mut c_void, buf: *mut u16, size: u32) -> u32;
    }

    #[cfg_attr(windows, link(name = "advapi32"))]
    extern "system" {
        fn OpenProcessToken(proc: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn RegOpenKeyExW(key: *mut c_void, sub: *const u16, opts: u32, sam: u32, out: *mut *mut c_void) -> i32;
        fn RegSetValueExW(key: *mut c_void, name: *const u16, res: u32, ty: u32, data: *const u8, cb: u32) -> i32;
        fn RegDeleteValueW(key: *mut c_void, name: *const u16) -> i32;
        fn RegCloseKey(key: *mut c_void) -> i32;
        fn GetTokenInformation(token: *mut c_void, class: u32, info: *mut c_void, len: u32, out: *mut u32) -> i32;
    }

    #[cfg_attr(windows, link(name = "user32"))]
    extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(fmt: u32, mem: *mut c_void) -> *mut c_void;
        fn GetClipboardData(fmt: u32) -> *mut c_void;
        fn CloseClipboard() -> i32;
    }

    #[cfg_attr(windows, link(name = "kernel32"))]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(mem: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(mem: *mut c_void) -> i32;
    }

    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_ELEVATION_CLASS: u32 = 20;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const SW_HIDE: i32 = 0;
    const SW_SHOWNA: i32 = 8;
    const KEY_SET_VALUE: u32 = 0x0002;
    const HKEY_CURRENT_USER: usize = 0x8000_0001;
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const NIF_INFO: u32 = 0x10;
    const NIIF_INFO: u32 = 0x1;

    /// True when the process already runs with administrator rights.
    pub fn is_elevated() -> bool {
        unsafe {
            let mut token: *mut c_void = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elev: u32 = 0;
            let mut out: u32 = 0;
            let ok = GetTokenInformation(
                token,
                TOKEN_ELEVATION_CLASS,
                &mut elev as *mut u32 as *mut c_void,
                4,
                &mut out,
            );
            CloseHandle(token);
            ok != 0 && elev != 0
        }
    }

    /// Relaunch the current executable with the UAC "runas" verb.
    pub fn relaunch_elevated() -> bool {
        unsafe {
            let mut buf = [0u16; 32768];
            let n = GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), buf.len() as u32);
            if n == 0 {
                return false;
            }
            let exe: Vec<u16> = buf[..n as usize]
                .iter()
                .copied()
                .chain(std::iter::once(0))
                .collect();
            let args: Vec<u16> = "--elevated-relaunch\0".encode_utf16().collect();
            let ret = ShellExecuteW(null_mut(), wide("runas").as_ptr(), exe.as_ptr(), args.as_ptr(), null(), 1);
            ret as isize > 32
        }
    }

    /// Named-mutex single-instance guard. True = this process runs the app.
    /// The fresh handle is intentionally left open — Windows closes it at
    /// process exit and the mutex then dies with the process.
    pub fn acquire_single_instance() -> bool {
        unsafe {
            let name: Vec<u16> = "TSN_Notifier_SingleInstance_v1\0".encode_utf16().collect();
            for attempt in 0..3 {
                let h = CreateMutexW(null_mut(), 0, name.as_ptr());
                if h.is_null() {
                    return true; // cannot check — run anyway
                }
                if GetLastError() != 183 {
                    return true; // created fresh
                }
                // Already exists: could be a relaunching parent that is about
                // to exit (elevation handover) — retry briefly, then give up.
                crate::overlay::win32::close_handle_retry(h);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(600));
                }
            }
            false
        }
    }

    fn close_handle_retry(h: *mut c_void) {
        unsafe {
            CloseHandle(h);
        }
    }

    /// Honor `desktop.elevate` (default ON): relaunch with admin rights unless
    /// already elevated. A declined UAC prompt continues un-elevated («UAC ok»).
    pub fn maybe_elevate(cfg: &crate::config::AppConfig) {
        if !cfg.desktop.elevate || is_elevated() {
            return;
        }
        if relaunch_elevated() {
            std::process::exit(0);
        }
    }

    /// HKCU Run entry — «Запуск с системой» (Launch at startup).
    pub fn set_autostart(on: bool) -> bool {
        unsafe {
            let mut key: *mut c_void = null_mut();
            let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
            if RegOpenKeyExW(
                HKEY_CURRENT_USER as *mut c_void,
                wide(path).as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            ) != 0
            {
                return false;
            }
            let name = wide("TS3-TS6 Notifier");
            let ok = if on {
                let mut exe = [0u16; 320];
                let n = GetModuleFileNameW(null_mut(), exe.as_mut_ptr(), 320) as usize;
                let exe = String::from_utf16_lossy(&exe[..n.min(320)]);
                let data: Vec<u16> = format!("\"{}\"", exe)
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                RegSetValueExW(key, name.as_ptr(), 0, 1, data.as_ptr() as *const u8, data.len() as u32 * 2)
            } else {
                RegDeleteValueW(key, name.as_ptr())
            };
            RegCloseKey(key);
            ok == 0
        }
    }

    /// True when the window covers (almost) its entire monitor — fullscreen and
    /// borderless-fullscreen («без рамки») games included.
    pub fn window_is_fullscreen(hwnd: *mut c_void) -> bool {
        unsafe {
            if hwnd.is_null() {
                return false;
            }
            let mut wr: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut wr) == 0 {
                return false;
            }
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cb_size = std::mem::size_of::<MONITORINFO>() as u32;
            if mon.is_null() || GetMonitorInfoW(mon, &mut mi) == 0 {
                return false;
            }
            (wr.right - wr.left) >= (mi.rc_monitor.right - mi.rc_monitor.left) - 8
                && (wr.bottom - wr.top) >= (mi.rc_monitor.bottom - mi.rc_monitor.top) - 8
        }
    }

    /// True when the process owning `hwnd` runs with administrator rights.
    pub fn process_is_elevated(hwnd: *mut c_void) -> bool {
        unsafe {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 {
                return false;
            }
            let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if proc.is_null() {
                // cannot query (protected process) — assume elevated
                return true;
            }
            let mut token: *mut c_void = null_mut();
            let ok = OpenProcessToken(proc, TOKEN_QUERY, &mut token);
            CloseHandle(proc);
            if ok == 0 {
                return true;
            }
            let mut elev: u32 = 0;
            let mut out: u32 = 0;
            let q = GetTokenInformation(
                token,
                TOKEN_ELEVATION_CLASS,
                &mut elev as *mut u32 as *mut c_void,
                4,
                &mut out,
            );
            CloseHandle(token);
            q != 0 && elev != 0
        }
    }

    #[cfg_attr(windows, link(name = "shell32"))]
    extern "system" {
        fn Shell_NotifyIconW(dw: usize, data: *mut NOTIFYICONDATAW) -> i32;
        fn ShellExecuteW(
            hwnd: *mut c_void,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> *mut c_void;
    }

    #[cfg_attr(windows, link(name = "gdi32"))]
    extern "system" {
        fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
        fn DeleteDC(dc: *mut c_void) -> i32;
        fn CreateDIBSection(
            dc: *mut c_void,
            bmi: *const BITMAPINFO,
            usage: u32,
            bits: *mut *mut c_void,
            section: *mut c_void,
            offset: u32,
        ) -> *mut c_void;
        fn SelectObject(dc: *mut c_void, obj: *mut c_void) -> *mut c_void;
        fn DeleteObject(obj: *mut c_void) -> i32;
    }

    // window messages / styles
    const WM_DESTROY: u32 = 0x0002;
    const WM_CHAR: u32 = 0x0102;
    const WM_PASTE: u32 = 0x0302;
    const CF_UNICODETEXT: u32 = 13;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_CLOSE: u32 = 0x0010;
    const WM_MOUSEMOVE: u32 = 0x0200;
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const PM_REMOVE: u32 = 0x0001;
    const WS_EX_LAYERED: i32 = 0x0008_0000;
    const WS_EX_TRANSPARENT: i32 = 0x0000_0020;
    const WS_EX_TOOLWINDOW: i32 = 0x0000_0080;
    const WS_EX_TOPMOST: i32 = 0x0000_0008;
    const WS_POPUP: u32 = 0x8000_0000;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const HWND_TOPMOST: isize = -1;
    const GWL_EXSTYLE: i32 = -20;
    const ULW_ALPHA: u32 = 0x0000_0002;
    const AC_SRC_OVER: u8 = 0;
    const AC_SRC_ALPHA: u8 = 1;
    const DIB_RGB_COLORS: u32 = 0;
    const SPI_GETWORKAREA: u32 = 0x0030;
    const IDC_ARROW: usize = 32512;
    const WS_EX_APPWINDOW: i32 = 0x0004_0000;
    const HWND_NOTOPMOST: isize = -2;
    const WM_ACTIVATE: u32 = 0x0006;
    const WM_NCLBUTTONDOWN: u32 = 0x00A1;
    const HTCAPTION: usize = 2;

    static SETTINGS_HWND: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn settings_hwnd_is(hwnd: *mut c_void) -> bool {
        hwnd as usize == SETTINGS_HWND.load(std::sync::atomic::Ordering::SeqCst) && !hwnd.is_null()
    }

    /// Global wheel queue (WH_MOUSE_LL): (screen point, wheel delta, timestamp ms).
    static WHEELS: std::sync::Mutex<Vec<(POINT, f32, u64)>> = std::sync::Mutex::new(Vec::new());
    /// Keyboard input queued by the settings window proc (WM_CHAR / paste).
    static KEYS: std::sync::Mutex<Vec<Ev>> = std::sync::Mutex::new(Vec::new());
    const WHEEL_TTL_MS: u64 = 1500;

    fn wheel_now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    unsafe extern "system" fn mouse_ll_proc(code: i32, w: usize, l: isize) -> isize {
        if code >= 0 && w as u32 == WM_MOUSEWHEEL {
            let info = &*(l as *const MSLLHOOKSTRUCT);
            let delta = ((info.mouse_data >> 16) & 0xffff) as u16 as i16 as f32;
            let now = wheel_now_ms();
            // MSLLHOOKSTRUCT.pt is always PHYSICAL screen px; each host
            // compares in its own coordinate space (see WinHost::pump)
            let px = info.pt.x;
            let py = info.pt.y;
            let mut q = WHEELS.lock().unwrap();
            // prune stale entries, then push — fresh events are NEVER dropped
            q.retain(|(_, _, ts)| now.saturating_sub(*ts) < WHEEL_TTL_MS);
            if q.len() < 64 {
                q.push((POINT { x: px, y: py }, delta / 120.0 * 40.0, now));
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, w, l)
    }

    #[repr(C)]
    struct MSLLHOOKSTRUCT {
        pt: POINT,
        mouse_data: u32,
        flags: u32,
        time: u32,
        dw_extra_info: usize,
    }

    struct WinHost {
        hwnd: *mut c_void,
        last_rect: (f64, f64, f64, f64),
        topmost_at: Option<std::time::Instant>,
        /// last foreground window («поверх всех окон»: re-assert on focus change)
        last_fg: *mut c_void,
        /// the foreground window covers its monitor — a game owns the screen
        fg_full: bool,
        /// «Показывать оверлей» — window show state
        visible_shown: bool,
        mem_dc: *mut c_void,
        dib: *mut c_void,
        dib_bits: *mut u8,
        w: i32,
        h: i32,
        interactive: bool,
        drag_pending: bool,
        settings_host: bool,
        shown: bool,
    }

    unsafe extern "system" fn wnd_proc(hwnd: *mut c_void, msg: u32, wp: usize, lp: isize) -> isize {
        match msg {
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_ACTIVATE => {
                // the settings window behaves like a normal app window: it is
                // topmost only while it has focus (so it stays usable over the
                // overlay) and drops behind other windows when deactivated
                let is_settings = settings_hwnd_is(hwnd);
                if is_settings {
                    let active = (wp & 0xffff) as u16 != 0;
                    let after = if active { HWND_TOPMOST } else { HWND_NOTOPMOST };
                    SetWindowPos(
                        hwnd,
                        after as *mut c_void,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CLOSE => {
                // the settings window «крестик»/Alt+F4: let settings.rs finish
                // its loop (it destroys the window on the way out)
                if settings_hwnd_is(hwnd) {
                    crate::settings::request_close();
                    0
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_PASTE => {
                if settings_hwnd_is(hwnd) {
                    let text = get_clipboard_text();
                    if !text.is_empty() {
                        KEYS.lock().unwrap().push(Ev::Text(text));
                    }
                    0
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_CHAR => {
                if settings_hwnd_is(hwnd) {
                    let ch = char::from_u32(wp as u32);
                    match ch {
                        Some('\x08') => KEYS.lock().unwrap().push(Ev::Text("\x08".into())),
                        Some(c) if (c as u32) >= 0x20 && c != '\x7f' => {
                            KEYS.lock().unwrap().push(Ev::Text(c.to_string()))
                        }
                        _ => {}
                    }
                    0
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    impl WinHost {
        unsafe fn new() -> WinHost {
            let cls: Vec<u16> = "TSNOverlay\0".encode_utf16().collect();
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: wnd_proc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: std::ptr::null_mut(),
                hIcon: app_icon(32, 32),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW as *const u16),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: cls.as_ptr(),
                hIconSm: app_icon(16, 16),
            };
            RegisterClassExW(&wc);
            let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_TRANSPARENT;
            let title: Vec<u16> = "TS3-TS6 Notifier Overlay\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                ex as u32,
                cls.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                100,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            WinHost {
                hwnd,
                last_rect: (0.0, 0.0, 0.0, 0.0),
                topmost_at: None,
                last_fg: null_mut(),
                fg_full: false,
                visible_shown: true,
                mem_dc: std::ptr::null_mut(),
                dib: std::ptr::null_mut(),
                dib_bits: std::ptr::null_mut(),
                w: 0,
                h: 0,
                interactive: false,
                drag_pending: false,
                settings_host: false,
                shown: false,
            }
        }

        unsafe fn ensure_surface(&mut self, w: i32, h: i32) {
            if self.dib_bits.is_null() || self.w != w || self.h != h {
                if !self.dib.is_null() {
                    DeleteObject(self.dib);
                    DeleteDC(self.mem_dc);
                }
                let screen_dc = GetDC(std::ptr::null_mut());
                self.mem_dc = CreateCompatibleDC(screen_dc);
                ReleaseDC(std::ptr::null_mut(), screen_dc);
                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w,
                        biHeight: -h, // top-down
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: 0,
                        biSizeImage: (w * h * 4) as u32,
                        biXPelsPerMeter: 0,
                        biYPelsPerMeter: 0,
                        biClrUsed: 0,
                        biClrImportant: 0,
                    },
                    bmiColors: [0; 3],
                };
                let mut bits: *mut c_void = std::ptr::null_mut();
                self.dib = CreateDIBSection(
                    self.mem_dc,
                    &bmi,
                    DIB_RGB_COLORS,
                    &mut bits,
                    std::ptr::null_mut(),
                    0,
                );
                self.dib_bits = bits as *mut u8;
                SelectObject(self.mem_dc, self.dib);
                self.w = w;
                self.h = h;
            }
        }
    }

    impl Drop for WinHost {
        fn drop(&mut self) {
            unsafe {
                if !self.dib.is_null() {
                    DeleteObject(self.dib);
                    DeleteDC(self.mem_dc);
                }
                if !self.hwnd.is_null() {
                    DestroyWindow(self.hwnd);
                }
            }
        }
    }

    impl Host for WinHost {
        fn present(&mut self, buf: &Buf, win_x: f64, win_y: f64, hit: (f64, f64, f64, f64)) {
            self.last_rect = hit;
            unsafe {
                let old_dim = (self.w, self.h);
                self.ensure_surface(buf.w, buf.h);
                let resized = old_dim != (self.w, self.h);
                // RGBA -> BGRA (chunked slice copy — auto-vectorizable)
                let n = (buf.w * buf.h * 4) as usize;
                let dst = std::slice::from_raw_parts_mut(self.dib_bits, n);
                for (d, s) in dst.chunks_exact_mut(4).zip(buf.px.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = s[3];
                }
                let pt_dst = POINT { x: win_x as i32, y: win_y as i32 };
                let pt_src = POINT { x: 0, y: 0 };
                let size = POINT { x: buf.w, y: buf.h };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA,
                };
                let dc = GetDC(self.hwnd);
                UpdateLayeredWindow(
                    self.hwnd,
                    dc,
                    &pt_dst,
                    &size,
                    self.mem_dc,
                    &pt_src,
                    0,
                    &blend,
                    ULW_ALPHA,
                );
                ReleaseDC(self.hwnd, dc);
                // «Повесить оверлей над всеми окнами» — games re-assert their
                // own z-order when they take focus: watch the foreground window
                // and push TOPMOST the moment it changes; a fullscreen/borderless
                // («без рамки») game gets a faster re-assert loop. Not every
                // frame: a per-frame SetWindowPos on a fullscreen layered window
                // stalls the compositor.
                if !self.settings_host {
                    let fg = GetForegroundWindow();
                    if fg != self.last_fg {
                        self.last_fg = fg;
                        let ours = fg.is_null() || fg == self.hwnd || settings_hwnd_is(fg);
                        self.fg_full = !ours && window_is_fullscreen(fg);
                        if !ours {
                            // something else just took focus — re-assert at once
                            self.topmost_at = None;
                        }
                        // an ADMIN-run fullscreen window: only an elevated overlay
                        // can stay above it (UIPI) — tell the user once
                        if self.fg_full && !is_elevated() && process_is_elevated(fg) {
                            maybe_admin_hint();
                        }
                    }
                }
                let interval: u128 = if self.fg_full { 120 } else { 500 };
                let now = std::time::Instant::now();
                let due = self
                    .topmost_at
                    .map_or(true, |t| now.duration_since(t).as_millis() >= interval);
                if due || resized {
                    self.topmost_at = Some(now);
                    if self.settings_host {
                        // shown once; z-order then follows WM_ACTIVATE (window
                        // can go to the background like a normal app window)
                        if !self.shown {
                            self.shown = true;
                            SetWindowPos(
                                self.hwnd,
                                HWND_NOTOPMOST as *mut c_void,
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                            );
                        }
                    } else if !crate::settings::is_open() {
                        // while the settings window is open the OVERLAY leaves the
                        // top of the topmost band alone (it would slide over it)
                        SetWindowPos(
                            self.hwnd,
                            HWND_TOPMOST as *mut c_void,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        );
                    }
                }
            }
        }

        fn pump(&mut self) -> Vec<Ev> {
            let mut out = Vec::new();
            out.append(&mut KEYS.lock().unwrap());
            {
                // wheel captured by the LL hook — only events over OUR window;
                // stale entries expire so a busy desktop never starves fresh events
                let mut kept: Vec<(POINT, f32, u64)> = Vec::new();
                let (rx, ry, rw, rh) = self.last_rect;
                let now = wheel_now_ms();
                let ds = dpi_scale();
                for (pt, delta, ts) in std::mem::take(&mut *WHEELS.lock().unwrap()) {
                    if now.saturating_sub(ts) >= WHEEL_TTL_MS {
                        continue;
                    }
                    // the settings host's rect is PHYSICAL (DPI-aware thread);
                    // the overlay's rects are LOGICAL — compare accordingly
                    let (cx, cy) = if self.settings_host {
                        (pt.x as f64, pt.y as f64)
                    } else {
                        (pt.x as f64 / ds, pt.y as f64 / ds)
                    };
                    let mut inside = cx >= rx
                        && cx <= rx + rw
                        && cy >= ry
                        && cy <= ry + rh;
                    // the settings window wins the wheel wherever the two overlap
                    if inside
                        && !self.settings_host
                        && crate::settings::is_open()
                        && crate::settings::rect_contains(pt.x as f64, pt.y as f64)
                    {
                        inside = false;
                    }
                    if inside {
                        out.push(Ev::Wheel(delta));
                    } else {
                        kept.push((pt, delta, ts));
                    }
                }
                *WHEELS.lock().unwrap() = kept;
            }
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    let mut to_screen = |lp: isize| -> (f32, f32) {
                        // lParam halves are SIGNED 16-bit — outside the window
                        // they go negative; sign-extend or drags jump («прыгают»)
                        let mut p = POINT {
                            x: ((lp & 0xffff) as u16) as i16 as i32,
                            y: (((lp >> 16) & 0xffff) as u16) as i16 as i32,
                        };
                        ClientToScreen(self.hwnd, &mut p);
                        (p.x as f32, p.y as f32)
                    };
                    if msg.message == WM_LBUTTONDOWN {
                        let (x, y) = to_screen(msg.lParam);
                        self.drag_pending = true;
                        out.push(Ev::DragStart(x, y));
                        SetCapture(self.hwnd);
                    } else if msg.message == WM_MOUSEMOVE && self.drag_pending {
                        let (x, y) = to_screen(msg.lParam);
                        out.push(Ev::DragMove(x, y));
                    } else if msg.message == WM_LBUTTONUP {
                        let (x, y) = to_screen(msg.lParam);
                        if self.drag_pending {
                            self.drag_pending = false;
                            ReleaseCapture();
                            out.push(Ev::DragEnd);
                        }
                        out.push(Ev::Click(x, y));
                    } else {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            out
        }

        fn set_visible(&mut self, on: bool) {
            if self.visible_shown != on {
                self.visible_shown = on;
                unsafe {
                    ShowWindow(self.hwnd, if on { SW_SHOWNA } else { SW_HIDE });
                }
            }
        }
        fn set_interactive(&mut self, on: bool) {
            if self.interactive == on {
                return;
            }
            self.interactive = on;
            unsafe {
                let cur = GetWindowLongW(self.hwnd, GWL_EXSTYLE);
                let next = if on {
                    cur & !WS_EX_TRANSPARENT
                } else {
                    cur | WS_EX_TRANSPARENT
                };
                SetWindowLongW(self.hwnd, GWL_EXSTYLE, next);
            }
        }

        fn activate(&mut self) {
            unsafe {
                SetForegroundWindow(self.hwnd);
            }
        }

        fn window_pos(&self) -> (f64, f64) {
            unsafe {
                let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                if GetWindowRect(self.hwnd, &mut r) != 0 {
                    (r.left as f64, r.top as f64)
                } else {
                    (0.0, 0.0)
                }
            }
        }

        fn begin_window_drag(&mut self) {
            unsafe {
                // borderless caption drag: DefWindowProc runs the modal move loop
                ReleaseCapture();
                SendMessageW(self.hwnd, WM_NCLBUTTONDOWN, HTCAPTION, 0);
            }
        }

        fn open_url(&self, url: &str) {
            open_url_external(url);
        }

        fn screen_area(&self) -> (f64, f64, f64, f64) {
            unsafe {
                let mon = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTOPRIMARY);
                let mut mi = MONITORINFO {
                    cb_size: std::mem::size_of::<MONITORINFO>() as u32,
                    rc_monitor: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                    rc_work: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                    dw_flags: 0,
                };
                if !mon.is_null() && GetMonitorInfoW(mon, &mut mi) != 0 {
                    let r = mi.rc_monitor;
                    return (r.left as f64, r.top as f64, (r.right - r.left) as f64, (r.bottom - r.top) as f64);
                }
                self.work_area()
            }
        }

        fn work_area(&self) -> (f64, f64, f64, f64) {
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            unsafe {
                SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut RECT as *mut c_void, 0);
            }
            (
                r.left as f64,
                r.top as f64,
                (r.right - r.left) as f64,
                (r.bottom - r.top) as f64,
            )
        }
    }

    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOSIZE: u32 = 0x0001;

    /* ---------------- system tray ---------------- */

    const WM_APP_TRAY: u32 = 0x8001;
    const NIM_ADD: usize = 0;
    const NIM_MODIFY: usize = 1;
    const NIM_DELETE: usize = 2;
    const NIF_MESSAGE: u32 = 0x1;
    const NIF_ICON: u32 = 0x2;
    const NIF_TIP: u32 = 0x4;
    const WM_COMMAND: u32 = 0x0111;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_TIMER: u32 = 0x0113;
    const TPM_RETURNCMD: u32 = 0x0100;
    const MF_STRING: u32 = 0x0;
    const MF_CHECKED: u32 = 0x8;
    const MF_SEPARATOR: u32 = 0x800;
    const MF_POPUP: u32 = 0x10;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NOTIFYICONDATAW {
        cb_size: u32,
        hwnd: *mut c_void,
        u_id: u32,
        u_flags: u32,
        u_callback: u32,
        h_icon: *mut c_void,
        sz_tip: [u16; 128],
        dw_state: u32,
        dw_state_mask: u32,
        sz_info: [u16; 256],
        u_timeout_version: u32,
        sz_info_title: [u16; 64],
        dw_info_flags: u32,
        guid_item: [u8; 16],
        h_balloon_icon: *mut c_void,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ICONINFO {
        f_icon: u32,
        x_hotspot: u32,
        y_hotspot: u32,
        hbm_mask: *mut c_void,
        hbm_color: *mut c_void,
    }

    use std::ptr::{null, null_mut};

    static TRAY: std::sync::Mutex<Option<std::sync::Arc<crate::http::App>>> = std::sync::Mutex::new(None);
    static TRAY_PORT: std::sync::Mutex<u16> = std::sync::Mutex::new(0);

    fn status_text(app: &crate::http::App) -> (String, bool) {
        let state = app.latest.read().unwrap().clone();
        let ru = app.store.lock().unwrap().get().appearance.language != "en";
        let (text, live) = match state.status {
            "live" => (if ru { "Подключено" } else { "Connected" }, true),
            "waiting-auth" => (if ru { "Ожидание авторизации" } else { "Waiting for auth" }, false),
            "connecting" => (if ru { "Подключение…" } else { "Connecting…" }, false),
            "error" => (if ru { "Ошибка" } else { "Error" }, false),
            _ => (if ru { "Не подключено" } else { "Disconnected" }, false),
        };
        let source = if state.source == "ts6" { "TS6" } else if state.source == "ts3" { "TS3" } else { "Mock" };
        (format!("{} · {}", source, text), live)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The icon embedded by build.rs (RT_GROUP_ICON id 1); falls back to the
    /// drawn placeholder / IDI_APPLICATION when the resource is missing.
    fn app_icon(cx: i32, cy: i32) -> *mut c_void {
        unsafe {
            let inst = GetModuleHandleW(std::ptr::null());
            let h = LoadImageW(inst, 1 as *const u16, 1 /* IMAGE_ICON */, cx, cy, 0x00000040 /* LR_DEFAULTCOLOR */);
            if !h.is_null() {
                return h;
            }
            if cx <= 32 && cy <= 32 {
                let t = tray_icon();
                if !t.is_null() {
                    return t;
                }
            }
            LoadIconW(null_mut(), 32512 as *const u16)
        }
    }

    fn tray_icon() -> *mut c_void {
        let size = 32i32;
        let mut b = crate::render::Buf::new(size, size);
        let pal = crate::theme::Palette::from_appearance(&crate::config::Appearance::default());
        b.round_rect(1.0, 1.0, 30.0, 30.0, 8.0, pal.panel_bg);
        b.stroke_round_rect(1.0, 1.0, 30.0, 30.0, 8.0, pal.accent.scale_a(200.0), 1.5);
        b.circle(16.0, 16.0, 6.5, pal.accent);
        b.circle(16.0, 16.0, 2.5, pal.panel_bg);
        let mut di = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: 40,
                biWidth: size,
                biHeight: size * 2,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                biSizeImage: 0,
                biXPelsPerMeter: 2835,
                biYPelsPerMeter: 2835,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [0; 3],
        };
        let mut px = vec![0u8; (size * size * 4) as usize];
        for y in 0..size as usize {
            for x in 0..size as usize {
                let idx = ((y * size as usize) + x) * 4;
                px[idx] = b.px[idx + 2]; // B
                px[idx + 1] = b.px[idx + 1]; // G
                px[idx + 2] = b.px[idx]; // R
                px[idx + 3] = b.px[idx + 3]; // A
            }
        }
        unsafe {
            let mut bits: *mut c_void = null_mut();
            let dib = CreateDIBSection(null_mut(), &mut di, 0, &mut bits, null_mut(), 0);
            if dib.is_null() || bits.is_null() {
                return LoadIconW(null_mut(), 32512 as *const u16);
            }
            std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, px.len());
            let icon_info = ICONINFO { f_icon: 1, x_hotspot: 0, y_hotspot: 0, hbm_mask: null_mut(), hbm_color: dib };
            let icon = CreateIconIndirect(&icon_info);
            DeleteObject(dib);
            if icon.is_null() {
                return LoadIconW(null_mut(), 32512 as *const u16);
            }
            icon
        }
    }

    static TRAY_HWND: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// One-shot tray balloon («Если игра запущена от администратора…»).
    pub fn tray_balloon(title: &str, text: &str) {
        unsafe {
            let hwnd = TRAY_HWND.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
            if hwnd.is_null() {
                return;
            }
            let mut nid = tray_nid(hwnd, null_mut(), "");
            nid.u_flags = NIF_INFO;
            let t: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
            let m: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let n = t.len().min(63);
            nid.sz_info_title[..n].copy_from_slice(&t[..n]);
            let n = m.len().min(255);
            nid.sz_info[..n].copy_from_slice(&m[..n]);
            nid.dw_info_flags = NIIF_INFO;
            Shell_NotifyIconW(NIM_MODIFY, &mut nid);
        }
    }

    /// One-shot hint: a fullscreen ADMIN window can only be covered by an
    /// elevated overlay («Запуск от имени администратора»).
    fn maybe_admin_hint() {
        static ADMIN_HINTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if ADMIN_HINTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let ru = TRAY
            .lock()
            .unwrap()
            .as_ref()
            .map(|app| app.store.lock().unwrap().get().appearance.language != "en")
            .unwrap_or(true);
        if ru {
            tray_balloon(
                "TS3-TS6 Notifier",
                "Игра запущена от администратора — включите «Запуск от имени администратора» в настройках, чтобы оверлей был поверх неё",
            );
        } else {
            tray_balloon(
                "TS3-TS6 Notifier",
                "The game runs as administrator — enable \"Run as administrator\" in settings to keep the overlay above it",
            );
        }
    }

    fn tray_nid(hwnd: *mut c_void, icon: *mut c_void, tip: &str) -> NOTIFYICONDATAW {
        let t: Vec<u16> = tip.encode_utf16().chain(std::iter::once(0)).collect();
        let mut nid = NOTIFYICONDATAW {
            cb_size: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hwnd,
            u_id: 1,
            u_flags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            u_callback: WM_APP_TRAY,
            h_icon: icon,
            sz_tip: [0; 128],
            dw_state: 0,
            dw_state_mask: 0,
            sz_info: [0; 256],
            u_timeout_version: 0,
            sz_info_title: [0; 64],
            dw_info_flags: 0,
            guid_item: [0; 16],
            h_balloon_icon: null_mut(),
        };
        let n = t.len().min(127);
        nid.sz_tip[..n].copy_from_slice(&t[..n]);
        nid
    }

    /// Ask for per-monitor DPI v2 on this thread (Win10 1607+): system menus
    /// drawn by a DPI-unaware thread get bitmap-stretched and look blurry.
    fn set_thread_dpi_aware() {
        unsafe {
            let u = GetModuleHandleW(wide("user32").as_ptr());
            if !u.is_null() {
                let p = GetProcAddress(u, b"SetThreadDpiAwarenessContext\0".as_ptr());
                if !p.is_null() {
                    let f: extern "system" fn(isize) -> isize = std::mem::transmute(p);
                    f(-4); // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
                }
            }
        }
    }

    /// Logical-pixel ratio of this DPI-unaware process (physical / logical).
    pub fn dpi_scale() -> f64 {
        unsafe {
            let u = GetModuleHandleW(wide("user32").as_ptr());
            if !u.is_null() {
                let p = GetProcAddress(u, b"GetDpiForSystem\0".as_ptr());
                if !p.is_null() {
                    let f: extern "system" fn() -> u32 = std::mem::transmute(p);
                    return ((f() as f64) / 96.0).max(1.0);
                }
            }
            1.0
        }
    }

    fn show_tray_menu(hwnd: *mut c_void, app: &std::sync::Arc<crate::http::App>, port: u16) {
        set_thread_dpi_aware();
        let cfg = app.store.lock().unwrap().get().clone();
        let ru = cfg.appearance.language != "en";
        unsafe {
            let menu = CreatePopupMenu();
            let sub = CreatePopupMenu();
            let sources = ["mock", "ts3", "ts6"];
            let labels: [&str; 3] = if ru {
                ["Mock (тестовый)", "TeamSpeak 3", "TeamSpeak 6"]
            } else {
                ["Mock (test)", "TeamSpeak 3", "TeamSpeak 6"]
            };
            for (i, src) in sources.iter().enumerate() {
                let mut flags = MF_STRING;
                if cfg.source == *src {
                    flags |= MF_CHECKED;
                }
                AppendMenuW(sub, flags, 101 + i, wide(labels[i]).as_ptr());
            }
            AppendMenuW(menu, MF_POPUP, sub as usize, wide(if ru { "Источник данных" } else { "Data source" }).as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(menu, MF_STRING, 111, wide(if ru { "Настройки" } else { "Settings" }).as_ptr());
            {
                let vis = app.store.lock().unwrap().get().desktop.visible;
                let label = if ru {
                    if vis { "Скрыть оверлей" } else { "Показывать оверлей" }
                } else {
                    if vis { "Hide overlay" } else { "Show overlay" }
                };
                AppendMenuW(menu, MF_STRING, 121, wide(label).as_ptr());
            }
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(menu, MF_STRING, 120, wide(if ru { "Выход" } else { "Exit" }).as_ptr());
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, null());
            // DestroyMenu(menu) recursively destroys the attached submenus —
            // destroying `sub` again corrupts the Win32 menu heap (app crash!)
            DestroyMenu(menu);
            if cmd == 0 {
                return;
            }
            let cmd = cmd as usize;
            if (101..=103).contains(&cmd) {
                let src = sources[cmd - 101];
                let patch = crate::json::obj(vec![("source", crate::json::Json::s(src))]);
                app.store.lock().unwrap().update(&patch);
                (app.restart_connector)();
            } else if cmd == 111 {
                crate::settings::spawn_once(app.clone(), port);
            } else if cmd == 121 {
                // «отключает показ оверлея … не выходя из приложения»
                let on = !app.store.lock().unwrap().get().desktop.visible;
                let patch = crate::json::obj(vec![(
                    "desktop",
                    crate::json::obj(vec![("visible", crate::json::Json::b(on))]),
                )]);
                app.store.lock().unwrap().update(&patch);
            } else if cmd == 120 {
                let mut nid = tray_nid(hwnd, null_mut(), "");
                Shell_NotifyIconW(NIM_DELETE, &mut nid);
                // «установится при выходе из приложения»
                crate::settings::try_launch_pending_update();
                std::process::exit(0);
            }
        }
    }

    unsafe extern "system" fn tray_wndproc(hwnd: *mut c_void, msg: u32, w: usize, l: isize) -> isize {
        match msg {
            WM_APP_TRAY => {
                let app = TRAY.lock().unwrap().clone();
                if let Some(app) = app {
                    if (l as u32) == WM_RBUTTONUP || (l as u32) == WM_LBUTTONUP {
                        let port = *TRAY_PORT.lock().unwrap();
                        show_tray_menu(hwnd, &app, port);
                    }
                }
                0
            }
            WM_TIMER => {
                let app = TRAY.lock().unwrap().clone();
                if let Some(app) = app {
                    let (tip, _) = status_text(&app);
                    let mut nid = tray_nid(hwnd, null_mut(), &tip);
                    nid.u_flags = NIF_TIP;
                    Shell_NotifyIconW(NIM_MODIFY, &mut nid);
                }
                0
            }
            WM_COMMAND => 0,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }

    pub fn spawn_tray(app: std::sync::Arc<crate::http::App>, port: u16) {
        unsafe {
            *TRAY.lock().unwrap() = Some(app.clone());
            *TRAY_PORT.lock().unwrap() = port;
            let class = wide("TSNTrayClass");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: tray_wndproc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: std::ptr::null_mut(),
                hIcon: std::ptr::null_mut(),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW as *const u16),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
                hIconSm: std::ptr::null_mut(),
            };
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null_mut(),
            );
            if hwnd.is_null() {
                return;
            }
            let hwnd_addr = hwnd as usize;
            std::thread::spawn(move || unsafe {
                let hwnd = hwnd_addr as *mut c_void;
                TRAY_HWND.store(hwnd_addr, std::sync::atomic::Ordering::Relaxed);
                let icon = app_icon(32, 32);
                let (tip, _) = status_text(&app);
                let mut nid = tray_nid(hwnd, icon, &tip);
                Shell_NotifyIconW(NIM_ADD, &mut nid);
                SetTimer(hwnd, 1, 2000, null_mut());
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, hwnd, 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                Shell_NotifyIconW(NIM_DELETE, &mut nid);
            });
        }
    }

    /// Copy `text` to the Windows clipboard.
    /// Open a URL in the default browser («Открыть панель»).
    pub fn open_url(url: &str) {
        unsafe {
            let u: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
            ShellExecuteW(null_mut(), wide("open").as_ptr(), u.as_ptr(), null(), null(), 1);
        }
    }

    pub fn set_clipboard_text(text: &str) {
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return;
            }
            EmptyClipboard();
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = wide.len() * 2;
            let h = GlobalAlloc(0x0002 /* GMEM_MOVEABLE */, bytes);
            if !h.is_null() {
                let dst = GlobalLock(h);
                if !dst.is_null() {
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), dst as *mut u16, wide.len());
                    GlobalUnlock(h);
                    SetClipboardData(CF_UNICODETEXT, h);
                }
            }
            CloseClipboard();
        }
    }

    /// Current clipboard text ("" when empty or unavailable).
    pub fn get_clipboard_text() -> String {
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return String::new();
            }
            let mut out = String::new();
            let h = GetClipboardData(CF_UNICODETEXT);
            if !h.is_null() {
                let p = GlobalLock(h) as *const u16;
                if !p.is_null() {
                    let mut len = 0usize;
                    while *p.add(len) != 0 {
                        len += 1;
                    }
                    let slice = std::slice::from_raw_parts(p, len);
                    out = String::from_utf16_lossy(slice);
                    GlobalUnlock(h);
                }
            }
            CloseClipboard();
            out
        }
    }

    /// Download `url` to `path` via urlmon (WinINet HTTPS). Used by the
    /// update check — no TLS stack needed in-process.
    pub fn fetch_url_to_file(url: &str, path: &std::path::Path) -> bool {
        #[cfg_attr(windows, link(name = "urlmon"))]
        extern "system" {
            fn URLDownloadToFileW(
                p0: *mut c_void,
                sz_url: *const u16,
                sz_file: *const u16,
                reserved: u32,
                cb: *mut c_void,
            ) -> i32;
        }
        #[cfg_attr(windows, link(name = "ole32"))]
        extern "system" {
            fn CoInitializeEx(reserved: *mut c_void, model: u32) -> i32;
        }
        unsafe {
            CoInitializeEx(std::ptr::null_mut(), 0);
        }
        let wurl: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let wpath: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            URLDownloadToFileW(
                std::ptr::null_mut(),
                wurl.as_ptr(),
                wpath.as_ptr(),
                0,
                std::ptr::null_mut(),
            ) == 0
        }
    }

    /// Launch a file with the shell ("open" verb) — used to run the downloaded
    /// update installer.
    pub fn shell_open(path: &str) {
        unsafe {
            ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(path).as_ptr(), null(), null(), 1);
        }
    }

    /// ShellExecuteW "open" — launch the default browser on `url`.
    pub fn open_url_external(url: &str) {
        let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let op: Vec<u16> = "open\0".encode_utf16().collect();
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                op.as_ptr(),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            );
        }
    }

    /// The settings window: a separate taskbar app («настройки отдельным
    /// приложением в панели задач»).
    pub fn run_settings(app: std::sync::Arc<crate::http::App>) {
        set_thread_dpi_aware(); // crisp rendering at fractional display scaling
        let mut host = unsafe { WinHost::new() };
        host.settings_host = true;
        unsafe {
            SETTINGS_HWND.store(host.hwnd as usize, std::sync::atomic::Ordering::SeqCst);
            // taskbar button + normal-window behaviour: not a tool window, not
            // always-on-top (WM_ACTIVATE manages z-order), human title
            SetWindowLongW(
                host.hwnd,
                GWL_EXSTYLE,
                WS_EX_LAYERED | WS_EX_APPWINDOW,
            );
            SetWindowTextW(host.hwnd, wide("TS3-TS6 Notifier — Настройки").as_ptr());
        }
        let cfg = app.store.lock().unwrap().get().clone();
        let mut m = crate::settings::SettingsModel::new_full(cfg.appearance.clone(), cfg.desktop.clone(), app.version.clone());
        m.source = cfg.source.clone();
        m.ts3 = cfg.ts3.clone();
        m.ts6 = cfg.ts6.clone();
        for (id, cls) in &cfg.classes.map {
            if let Some(row) = m.groups.iter_mut().find(|(rid, _)| rid == id) {
                row.1 = cls.clone();
            }
        }
        let fonts = crate::font::Fonts::load();
        let icons = crate::assets::Icons::load();
        crate::settings::run_loop(&mut m, &app, &fonts, &icons, &mut host);
    }

    pub fn run(ov: &mut Overlay) {
        start_input_hook();
        let mut host = unsafe { WinHost::new() };
        loop {
            ov.tick(&mut host);
            std::thread::sleep(Duration::from_millis(33));
        }
    }

    /// The system-wide mouse hook lives on its OWN message-pump thread: it must
    /// never be hostage to render/present/locks — while a hook thread is busy,
    /// ALL mouse input on the whole system stalls.
    pub fn start_input_hook() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
        std::thread::Builder::new()
            .name("input-hook".into())
            .spawn(|| unsafe {
                SetWindowsHookExW(
                    WH_MOUSE_LL,
                    mouse_ll_proc as usize as *mut c_void,
                    std::ptr::null_mut(),
                    0,
                );
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            })
            .ok();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChatMessage;

    #[test]
    fn coords_roundtrip_and_clamp() {
        let work = (0.0, 0.0, 1920.0, 1040.0);
        let (pw, ph) = (292.0, 400.0);
        // «Координата X/Y» are absolute: position round-trips through the
        // panel top-left for any spot on the screen
        for (x, y) in [(0.0, 0.0), (18.0, 24.0), (900.0, 500.0), (1628.0, 640.0)] {
            let (px, py) = panel_pos(work, x, y);
            let (rx, ry) = coords_from_pos(work, px, py);
            assert!((rx - x).abs() < 1e-6 && (ry - y).abs() < 1e-6, "{x},{y}");
        }
        // clamped: panel always fully visible («не уводя за рамки экрана»)
        let (cx, cy) = clamp_coords(work, pw, ph, -50.0, 9999.0);
        assert_eq!((cx, cy), (0.0, 1040.0 - ph));
        let (cx, cy) = clamp_coords(work, pw, ph, 9999.0, -5.0);
        assert_eq!((cx, cy), (1920.0 - pw, 0.0));
        // corner presets snap exactly into the corner
        assert_eq!(corner_coords(work, "tl", pw, ph), (0.0, 0.0));
        assert_eq!(corner_coords(work, "br", pw, ph), (1920.0 - pw, 1040.0 - ph));
    }

    #[test]
    fn whisper_split_and_linger() {
        let mut seen = HashMap::new();
        let u = |id: &str, whisper: bool| {
            let mut x = UserState::new(id, "N");
            x.whispering = whisper;
            x.talking = whisper;
            x.input_muted = true; // full state must survive into the whisper block
            x
        };
        // whispering user lives in the whisper block ONLY (app.js containerFor)
        let (roster, whisper) = split_whisper(vec![u("1", true), u("2", false)], &mut seen, 1000, 4000);
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].id, "2");
        assert_eq!(whisper.len(), 1);
        assert!(whisper[0].input_muted, "full UserState preserved");
        // stops whispering — lingers 4s, then returns to the roster
        let (roster, whisper) = split_whisper(vec![u("1", false), u("2", false)], &mut seen, 3000, 4000);
        assert_eq!((roster.len(), whisper.len()), (1, 1));
        let (roster, whisper) = split_whisper(vec![u("1", false), u("2", false)], &mut seen, 4900, 4000);
        assert_eq!((roster.len(), whisper.len()), (1, 1), "3.9s < 4s linger");
        let (roster, whisper) = split_whisper(vec![u("1", false), u("2", false)], &mut seen, 5500, 4000);
        assert_eq!((roster.len(), whisper.len()), (2, 0), "4.5s past linger — back to roster");
    }

    #[test]
    fn feed_filters_and_order() {
        let mut a = crate::config::Appearance::default();
        a.events_count = 2.0;
        a.show_ev_join = true;
        a.show_ev_leave = false;
        a.ev_scope_move = "own".into();
        let mut st = OverlayState::empty("mock", "live", "");
        let mut m = ChatMessage {
            id: "c1".into(),
            user_id: "1".into(),
            nickname: "A".into(),
            text: "A".into(),
            kind: ChatKind::System,
            at: 199_000,
            event: Some("join"),
            detail: Some("General".into()),
            own: Some(true),
        };
        st.chat.push(m.clone());
        m.id = "c2".into();
        m.event = Some("leave");
        m.at = 199_100;
        st.chat.push(m.clone());
        m.id = "c3".into();
        m.event = Some("move");
        m.own = Some(false); // filtered by own scope
        m.at = 199_200;
        st.chat.push(m.clone());
        m.id = "c4".into();
        m.event = Some("move");
        m.own = Some(true);
        m.detail = Some("AFK".into());
        m.at = 199_300;
        st.chat.push(m.clone());
        let feed = build_feed(&st, &a, "ru", 200_000);
        // leave filtered, move-not-own filtered, cap 2 → newest pair
        assert_eq!(feed.len(), 2);
        assert_eq!(feed[0].nickname, "A");
        assert!(feed[0].text.contains("General"));
        assert!(feed[1].text.contains("AFK"));
    }

    #[test]
    fn feed_rows_expire_and_fade() {
        // «Чат(панель событий) пропадает через определённое время (затухает)»
        let mut a = crate::config::Appearance::default();
        a.chat_delay = 5.0;
        a.events_delay = 5.0;
        a.events_count = 10.0;
        let mut st = OverlayState::empty("mock", "live", "");
        for (id, at) in [("f1", 4_600u64), ("f2", 1_500), ("f3", 0)] {
            st.chat.push(ChatMessage {
                id: id.into(),
                user_id: "1".into(),
                nickname: "A".into(),
                text: "hi".into(),
                kind: ChatKind::Channel,
                at,
                event: None,
                detail: None,
                own: None,
            });
        }
        let feed = build_feed(&st, &a, "ru", 5_000);
        assert_eq!(feed.len(), 2, "expired row is dropped");
        assert_eq!(feed[0].nickname, "A");
        assert_eq!(feed[0].alpha, 1.0, "fresh row is opaque");
        assert!(
            feed[1].alpha < 1.0 && feed[1].alpha > 0.0,
            "row fades during the last third of its life (got {})",
            feed[1].alpha
        );
    }

    #[test]
    fn roster_caps_at_ten() {
        // «до 10 человек на канале»; whisperers are split out BEFORE the cap
        let users: Vec<UserState> = (0..15)
            .map(|i| UserState::new(&format!("u{i}"), &format!("U{i}")))
            .collect();
        assert_eq!(cap_roster(users.clone()).len(), MAX_ROSTER);
        let (roster, whisper) = split_whisper(users, &mut HashMap::new(), 1000, 4000);
        assert!(whisper.is_empty(), "nobody whispered");
        assert_eq!(cap_roster(roster).len(), MAX_ROSTER);
    }

    #[test]
    fn roster_filters() {
        let mut a = crate::config::Appearance::default();
        a.talking_first = true;
        let mut u1 = UserState::new("1", "A");
        u1.talking = true;
        let u2 = UserState::new("2", "B");
        let mut u3 = UserState::new("3", "Q");
        u3.is_query = true;
        let rows = roster_rows(&[u2.clone(), u3, u1.clone()], &a);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "1"); // talking first
        a.show_self = false;
        u1.is_self = true;
        let rows = roster_rows(&[u1, u2], &a);
        assert_eq!(rows.len(), 1);
    }
}
