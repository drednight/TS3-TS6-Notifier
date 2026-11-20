//! TS3-TS6 Notifier — native core: TeamSpeak connectors, state model and the
//! HTTP/WS server for the web overlay. Drop-in replacement for the Node
//! server (same config.json, same wire protocol).

// GUI app on Windows: no console window.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod assets;
mod auto;
mod chatlog;
mod config;
mod diag;
mod http;
mod icons;
mod font;
mod json;
mod model;
mod layout;
mod mock;
mod overlay;
mod plugin;
mod render;
mod settings;
mod theme;
mod ts3;
mod ts6;
mod wire;
mod wsframe;

use config::ConfigStore;
use http::App;
use json::Json;
use model::{now_ms, OverlayState};
use plugin::Emit;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

enum Handle {
    /// v2.0.32: TS3 — только ClientQuery (Ts3Connector), как в референсе.
    /// Плагин и мост 48912 выведены из рабочего пути (полное удаление — 2.1.0).
    Ts3(Arc<AtomicBool>),
    Mock(mock::MockConnector),
    Ts6(Arc<AtomicBool>),
    /// v2.0.34: супервисор автоопределения (внешний stop останавливает и
    /// текущий коннектор внутри).
    Auto(Arc<AtomicBool>),
    Stub,
}

impl Handle {
    fn stop(&self) {
        match self {
            Handle::Ts3(stop) | Handle::Ts6(stop) | Handle::Auto(stop) => {
                stop.store(true, std::sync::atomic::Ordering::SeqCst)
            }
            Handle::Mock(c) => c.stop(),
            Handle::Stub => {}
        }
    }
}

/// v2.0.34: «connecting/offline/waiting-auth» с пустым списком наследует
/// прежний ростер — при переподключении и смене источника меняется только
/// статус, список людей не мигает (правило v2.0.32, обобщённое на app-уровень).
fn inherit_roster(prev: Option<&OverlayState>, st: &mut OverlayState) {
    let bare = matches!(st.status, "connecting" | "offline" | "waiting-auth") && st.users.is_empty();
    if !bare {
        return;
    }
    let Some(p) = prev else { return };
    if p.users.is_empty() {
        return;
    }
    st.users = p.users.clone();
    st.whisperers = p.whisperers.clone();
    st.chat = p.chat.clone();
    st.groups = p.groups.clone();
    if st.server_name.is_empty() {
        st.server_name = p.server_name.clone();
    }
    if st.channel_name.is_empty() {
        st.channel_name = p.channel_name.clone();
    }
    if st.channel_id.is_empty() {
        st.channel_id = p.channel_id.clone();
    }
}

/// Coalesce state bursts into at most ~30 broadcasts/second (same as the TS
/// connectors' emitState throttle). `live` — 1, пока источник отдаёт статус
/// «live» (супервисор автоопределения решает, нужен ли handover).
fn throttled_sink(app: Arc<App>, live: Arc<std::sync::atomic::AtomicU8>) -> Emit {
    let (tx, rx) = channel::<OverlayState>();
    thread::spawn(move || {
        let mut pending: Option<OverlayState> = None;
        let mut prev: Option<OverlayState> = None;
        let mut last_sent = 0u64;
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(st) => pending = Some(st),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(newer) = rx.try_recv() {
                pending = Some(newer);
            }
            let Some(st) = pending.take() else { continue };
            let elapsed = now_ms() - last_sent;
            // v2.0.32: 30 мс вместо 100 — цель «событие → кадр» < 30–50 мс
            if elapsed < 30 {
                thread::sleep(Duration::from_millis(30 - elapsed));
                while let Ok(newer) = rx.try_recv() {
                    pending = Some(newer);
                }
            }
            let mut st = pending.take().unwrap_or(st);
            inherit_roster(prev.as_ref(), &mut st);
            live.store(
                if st.status == "live" { 1 } else { 0 },
                std::sync::atomic::Ordering::SeqCst,
            );
            prev = Some(st.clone());
            last_sent = now_ms();
            app.push_state(&st);
        }
    });
    Arc::new(move |st| {
        let _ = tx.send(st);
    })
}

