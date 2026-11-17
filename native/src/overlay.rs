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
pub fn panel_pos(work: (f64, f64, f64, f64), corner: &str, mx: f64, my: f64, pw: f64, ph: f64) -> (f64, f64) {
    let (wx, wy, ww, wh) = work;
    match corner {
        "tl" => (wx + mx, wy + my),
        "tr" => (wx + ww - mx - pw, wy + my),
        "bl" => (wx + mx, wy + wh - my - ph),
        _ => (wx + ww - mx - pw, wy + wh - my - ph), // br
    }
}

/// Inverse: margins from the panel top-left. Grows past half-screen fine.
pub fn margins_from_pos(work: (f64, f64, f64, f64), corner: &str, px: f64, py: f64, pw: f64, ph: f64) -> (f64, f64) {
    let (wx, wy, ww, wh) = work;
    let mx = match corner {
        "tl" | "bl" => px - wx,
        _ => wx + ww - px - pw,
    };
    let my = match corner {
        "tl" | "tr" => py - wy,
        _ => wy + wh - py - ph,
    };
    (mx, my)
}

/// Clamp so the panel always stays fully visible («оверлей всегда на экране»).
pub fn clamp_margins(work: (f64, f64, f64, f64), pw: f64, ph: f64, mx: f64, my: f64) -> (f64, f64) {
    let max_x = (work.2 - pw).max(0.0);
    let max_y = (work.3 - ph).max(0.0);
    (mx.clamp(0.0, max_x), my.clamp(0.0, max_y))
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

/// Build feed rows from state.chat, applying showEv*/scope filters (newest last).
pub fn build_feed(state: &OverlayState, a: &crate::config::Appearance, lang: &str) -> Vec<FeedItem> {
    let mut out: Vec<FeedItem> = Vec::new();
    for m in state.chat.iter().rev() {
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
}

pub trait Host {
    fn present(&mut self, buf: &crate::render::Buf, panel_w: f32, panel_h: f32, win_x: f64, win_y: f64);
    fn pump(&mut self) -> Vec<Ev>;
    fn set_interactive(&mut self, on: bool);
    fn open_url(&self, url: &str);
    fn work_area(&self) -> (f64, f64, f64, f64);
}

/// Headless host — tests and non-windows builds.
pub struct HeadlessHost {
    pub last_w: i32,
    pub last_h: i32,
    pub interactive: bool,
}

impl HeadlessHost {
    pub fn new() -> HeadlessHost {
        HeadlessHost { last_w: 0, last_h: 0, interactive: false }
    }
}

impl Host for HeadlessHost {
    fn present(&mut self, buf: &crate::render::Buf, _pw: f32, _ph: f32, _x: f64, _y: f64) {
        self.last_w = buf.w;
        self.last_h = buf.h;
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
}

/* ---------------- window loop ---------------- */

pub struct Overlay {
    app: Arc<App>,
    pub port: u16,
    fonts: Fonts,
    icons: Icons,
    whisper_seen: HashMap<String, u64>,
    pending: Option<(f64, f64)>,
    saved_margins: Option<(f64, f64)>,
    dragging: bool,
    drag_off: (f64, f64),
    last_size: (f64, f64),
}

impl Overlay {
    pub fn new(app: Arc<App>, port: u16) -> Overlay {
        Overlay {
            app,
            port,
            fonts: Fonts::load(),
            icons: Icons::load(),
            whisper_seen: HashMap::new(),
            pending: None,
            saved_margins: None,
            dragging: false,
            drag_off: (0.0, 0.0),
            last_size: (PANEL_W as f64, 200.0),
        }
    }

    fn snapshot(&self) -> OverlayState {
        self.app.latest.read().unwrap().clone()
    }

    /// One iteration: build the frame, present, process input.
    pub fn tick(&mut self, host: &mut dyn Host) {
        let state = self.snapshot();
        let now_ms = crate::model::now_ms();
        let cfg = self.app.store.lock().unwrap().get().clone();
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

        let all = roster_rows(&state.users, &a);
        let linger = (a.whisper_delay.max(0.0) * 1000.0) as u64;
        let (rows, whisperers) = if a.show_whispers {
            split_whisper(all, &mut self.whisper_seen, now_ms, linger)
        } else {
            (all, Vec::new())
        };
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
        let feed = build_feed(&state, &a, lang);

        let move_mode = self.app.move_mode.load(Ordering::SeqCst);
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
            class_keys,
            whisper_keys,
            whisperers,
            feed,
            lang,
            move_mode,
            now_ms,
            work_h: host.work_area().3 as f32,
        };
        let frame = render_frame(&inp);

        // placement (css px margins; frame sizes are scaled css px)
        let work = host.work_area();
        let (mut mx, mut my) = self.pending.unwrap_or((a.margin_x, a.margin_y));
        let pw = frame.panel_w as f64;
        let ph = frame.panel_h as f64;
        let (mx2, my2) = clamp_margins(work, pw, ph, mx, my);
        mx = mx2;
        my = my2;
        let (px, py) = panel_pos(work, &a.corner, mx, my, pw, ph);
        self.last_size = (pw, ph);
        // window rect includes the shadow margin
        let pad = (SHADOW_PAD * a.scale.clamp(0.5, 3.0) as f32) as f64;
        host.present(&frame.buf, frame.panel_w, frame.panel_h, px - pad, py - pad);

        // input
        let events = host.pump();
        let interactive_wanted = move_mode || self.dragging;
        host.set_interactive(interactive_wanted);
        for ev in events {
            match ev {
                Ev::Click(x, y) => {
                    for (rect, hit) in &frame.hits {
                        if rect.contains(x, y) {
                            match hit {
                                Hit::Gear => {
                                    host.open_url(&format!("http://127.0.0.1:{}/", self.port));
                                }
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
                    let (mmx, mmy) = margins_from_pos(work, &a.corner, npx, npy, pw, ph);
                    let (mmx, mmy) = clamp_margins(work, pw, ph, mmx, mmy);
                    self.pending = Some((mmx, mmy));
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
            let mut ov = Overlay::new(app, port);
            #[cfg(windows)]
            {
                win32::run(&mut ov);
            }
            #[cfg(not(windows))]
            {
                let mut host = HeadlessHost::new();
                loop {
                    ov.tick(&mut host);
                    std::thread::sleep(Duration::from_millis(66));
                }
            }
        })
        .expect("overlay thread")
}

/* ---------------- win32 host (windows only) ---------------- */

#[cfg(windows)]
mod win32 {
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

    #[link(name = "user32")]
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
    }

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> *mut c_void;
    }

    #[link(name = "gdi32")]
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
    const WM_LBUTTONUP: u32 = 0x0202;
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

    struct WinHost {
        hwnd: *mut c_void,
        mem_dc: *mut c_void,
        dib: *mut c_void,
        dib_bits: *mut u8,
        w: i32,
        h: i32,
        interactive: bool,
        drag_pending: bool,
    }

    unsafe extern "system" fn wnd_proc(hwnd: *mut c_void, msg: u32, wp: usize, lp: isize) -> isize {
        match msg {
            WM_DESTROY => {
                PostQuitMessage(0);
                0
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
                hIcon: std::ptr::null_mut(),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW as *const u16),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: cls.as_ptr(),
                hIconSm: std::ptr::null_mut(),
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
                mem_dc: std::ptr::null_mut(),
                dib: std::ptr::null_mut(),
                dib_bits: std::ptr::null_mut(),
                w: 0,
                h: 0,
                interactive: false,
                drag_pending: false,
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

    impl Host for WinHost {
        fn present(&mut self, buf: &Buf, _pw: f32, _ph: f32, win_x: f64, win_y: f64) {
            unsafe {
                self.ensure_surface(buf.w, buf.h);
                // RGBA -> BGRA
                let n = (buf.w * buf.h) as usize;
                for i in 0..n {
                    let s = i * 4;
                    *self.dib_bits.add(s) = buf.px[s + 2];
                    *self.dib_bits.add(s + 1) = buf.px[s + 1];
                    *self.dib_bits.add(s + 2) = buf.px[s];
                    *self.dib_bits.add(s + 3) = buf.px[s + 3];
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

        fn pump(&mut self) -> Vec<Ev> {
            let mut out = Vec::new();
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    let mut to_screen = |lp: isize| -> (f32, f32) {
                        let mut p = POINT { x: (lp & 0xffff) as i32, y: ((lp >> 16) & 0xffff) as i32 };
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

        fn open_url(&self, url: &str) {
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

    pub fn run(ov: &mut Overlay) {
        let mut host = unsafe { WinHost::new() };
        loop {
            ov.tick(&mut host);
            std::thread::sleep(Duration::from_millis(33));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChatMessage;

    #[test]
    fn margin_roundtrip_and_clamp() {
        let work = (0.0, 0.0, 1920.0, 1040.0);
        let (pw, ph) = (292.0, 400.0);
        for corner in ["tl", "tr", "bl", "br"] {
            let (x, y) = panel_pos(work, corner, 18.0, 24.0, pw, ph);
            let (mx, my) = margins_from_pos(work, corner, x, y, pw, ph);
            assert!((mx - 18.0).abs() < 1e-6 && (my - 24.0).abs() < 1e-6, "{corner}");
        }
        // clamped: panel always fully visible
        let (mx, my) = clamp_margins(work, pw, ph, -50.0, 9999.0);
        assert_eq!((mx, my), (0.0, 1040.0 - ph));
        let (mx, my) = clamp_margins(work, pw, ph, 9999.0, -5.0);
        assert_eq!((mx, my), (1920.0 - pw, 0.0));
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
            at: 1,
            event: Some("join"),
            detail: Some("General".into()),
            own: Some(true),
        };
        st.chat.push(m.clone());
        m.id = "c2".into();
        m.event = Some("leave");
        m.at = 2;
        st.chat.push(m.clone());
        m.id = "c3".into();
        m.event = Some("move");
        m.own = Some(false); // filtered by own scope
        m.at = 3;
        st.chat.push(m.clone());
        m.id = "c4".into();
        m.event = Some("move");
        m.own = Some(true);
        m.detail = Some("AFK".into());
        m.at = 4;
        st.chat.push(m.clone());
        let feed = build_feed(&st, &a, "ru");
        // leave filtered, move-not-own filtered, cap 2 → newest pair
        assert_eq!(feed.len(), 2);
        assert_eq!(feed[0].nickname, "A");
        assert!(feed[0].text.contains("General"));
        assert!(feed[1].text.contains("AFK"));
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
