//! HTTP + WebSocket server: serves the web overlay (public/), the /api
//! endpoints and the /ws hub — the same wire protocol as the TS app.

use crate::config::ConfigStore;
use crate::diag::{lines_for, other_lines};
use crate::icons::find_ts3_icon;
use crate::json::{obj, Json};
use crate::model::OverlayState;
use crate::wsframe::{read_frame, ws_accept, write_frame, Frame};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

pub type RestartFn = Arc<dyn Fn() + Send + Sync>;

pub struct App {
    pub store: Arc<Mutex<ConfigStore>>,
    pub latest: RwLock<OverlayState>,
    pub clients: Mutex<Vec<(u64, Sender<String>)>>,
    pub next_client_id: Mutex<u64>,
    pub move_mode: AtomicBool,
    pub panel_size: Mutex<Option<(i64, i64)>>,
    pub version: String,
    pub public_dir: PathBuf,
    pub restart_connector: RestartFn,
    pub plugin_active: Arc<AtomicBool>,
    pub plugin_dll: String,
}

impl App {
    pub fn broadcast(&self, message: &Json) {
        let raw = message.to_string();
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|(_, tx)| tx.send(raw.clone()).is_ok());
    }

    pub fn push_state(&self, state: &OverlayState) {
        *self.latest.write().unwrap() = state.clone();
        self.broadcast(&obj(vec![
            ("type", Json::s("state")),
            ("state", state.to_json()),
        ]));
    }

    pub fn config_json(&self) -> Json {
        self.store.lock().unwrap().get().to_json()
    }
}

/* ---------------- helpers ---------------- */

fn http_response(stream: &mut impl Write, code: u16, ctype: &str, body: &[u8], extra: &str) {
    let status = match code {
        200 => "200 OK",
        204 => "204 No Content",
        400 => "400 Bad Request",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        _ => "200 OK",
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n{extra}\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn json_response(stream: &mut impl Write, code: u16, body: &Json) {
    http_response(stream, code, "application/json; charset=utf-8", body.to_string().as_bytes(), "Cache-Control: no-store\r\n");
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn parse_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).ok()?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; len.min(1_000_000)];
    if !body.is_empty() {
        reader.read_exact(&mut body).ok()?;
    }
    Some(Request { method, path, headers, body })
}

fn body_json(req: &Request) -> Json {
    Json::parse(std::str::from_utf8(&req.body).unwrap_or("")).unwrap_or(Json::Null)
}

/* ---------------- static files ---------------- */

const MIME: &[(&str, &str)] = &[
    (".html", "text/html; charset=utf-8"),
    (".css", "text/css; charset=utf-8"),
    (".js", "text/javascript; charset=utf-8"),
    (".json", "application/json; charset=utf-8"),
    (".svg", "image/svg+xml"),
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".ico", "image/x-icon"),
    (".webp", "image/webp"),
    (".woff2", "font/woff2"),
    (".gif", "image/gif"),
];

fn serve_static(stream: &mut impl Write, url_path: &str, public_dir: &Path) {
    let raw = url_path.split('?').next().unwrap_or("/");
    let segments: Vec<&str> = raw
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect();
    let rel = if segments.is_empty() { "index.html".to_string() } else { segments.join("/") };
    let file_path = public_dir.join(&rel);
    // traversal guard
    if !file_path.starts_with(public_dir) {
        http_response(stream, 403, "text/plain", b"Forbidden", "");
        return;
    }
    match std::fs::read(&file_path) {
        Ok(data) => {
            let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let mime = MIME.iter().find(|(e, _)| *e == format!(".{ext}").as_str()).map(|(_, m)| *m).unwrap_or("application/octet-stream");
            http_response(stream, 200, mime, &data, "Cache-Control: no-cache\r\n");
        }
        Err(_) => http_response(stream, 404, "text/plain", b"Not found", ""),
    }
}

/* ---------------- diagnostics ---------------- */

fn memory_rows() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    let kb: f64 = rest.trim().trim_end_matches(" kB").trim().parse().unwrap_or(0.0);
                    return vec![format!("главный процесс (Rust/сервер): rss={:.1}MB", kb / 1024.0)];
                }
            }
        }
    }
    vec!["главный процесс (Rust/сервер): rss=n/a".into()]
}