/// v2.0.34: супервисор автоопределения источника.
/// Пробуем TS3/TS6, запускаем победителя; если источник долго «не live»
/// и другой доступен — плавно переключаемся (handover).
///
/// v2.1.0: переключение через 8 с молчания (было 20 с) и активный источник
/// сразу публикуется в состоянии (`source` = ts3/ts6), чтобы оверлей, трей и
/// диагностика показывали, кто реально кормит данные в режиме «Авто».
fn run_auto_connector(
    store: Arc<Mutex<ConfigStore>>,
    emit: Emit,
    live: Arc<std::sync::atomic::AtomicU8>,
    stop: Arc<AtomicBool>,
) {
    const HANDOVER_MS: u64 = 8_000;
    thread::spawn(move || {
        loop {
            if stop.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let cfg = store.lock().unwrap().get().clone();
            let s3 = auto::probe_ts3(&cfg.ts3.host, cfg.ts3.port);
            let s6 = auto::probe_ts6(&cfg.ts6.host, cfg.ts6.port);
            let winner = auto::pick_source(s3, s6);
            let name = |src: auto::Source| match src {
                auto::Source::Ts3 => "ts3",
                auto::Source::Ts6 => "ts6",
            };
            diag::diag("auto", &format!("probe ts3={s3} ts6={s6} -> {}", name(winner)));
            // источник в состоянии — фактический, не «auto» (никаких двух
            // разных «источников» в интерфейсе: один выбор = один активный)
            emit(crate::model::OverlayState::empty(name(winner), "connecting", ""));

            let inner = Arc::new(AtomicBool::new(false));
            {
                let inner2 = inner.clone();
                let emit2 = emit.clone();
                let store2 = store.clone();
                thread::spawn(move || match winner {
                    auto::Source::Ts3 => {
                        let c = store2.lock().unwrap().get().clone();
                        ts3::Ts3Connector::run(c.ts3.host, c.ts3.port, c.ts3.api_key, inner2, move |st| {
                            emit2(st)
                        });
                    }
                    auto::Source::Ts6 => {
                        let c = store2.lock().unwrap().get().clone();
                        let save_key: ts6::SaveKeyFn = {
                            let store = store2.clone();
                            Arc::new(move |key: String| {
                                let patch = crate::json::obj(vec![(
                                    "ts6",
                                    crate::json::obj(vec![("apiKey", Json::s(key))]),
                                )]);
                                let _ = store.lock().unwrap().update(&patch);
                            })
                        };
                        ts6::run(c.ts6.host, c.ts6.port, c.ts6.api_key, save_key, inner2, move |st| {
                            emit2(st)
                        });
                    }
                });
            }

            // наблюдение: долго не live — пробуем другой источник
            let mut non_live_since = now_ms();
            loop {
                thread::sleep(Duration::from_millis(500));
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    inner.store(true, std::sync::atomic::Ordering::SeqCst);
                    return;
                }
                if live.load(std::sync::atomic::Ordering::SeqCst) == 1 {
                    non_live_since = now_ms();
                    continue;
                }
                if now_ms().saturating_sub(non_live_since) < HANDOVER_MS {
                    continue;
                }
                let cfg = store.lock().unwrap().get().clone();
                let a = auto::probe_ts3(&cfg.ts3.host, cfg.ts3.port);
                let b = auto::probe_ts6(&cfg.ts6.host, cfg.ts6.port);
                let other = auto::pick_source(a, b);
                if other != winner && a.max(b) >= 2 {
                    diag::diag(
                        "auto",
                        &format!("handover: {} -> {} (source silent >{}s)", name(winner), name(other), HANDOVER_MS / 1000),
                    );
                    inner.store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                non_live_since = now_ms();
            }
        }
    });
}

