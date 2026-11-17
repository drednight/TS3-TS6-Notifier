//! TS3-TS6 Notifier — native core: TeamSpeak connectors, state model and the
//! HTTP/WS server for the web overlay. Drop-in replacement for the Node
//! server (same config.json, same wire protocol).

mod assets;
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
mod theme;
mod ts3;
mod ts6;
mod wire;
mod wsframe;

use config::ConfigStore;
use http::App;
use json::Json;
use model::{now_ms, OverlayState};
use plugin::{Emit, Ts3WithPluginConnector};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

enum Handle {
    Ts3Plugin(Ts3WithPluginConnector),
    Mock(mock::MockConnector),
    Ts6(Arc<AtomicBool>),
    Stub,
}

impl Handle {
    fn stop(&self) {
        match self {
            Handle::Ts3Plugin(c) => c.stop(),
            Handle::Mock(c) => c.stop(),
            Handle::Ts6(stop) => stop.store(true, std::sync::atomic::Ordering::SeqCst),
            Handle::Stub => {}
        }
    }
}

/// Coalesce state bursts into at most ~10 broadcasts/second (same as the TS
/// connectors' emitState throttle).
fn throttled_sink(app: Arc<App>) -> Emit {
    let (tx, rx) = channel::<OverlayState>();
    thread::spawn(move || {
        let mut pending: Option<OverlayState> = None;
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
            if elapsed < 100 {
                thread::sleep(Duration::from_millis(100 - elapsed));
                while let Ok(newer) = rx.try_recv() {
                    pending = Some(newer);
                }
            }
            let st = pending.take().unwrap_or(st);
            last_sent = now_ms();
            app.push_state(&st);
        }
    });
    Arc::new(move |st| {
        let _ = tx.send(st);
    })
}

fn start_connector(store: &Arc<Mutex<ConfigStore>>, app: &Arc<App>) -> Handle {
    let cfg = store.lock().unwrap().get().clone();
    let emit = throttled_sink(app.clone());
    match cfg.source.as_str() {
        "ts3" => {
            let conn = Ts3WithPluginConnector::start(
                &cfg.ts3.host,
                cfg.ts3.port,
                &cfg.ts3.api_key,
                48912,  // plugin bridge port
                45000,  // plugin idle timeout
                4000,   // lazy ClientQuery fallback delay
                emit,
            );
            app.plugin_active.store(conn.bridge_active(), std::sync::atomic::Ordering::SeqCst);
            println!("[server] data source: ts3");
            Handle::Ts3Plugin(conn)
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

fn main() {
    let port_env: u16 = std::env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let root = PathBuf::from(std::env::var("TSN_ROOT").unwrap_or_else(|_| ".".into()));
    let data_dir = root.join("data");
    let public_dir = root.join("public");

    let store = Arc::new(Mutex::new(ConfigStore::load(&data_dir)));
    let handle_slot: Arc<Mutex<Handle>> = Arc::new(Mutex::new(Handle::Stub));

    let plugin_dll = root.join("dist").join("ts3plugin_tsn_win64.dll");
    let plugin_dll = if plugin_dll.exists() {
        plugin_dll.to_string_lossy().into_owned()
    } else {
        String::new()
    };

    let initial_state = {
        let source = store.lock().unwrap().get().source.clone();
        // 'static strs for the model — map once here
        let source: &'static str = match source.as_str() {
            "ts3" => "ts3",
            "ts6" => "ts6",
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
        plugin_active: Arc::new(AtomicBool::new(false)),
        plugin_dll: plugin_dll.clone(),
        restart_connector: Arc::new({
            let weak = weak.clone();
            let slot = handle_slot.clone();
            let store = store.clone();
            move || {
                if let Some(app) = weak.upgrade() {
                    let mut s = slot.lock().unwrap();
                    s.stop();
                    *s = start_connector(&store, &app);
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
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}