fn diagnostics(app: &App) -> String {
    let cfg = app.store.lock().unwrap().get().clone();
    let latest = app.latest.read().unwrap().clone();
    let ts3_lines = lines_for("[ts3]");
    let ts3_plugin: Vec<String> = lines_for("[plugin]");
    let mut ts3_all = ts3_lines.clone();
    ts3_all.extend(ts3_plugin.iter().cloned());
    let ts6_lines = lines_for("[ts6]");
    let other = other_lines();
    let section = |lines: &Vec<String>, active: &str| -> Vec<String> {
        if !lines.is_empty() {
            lines.clone()
        } else {
            vec![if active == "ACTIVE" {
                "(нет данных)".into()
            } else {
                format!("(раздел выключен — активный источник {active})")
            }]
        }
    };
    let mut report: Vec<String> = Vec::new();
    report.push(format!("TS3-TS6 Notifier diagnostics — {}", crate::diag::local_stamp()));
    report.push(format!("version: {}", app.version));
    report.push(format!(
        "source: {} (ts3 {}:{}, ts6 {}:{})",
        cfg.source, cfg.ts3.host, cfg.ts3.port, cfg.ts6.host, cfg.ts6.port
    ));
    report.push(format!("last status: {} — {}", latest.status, latest.detail));
    report.push(format!(
        "users on channel: {}, chat/event rows kept: {}",
        latest.users.len(),
        latest.chat.len()
    ));
    report.push(String::new());
    report.push("--- recent chat & events ---".into());
    for m in latest.chat.iter().take(30) {
        report.push(format!(
            "{} {}{} {}{}{}",
            crate::diag::hhmmss(m.at),
            m.kind.as_str(),
            m.event.map(|e| format!("/{e}")).unwrap_or_default(),
            if m.nickname.is_empty() { String::new() } else { format!("{}: ", m.nickname) },
            m.text,
            m.detail.as_ref().map(|d| format!(" [{d}]")).unwrap_or_default()
        ));
    }
    report.push(String::new());
    report.push("--- TS3 (ClientQuery / plugin) ---".into());
    let upper = cfg.source.to_uppercase();
    let ts3_tag = if cfg.source == "ts3" { "ACTIVE" } else { upper.as_str() };
    report.extend(section(&ts3_all, ts3_tag));
    report.push(String::new());
    report.push("--- TS6 (Remote Apps) ---".into());
    let ts6_tag = if cfg.source == "ts6" { "ACTIVE" } else { upper.as_str() };
    report.extend(section(&ts6_lines, ts6_tag));
    if !other.is_empty() {
        report.push(String::new());
        report.push("--- other ---".into());
        report.extend(other);
    }
    report.push(String::new());
    report.push("--- memory ---".into());
    report.extend(memory_rows());
    report.join("\n")
}

/* ---------------- API routing ---------------- */