fn start_connector(store: &Arc<Mutex<ConfigStore>>, app: &Arc<App>) -> Handle {
    let cfg = store.lock().unwrap().get().clone();
    let live = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let emit = throttled_sink(app.clone(), live.clone());
    match cfg.source.as_str() {
        "auto" => {
            let stop = Arc::new(AtomicBool::new(false));
            run_auto_connector(store.clone(), emit, live, stop.clone());
            println!("[server] data source: auto (ts3/ts6 probe)");
            Handle::Auto(stop)
        }
        "ts3" => {
            let stop = Arc::new(AtomicBool::new(false));
            {
                let stop = stop.clone();
                let host = cfg.ts3.host.clone();
                let api_key = cfg.ts3.api_key.clone();
                let port = cfg.ts3.port;
                thread::spawn(move || {
                    ts3::Ts3Connector::run(host, port, api_key, stop, move |st| emit(st));
                });
            }
            println!("[server] data source: ts3 (ClientQuery)");
            Handle::Ts3(stop)
        }
        "ts6" => {
            let save_key: ts6::SaveKeyFn = {
                let store = store.clone();
                Arc::new(move |key: String| {
                    let patch = crate::json::obj(vec![
                        ("ts6", crate::json::obj(vec![("apiKey", Json::s(key))])),
                    ]);
                    // Persist only — a restart is not triggered here (same as TS).
                    let _ = store.lock().unwrap().update(&patch);
                })
            };
            let stop = Arc::new(AtomicBool::new(false));
            {
                let stop = stop.clone();
                let host = cfg.ts6.host.clone();
                let api_key = cfg.ts6.api_key.clone();
                thread::spawn(move || {
                    ts6::run(host, cfg.ts6.port, api_key, save_key, stop, move |st| emit(st));
                });
            }
            println!("[server] data source: ts6");
            Handle::Ts6(stop)
        }
        _ => {
            let conn = mock::MockConnector::start(emit);
            println!("[server] data source: mock");
            Handle::Mock(conn)
        }
    }
}

/// Root folder holding `public/` + `data/`. TSN_ROOT wins; otherwise prefer
/// the exe's own folder (installed to Program Files) and fall back to CWD
/// (development runs).
fn detect_root() -> PathBuf {
    if let Ok(v) = std::env::var("TSN_ROOT") {
        return PathBuf::from(v);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("public").is_dir() {
                return dir.to_path_buf();
            }
        }
    }
    PathBuf::from(".")
}