fn handle_api(app: &Arc<App>, req: &Request, stream: &mut TcpStream) -> bool {
    let path = req.path.split('?').next().unwrap_or("/");
    let method = req.method.as_str();

    if method == "OPTIONS" {
        let _ = stream.write_all(
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET,PUT,POST,OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return true;
    }

    match (path, method) {
        ("/api/state", "GET") => {
            let latest = app.latest.read().unwrap().clone();
            json_response(stream, 200, &latest.to_json());
            true
        }
        ("/api/config", "GET") => {
            let cfg = app.config_json();
            json_response(stream, 200, &cfg);
            true
        }
        ("/api/config", "PUT") => {
            let patch = body_json(req);
            let restart = app.store.lock().unwrap().update(&patch);
            if restart {
                (app.restart_connector)();
            }
            let cfg = app.config_json();
            app.broadcast(&obj(vec![("type", Json::s("config")), ("config", cfg.clone())]));
            json_response(stream, 200, &obj(vec![("ok", Json::b(true)), ("restart", Json::b(restart)), ("config", cfg)]));
            true
        }
        _ if path.starts_with("/api/tsicon/") && method == "GET" => {
            let id: i64 = path.trim_start_matches("/api/tsicon/").split('?').next().unwrap_or("").parse().unwrap_or(0);
            match find_ts3_icon(id) {
                Some(icon) => {
                    http_response(stream, 200, icon.mime, &icon.data, "Cache-Control: public, max-age=3600\r\n");
                }
                None => http_response(stream, 404, "text/plain", b"Not found", ""),
            }
            true
        }
        ("/api/displays", "GET") => {
            let panel = *app.panel_size.lock().unwrap();
            json_response(stream, 200, &obj(vec![
                ("ok", Json::b(true)),
                ("displays", Json::Arr(vec![])),
                ("panelSize", match panel {
                    Some((w, h)) => obj(vec![("w", Json::i(w)), ("h", Json::i(h))]),
                    None => Json::Null,
                }),
            ]));
            true
        }
        ("/api/reconnect", "POST") => {
            (app.restart_connector)();
            json_response(stream, 200, &obj(vec![("ok", Json::b(true))]));
            true
        }
        ("/api/overlay/move", "POST") => {
            let active = !app.move_mode.load(Ordering::SeqCst);
            app.move_mode.store(active, Ordering::SeqCst);
            app.broadcast(&obj(vec![("type", Json::s("move-mode")), ("active", Json::b(active))]));
            json_response(stream, 200, &obj(vec![("ok", Json::b(true)), ("moveMode", Json::b(active))]));
            true
        }
        ("/api/overlay/moved", "POST") => {
            let b = body_json(req);
            app.broadcast(&obj(vec![
                ("type", Json::s("move-pending")),
                ("marginX", b.get("marginX").cloned().unwrap_or(Json::Null)),
                ("marginY", b.get("marginY").cloned().unwrap_or(Json::Null)),
            ]));
            json_response(stream, 200, &obj(vec![("ok", Json::b(true))]));
            true
        }
        ("/api/overlay/move-confirm", "POST") => {
            app.move_mode.store(false, Ordering::SeqCst);
            app.broadcast(&obj(vec![("type", Json::s("move-confirm"))]));
            json_response(stream, 200, &obj(vec![("ok", Json::b(true))]));
            true
        }
        ("/api/overlay/move-cancel", "POST") => {
            app.move_mode.store(false, Ordering::SeqCst);
            app.broadcast(&obj(vec![("type", Json::s("move-cancel"))]));
            json_response(stream, 200, &obj(vec![("ok", Json::b(true))]));
            true
        }
        ("/api/overlay/size", "POST") => {
            let b = body_json(req);
            let w = b.get("w").and_then(|v| v.as_i64());
            let h = b.get("h").and_then(|v| v.as_i64());
            if let (Some(w), Some(h)) = (w, h) {
                *app.panel_size.lock().unwrap() = Some((w, h));
            }
            json_response(stream, 200, &obj(vec![("ok", Json::b(true))]));
            true
        }
        ("/api/overlay/bounds", "POST") => {
            let b = body_json(req);
            let nums: Vec<Option<f64>> = ["x", "y", "w", "h"].iter().map(|k| b.get(k).and_then(|v| v.as_f64())).collect();
            if nums.iter().all(|n| n.is_some()) {
                // No desktop shell in the native core yet — report no crop.
                json_response(stream, 200, &obj(vec![
                    ("ok", Json::b(true)),
                    ("cropX", Json::i(0)),
                    ("cropY", Json::i(0)),
                    ("screenW", Json::i(0)),
                    ("screenH", Json::i(0)),
                ]));
            } else {
                json_response(stream, 200, &obj(vec![("ok", Json::b(false))]));
            }
            true
        }
        ("/api/plugin/install", "POST") => {
            let result = plugin_install(&app.plugin_dll);
            json_response(stream, 200, &result);
            true
        }
        ("/api/plugin/status", "GET") => {
            json_response(stream, 200, &obj(vec![
                ("active", Json::b(app.plugin_active.load(Ordering::SeqCst))),
                ("dll", Json::b(!app.plugin_dll.is_empty() && Path::new(&app.plugin_dll).exists())),
            ]));
            true
        }
        ("/api/diagnostics", "GET") => {
            let report = diagnostics(app);
            http_response(stream, 200, "text/plain; charset=utf-8", report.as_bytes(), "Cache-Control: no-store\r\n");
            true
        }
        ("/api/update/restart", "POST") => {
            json_response(stream, 200, &obj(vec![("ok", Json::b(true)), ("applied", Json::b(false))]));
            true
        }
        _ if path.starts_with("/api/update/check") && method == "POST" => {
            json_response(stream, 200, &obj(vec![("ok", Json::b(true)), ("applied", Json::b(false))]));
            true
        }
        _ if path.starts_with("/api/overlay/clickthrough") && method == "POST" => {
            json_response(stream, 200, &obj(vec![("ok", Json::b(true)), ("applied", Json::b(false))]));
            true
        }
        _ if path.starts_with("/api/") => {
            json_response(stream, 404, &obj(vec![("ok", Json::b(false)), ("error", Json::s("unknown endpoint"))]));
            true
        }
        _ => false,
    }
}

fn plugin_install(dll: &str) -> Json {
    if !cfg!(windows) {
        return obj(vec![("ok", Json::b(false)), ("error", Json::s("windows-only"))]);
    }
    if dll.is_empty() || !Path::new(dll).exists() {
        return obj(vec![("ok", Json::b(false)), ("error", Json::s("dll-missing"))]);
    }
    #[cfg(any(windows, tsn_win32))]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        let dst_dir = format!("{appdata}\\TS3Client\\plugins");
        let dst = format!("{dst_dir}\\ts3plugin_tsn_win64.dll");
        let _ = std::fs::create_dir_all(&dst_dir);
        // identical → nothing to do
        if let (Ok(a), Ok(b)) = (std::fs::read(dll), std::fs::read(&dst)) {
            if a == b {
                return obj(vec![("ok", Json::b(true)), ("same", Json::b(true)), ("path", Json::s(dst))]);
            }
        }
        // copy; on lock errors rename aside first (Windows maps the DLL)
        if std::fs::copy(dll, &dst).is_err() {
            let old = format!("{dst}.old-{}", crate::model::now_ms());
            if std::fs::rename(&dst, &old).is_err() {
                return obj(vec![("ok", Json::b(false)), ("error", Json::s("locked"))]);
            }
            if std::fs::copy(dll, &dst).is_err() {
                return obj(vec![("ok", Json::b(false)), ("error", Json::s("copy-failed"))]);
            }
        }
        crate::diag::diag("plugin", &format!("installed to {dst}"));
        obj(vec![("ok", Json::b(true)), ("path", Json::s(dst))])
    }
    #[cfg(not(any(windows, tsn_win32)))]
    {
        obj(vec![("ok", Json::b(false)), ("error", Json::s("windows-only"))])
    }
}

/* ---------------- WebSocket ---------------- */

fn ws_handshake(req: &Request, stream: &mut TcpStream) -> bool {
    let Some(key) = req.headers.get("sec-websocket-key") else {
        return false;
    };
    let accept = ws_accept(key);
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(resp.as_bytes()).is_ok()
}

fn ws_client_loop(app: Arc<App>, stream: TcpStream, rx: Receiver<String>) {
    let mut write_stream = stream.try_clone().expect("clone ws stream");
    // hello
    let hello = obj(vec![
        ("type", Json::s("hello")),
        ("config", app.config_json()),
        ("state", app.latest.read().unwrap().clone().to_json()),
        ("moveMode", Json::b(app.move_mode.load(Ordering::SeqCst))),
        ("version", Json::s(app.version.clone())),
    ]);
    if write_frame(&mut write_stream, &Frame::Text(hello.to_string()), false).is_err() {
        return;
    }
    stream.set_read_timeout(Some(Duration::from_millis(200))).ok();
    let mut read_stream = stream.try_clone().expect("clone ws stream");
    loop {
        // outbound
        while let Ok(msg) = rx.try_recv() {
            if write_frame(&mut write_stream, &Frame::Text(msg), false).is_err() {
                let _ = stream.shutdown(Shutdown::Both);
                return;
            }
        }
        // inbound (the app protocol has no client messages; handle ping/close)
        match read_frame(&mut read_stream) {
            Ok(Some(Frame::Ping(p))) => {
                if write_frame(&mut write_stream, &Frame::Pong(p), false).is_err() {
                    break;
                }
            }
            Ok(Some(Frame::Close)) | Ok(None) => break,
            Ok(Some(_)) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

/* ---------------- server loop ---------------- */

pub fn serve(app: Arc<App>, host: &str, preferred_port: u16) -> std::io::Result<u16> {
    let listener = if preferred_port > 0 {
        match TcpListener::bind((host, preferred_port)) {
            Ok(l) => l,
            Err(_) => {
                eprintln!("[server] port {preferred_port} busy — falling back to a random port");
                TcpListener::bind((host, 0))?
            }
        }
    } else {
        TcpListener::bind((host, 0))?
    };
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let app = app.clone();
            thread::spawn(move || handle_conn(app, stream));
        }
    });
    Ok(port)
}

fn handle_conn(app: Arc<App>, stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut stream = stream;
    let Some(req) = parse_request(&mut reader) else {
        return;
    };
    let path = req.path.split('?').next().unwrap_or("/").to_string();

    // websocket upgrade (exact /ws — same as the TS server)
    if path == "/ws"
        && req.headers.get("upgrade").map(|v| v.eq_ignore_ascii_case("websocket")).unwrap_or(false)
    {
        if ws_handshake(&req, &mut stream) {
            let (tx, rx) = channel::<String>();
            let id = {
                let mut next = app.next_client_id.lock().unwrap();
                *next += 1;
                *next
            };
            app.clients.lock().unwrap().push((id, tx));
            ws_client_loop(app.clone(), stream, rx);
            app.clients.lock().unwrap().retain(|(cid, _)| *cid != id);
        }
        return;
    }

    if handle_api(&app, &req, &mut stream) {
        return;
    }
    serve_static(&mut stream, &path, &app.public_dir);
}