fn main() {
    let port_env: u16 = std::env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let root = detect_root();
    let data_dir = root.join("data");
    let public_dir = root.join("public");

    let store = Arc::new(Mutex::new(ConfigStore::load(&data_dir)));

    // Run-as-administrator (desktop.elevate, default ON): relaunch elevated.
    // A declined UAC prompt keeps running un-elevated.
    #[cfg(any(windows, tsn_win32))]
    {
        let relaunched = std::env::args().any(|a| a == "--elevated-relaunch");
        if !relaunched && !overlay::win32::acquire_single_instance() {
            // a copy is already running — exit before any UAC prompt
            std::process::exit(0);
        }
        let cfg = store.lock().unwrap().get().clone();
        overlay::win32::maybe_elevate(&cfg);
        if relaunched && !overlay::win32::acquire_single_instance() {
            std::process::exit(0);
        }
    }

    let handle_slot: Arc<Mutex<Handle>> = Arc::new(Mutex::new(Handle::Stub));

    let initial_state = {
        let source = store.lock().unwrap().get().source.clone();
        // 'static strs for the model — map once here
        let source: &'static str = match source.as_str() {
            "ts3" => "ts3",
            "ts6" => "ts6",
            "auto" => "auto",
            _ => "mock",
        };
        OverlayState::empty(source, "connecting", "")
    };

    let app: Arc<App> = Arc::new_cyclic(|weak| App {
        store: store.clone(),
        latest: RwLock::new(initial_state),
        clients: Mutex::new(Vec::new()),
        next_client_id: Mutex::new(0),
        move_mode: AtomicBool::new(false),
        panel_size: Mutex::new(None),
        version: env!("CARGO_PKG_VERSION").to_string(),
        public_dir: public_dir.clone(),
        restart_connector: Arc::new({
            let weak = weak.clone();
            let slot = handle_slot.clone();
            let store = store.clone();
            move || {
                if let Some(app) = weak.upgrade() {
                    // a connector that fails to start must never poison the
                    // slot lock or take the app down with it
                    let mut s = slot.lock().unwrap_or_else(|p| p.into_inner());
                    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.stop()));
                    if stopped.is_err() {
                        eprintln!("[server] connector stop panicked");
                    }
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| start_connector(&store, &app))) {
                        Ok(h) => *s = h,
                        Err(_) => {
                            eprintln!("[server] connector start panicked");
                            *s = Handle::Stub;
                        }
                    }
                    // clear the previous source's panel right away («connecting»)
                    let src = store.lock().unwrap().get().source.clone();
                    let src: &'static str = match src.as_str() {
                        "ts3" => "ts3",
                        "ts6" => "ts6",
                        _ => "mock",
                    };
                    app.push_state(&crate::model::OverlayState::empty(src, "connecting", ""));
                }
            }
        }),
    });

    {
        let mut slot = handle_slot.lock().unwrap();
        *slot = start_connector(&store, &app);
    }

    let port = match http::serve(app.clone(), &host, port_env) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("[server] failed to start: {err}");
            std::process::exit(1);
        }
    };
    println!("[server] TS3-TS6 Notifier running at http://{host}:{port}");
    // Native overlay window (win32 layered window on Windows; headless loop
    // elsewhere). TSN_OVERLAY=0 disables it (tests, server-only deployments).
    if std::env::var("TSN_OVERLAY").map(|v| v != "0").unwrap_or(true) {
        let _overlay = overlay::spawn(app.clone(), port);
        println!("[overlay] native overlay started");
    }
    // «Запуск с системой» — self-heal the HKCU Run entry
    #[cfg(any(windows, tsn_win32))]
    if app.store.lock().unwrap().get().desktop.autostart {
        overlay::win32::set_autostart(true);
    }
    // Автопроверка обновлений: через минуту после старта, затем каждые 24 часа
    {
        let app = app.clone();
        std::thread::Builder::new()
            .name("update-timer".into())
            .spawn(move || {
                thread::sleep(Duration::from_secs(90));
                loop {
                    settings::start_update_check(app.clone());
                    thread::sleep(Duration::from_secs(24 * 60 * 60));
                }
            })
            .ok();
    }
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatKind, ChatMessage, UserState};

    fn msg(text: &str) -> ChatMessage {
        ChatMessage {
            id: "m1".into(),
            user_id: "u1".into(),
            nickname: "N".into(),
            text: text.into(),
            kind: ChatKind::Channel,
            at: 0,
            event: None,
            detail: None,
            own: None,
        }
    }

    #[test]
    fn bare_connecting_inherits_prev_roster() {
        let mut prev = OverlayState::empty("ts3", "live", "");
        prev.users = vec![UserState::new("5", "Лиса")];
        prev.chat = vec![msg("x")];
        prev.server_name = "S".into();

        let mut st = OverlayState::empty("ts6", "connecting", "");
        inherit_roster(Some(&prev), &mut st);
        assert_eq!(st.users.len(), 1);
        assert_eq!(st.users[0].nickname, "Лиса");
        assert_eq!(st.chat.len(), 1);
        assert_eq!(st.server_name, "S");
        // меняется только статус — список людей не мигает
        assert_eq!(st.status, "connecting");
    }

    #[test]
    fn live_state_is_never_overridden() {
        let mut prev = OverlayState::empty("ts3", "live", "");
        prev.users = vec![UserState::new("5", "A")];
        prev.chat = vec![msg("x")];

        let mut st = OverlayState::empty("ts3", "live", "");
        st.users = vec![UserState::new("6", "B")];
        inherit_roster(Some(&prev), &mut st);
        assert_eq!(st.users.len(), 1);
        assert_eq!(st.users[0].nickname, "B");
        assert!(st.chat.is_empty());
    }

    #[test]
    fn first_bare_state_stays_empty() {
        let mut st = OverlayState::empty("ts3", "connecting", "");
        inherit_roster(None, &mut st);
        assert!(st.users.is_empty());
    }
}
