//! TS3 plugin bridge (Overwolf-style) — line-JSON over TCP — plus the
//! PluginState store and the plugin-preferred TS3 connector with ClientQuery
//! fallback.

use crate::chatlog::{sys_row, ChatLog};
use crate::diag::diag;
use crate::json::{obj, Json};
use crate::model::{now_ms, ChatKind, ChatMessage, OverlayState, ServerGroup, UserState};
use crate::ts3::Ts3Connector;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub type Emit = Arc<dyn Fn(OverlayState) + Send + Sync>;

/* ------------------------------------------------------------------ */
/* bridge                                                              */
/* ------------------------------------------------------------------ */

struct BridgeShared {
    sockets: Mutex<Vec<Arc<Mutex<TcpStream>>>>,
    last_seen: Mutex<u64>,
    was_active: AtomicBool,
}

pub struct PluginBridge {
    port: u16,
    idle_after_ms: u64,
    shared: Arc<BridgeShared>,
    stop: Arc<AtomicBool>,
    line_tx: Sender<Json>,
    active_tx: Sender<()>,
    idle_tx: Sender<()>,
}

impl PluginBridge {
    pub fn start(
        port: u16,
        idle_after_ms: u64,
    ) -> std::io::Result<(PluginBridge, Receiver<Json>, Receiver<()>, Receiver<()>)> {
        let (line_tx, line_rx) = channel();
        let (active_tx, active_rx) = channel();
        let (idle_tx, idle_rx) = channel();
        let shared = Arc::new(BridgeShared {
            sockets: Mutex::new(Vec::new()),
            last_seen: Mutex::new(0),
            was_active: AtomicBool::new(false),
        });
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let bound_port = listener.local_addr()?.port();
        diag("plugin", &format!("bridge listening on 127.0.0.1:{bound_port}"));
        let stop = Arc::new(AtomicBool::new(false));

        // accept loop
        {
            let shared = shared.clone();
            let stop = stop.clone();
            let line_tx = line_tx.clone();
            let active_tx = active_tx.clone();
            thread::spawn(move || {
                for conn in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(socket) = conn else { continue };
                    let _ = socket.set_nodelay(true);
                    let shared = shared.clone();
                    let line_tx = line_tx.clone();
                    let active_tx = active_tx.clone();
                    let stop = stop.clone();
                    thread::spawn(move || {
                        handle_plugin_conn(socket, shared, line_tx, active_tx, stop);
                    });
                }
            });
        }
        // idle watcher
        {
            let shared = shared.clone();
            let stop = stop.clone();
            let idle_tx = idle_tx.clone();
            thread::spawn(move || {
                let tick = Duration::from_millis(idle_after_ms.min(5000));
                while !stop.load(Ordering::SeqCst) {
                    thread::sleep(tick);
                    let active = {
                        let last = *shared.last_seen.lock().unwrap();
                        shared.was_active.load(Ordering::SeqCst) && now_ms() - last < idle_after_ms
                    };
                    if shared.was_active.load(Ordering::SeqCst) && !active {
                        shared.was_active.store(false, Ordering::SeqCst);
                        if idle_tx.send(()).is_err() {
                            break;
                        }
                    }
                }
            });
        }

        Ok((
            PluginBridge { port: bound_port, idle_after_ms, shared, stop, line_tx, active_tx, idle_tx },
            line_rx,
            active_rx,
            idle_rx,
        ))
    }

    pub fn active(&self) -> bool {
        let last = *self.shared.last_seen.lock().unwrap();
        self.shared.was_active.load(Ordering::SeqCst) && now_ms() - last < self.idle_after_ms
    }

    pub fn send(&self, msg: &Json) {
        let raw = format!("{}\n", msg.to_string());
        let mut sockets = self.shared.sockets.lock().unwrap();
        sockets.retain(|s| {
            match s.lock() {
                Ok(mut guard) => guard.write_all(raw.as_bytes()).is_ok(),
                Err(_) => false,
            }
        });
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let sockets = self.shared.sockets.lock().unwrap();
        for s in sockets.iter() {
            if let Ok(guard) = s.lock() {
                let _ = guard.shutdown(Shutdown::Both);
            }
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

fn handle_plugin_conn(
    socket: TcpStream,
    shared: Arc<BridgeShared>,
    line_tx: Sender<Json>,
    active_tx: Sender<()>,
    stop: Arc<AtomicBool>,
) {
    let writer = Arc::new(Mutex::new(socket.try_clone().expect("clone socket")));
    shared.sockets.lock().unwrap().push(writer);
    let reader = BufReader::new(socket);
    for line in reader.lines() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(line) = line else { break };
        if line.len() > 1_000_000 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // heartbeats keep the link marked alive
        *shared.last_seen.lock().unwrap() = now_ms();
        if !shared.was_active.swap(true, Ordering::SeqCst) {
            let _ = active_tx.send(());
        }
        let Ok(msg) = Json::parse(trimmed) else {
            diag("plugin", &format!("bad line: {}", &trimmed[..trimmed.len().min(120)]));
            continue;
        };
        if msg.get("e").map(|v| v.loose_str()).as_deref() == Some("ping") {
            continue;
        }
        diag("plugin", &trimmed[..trimmed.len().min(240)]);
        if line_tx.send(msg).is_err() {
            break;
        }
    }
    // drop the writer entry
    {
        let mut sockets = shared.sockets.lock().unwrap();
        sockets.retain(|s| {
            s.lock()
                .map(|g| g.peer_addr().is_ok())
                .unwrap_or(false)
        });
    }
}

/* ------------------------------------------------------------------ */
/* PluginState                                                         */
/* ------------------------------------------------------------------ */

pub struct PluginState {
    clients: HashMap<i64, UserState>,
    chat: ChatLog,
    own_id: i64,
    own_cid: i64,
    own_channel_name: String,
    channel_names: HashMap<i64, String>,
    server_name: String,
    groups: Vec<ServerGroup>,
    emit: Emit,
}

impl PluginState {
    pub fn new(emit: Emit) -> PluginState {
        PluginState {
            clients: HashMap::new(),
            chat: ChatLog::new('p'),
            own_id: 0,
            own_cid: 0,
            own_channel_name: String::new(),
            channel_names: HashMap::new(),
            server_name: String::new(),
            groups: Vec::new(),
            emit,
        }
    }

    pub fn ingest(&mut self, msg: &Json) {
        let e = msg.get("e").map(|v| v.loose_str()).unwrap_or_default();
        match e.as_str() {
            "snapshot" => {
                let prev = std::mem::take(&mut self.clients);
                if let Some(s) = msg.get("server").map(|v| v.loose_str()) {
                    if !s.is_empty() {
                        self.server_name = s;
                    }
                }
                self.own_id = msg.get("own").map(|v| v.loose_i64()).unwrap_or(0);
                self.own_cid = msg.get("owncid").map(|v| v.loose_i64()).unwrap_or(0);
                let mut clients = HashMap::new();
                let mut channel_names = HashMap::new();
                if let Some(list) = msg.get("clients").and_then(|v| v.as_arr()) {
                    for raw in list {
                        let id = raw.get("id").map(|v| v.loose_i64()).unwrap_or(0);
                        if id <= 0 {
                            continue;
                        }
                        let cid = raw.get("cid").map(|v| v.loose_i64()).unwrap_or(0);
                        if let Some(cn) = raw.get("cname").map(|v| v.loose_str()) {
                            if !cn.is_empty() {
                                channel_names.insert(cid, cn);
                            }
                        }
                        let mut u = UserState::new(id.to_string(), raw.get("name").map(|v| v.loose_str()).unwrap_or_default());
                        u.channel_id = cid;
                        u.is_self = id == self.own_id;
                        u.server_groups = raw.get("groups").map(|v| v.loose_str()).unwrap_or_default();
                        clients.insert(id, u);
                    }
                }
                self.channel_names = channel_names;
                self.own_channel_name = self
                    .channel_names
                    .get(&self.own_cid)
                    .cloned()
                    .unwrap_or_else(|| msg.get("owncname").map(|v| v.loose_str()).unwrap_or_default());
                // Diff-announce (v1.4.7 flood rule): only small deltas, never
                // the empty→populated baseline jump.
                let added: Vec<i64> = clients.keys().copied().filter(|id| !prev.contains_key(id)).collect();
                let removed: Vec<i64> = prev.keys().copied().filter(|id| !clients.contains_key(id)).collect();
                let mass = |n: usize| (n as f64) > 5f64.max(prev.len() as f64 * 0.3);
                if !prev.is_empty() && !clients.is_empty() && !mass(added.len()) && !mass(removed.len()) {
                    for id in &added {
                        if let Some(u) = clients.get(id) {
                            if !u.is_self && !u.is_query && !u.nickname.is_empty() {
                                let own = u.channel_id == self.own_cid;
                                self.push_event(&u.nickname.clone(), &u.id.clone(), "join", own, "");
                            }
                        }
                    }
                    for id in &removed {
                        if let Some(u) = prev.get(id) {
                            if !u.is_self && !u.is_query && !u.nickname.is_empty() {
                                let own = u.channel_id == self.own_cid;
                                self.push_event(&u.nickname.clone(), &u.id.clone(), "leave", own, "");
                            }
                        }
                    }
                }
                self.clients = clients;
                self.emit_state();
            }
            "join" => {
                let id = msg.get("clid").map(|v| v.loose_i64()).unwrap_or(0);
                let cid = msg.get("cid").map(|v| v.loose_i64()).unwrap_or(0);
                let cname = msg.get("cname").map(|v| v.loose_str()).unwrap_or_default();
                let mut u = UserState::new(id.to_string(), msg.get("name").map(|v| v.loose_str()).unwrap_or_default());
                u.channel_id = cid;
                u.is_self = id == self.own_id;
                u.server_groups = msg.get("groups").map(|v| v.loose_str()).unwrap_or_default();
                if !cname.is_empty() {
                    self.channel_names.insert(cid, cname.clone());
                }
                if id == self.own_id {
                    self.own_cid = cid;
                    self.own_channel_name = if cname.is_empty() {
                        self.channel_names.get(&cid).cloned().unwrap_or_else(|| self.own_channel_name.clone())
                    } else {
                        cname.clone()
                    };
                }
                if !u.is_self && !u.is_query && !u.nickname.is_empty() {
                    let own = u.channel_id == self.own_cid;
                    self.push_event(&u.nickname.clone(), &u.id.clone(), "join", own, "");
                }
                self.clients.insert(id, u);
                self.emit_state();
            }
            "leave" => {
                let id = msg.get("clid").map(|v| v.loose_i64()).unwrap_or(0);
                let prev = self.clients.remove(&id);
                let nick = msg
                    .get("name")
                    .map(|v| v.loose_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| prev.as_ref().map(|u| u.nickname.clone()).unwrap_or_default());
                let was_ours = prev.as_ref().map(|u| u.channel_id == self.own_cid).unwrap_or(false);
                let is_query = prev.as_ref().map(|u| u.is_query).unwrap_or(false);
                let is_self = prev.as_ref().map(|u| u.is_self).unwrap_or(false);
                if !nick.is_empty() && !is_query && !is_self && id != self.own_id {
                    let uid = prev.as_ref().map(|u| u.id.clone()).unwrap_or_else(|| id.to_string());
                    self.push_event(&nick, &uid, "leave", was_ours, "");
                }
                self.emit_state();
            }
            "move" => {
                let id = msg.get("clid").map(|v| v.loose_i64()).unwrap_or(0);
                let to = msg.get("to").map(|v| v.loose_i64()).unwrap_or(0);
                let from = msg.get("from").map(|v| v.loose_i64()).unwrap_or(0);
                let groups = msg.get("groups").map(|v| v.loose_str()).unwrap_or_default();
                let cname = msg.get("cname").map(|v| v.loose_str()).unwrap_or_default();
                let prev = self.clients.get(&id).cloned();
                let nick = msg
                    .get("name")
                    .map(|v| v.loose_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| prev.as_ref().map(|u| u.nickname.clone()).unwrap_or_default());
                let mut u = prev.unwrap_or_else(|| {
                    let mut u = UserState::new(id.to_string(), nick.clone());
                    u.is_self = id == self.own_id;
                    u
                });
                u.channel_id = to;
                if !groups.is_empty() {
                    u.server_groups = groups;
                }
                if !cname.is_empty() {
                    self.channel_names.insert(to, cname.clone());
                }
                if id == self.own_id {
                    self.own_cid = to;
                    self.own_channel_name = if cname.is_empty() {
                        self.channel_names.get(&to).cloned().unwrap_or_else(|| self.own_channel_name.clone())
                    } else {
                        cname.clone()
                    };
                }
                if !u.is_query && !nick.is_empty() {
                    let detail = if cname.is_empty() {
                        self.channel_names.get(&to).cloned().unwrap_or_default()
                    } else {
                        cname.clone()
                    };
                    let own = u.is_self || from == self.own_cid || to == self.own_cid;
                    self.push_event(&nick, &u.id.clone(), "move", own, &detail);
                }
                self.clients.insert(id, u);
                self.emit_state();
            }
            "talk" => {
                let id = msg.get("clid").map(|v| v.loose_i64()).unwrap_or(0);
                let s = msg.get("s").map(|v| v.loose_i64()).unwrap_or(0);
                // v2 plugin sends "w" (isReceivedWhisper); s==2 = whispering in
                // the TS3 SDK — support both so the whisper section works.
                let whisper = msg.get("w").map(|v| v.loose_i64()).unwrap_or(0) != 0 || s == 2;
                if let Some(u) = self.clients.get_mut(&id) {
                    u.talking = s == 1 || whisper;
                    u.whispering = whisper;
                    self.emit_state();
                }
            }
            "chat" => {
                let kind_raw = msg.get("kind").map(|v| v.loose_str()).unwrap_or_default();
                let kind = match kind_raw.as_str() {
                    "private" => ChatKind::Private,
                    "server" => ChatKind::Server,
                    _ => ChatKind::Channel,
                };
                let clid = msg.get("clid").map(|v| v.loose_i64()).unwrap_or(0);
                let name = msg.get("name").map(|v| v.loose_str()).unwrap_or_default();
                self.chat.push(ChatMessage {
                    id: String::new(),
                    user_id: if clid != 0 { clid.to_string() } else { name.clone() },
                    nickname: if name.is_empty() { "?".into() } else { name },
                    text: msg.get("text").map(|v| v.loose_str()).unwrap_or_default(),
                    kind,
                    at: 0,
                    event: None,
                    detail: None,
                    own: None,
                });
                self.emit_state();
            }
            "poke" => {
                let clid = msg.get("clid").map(|v| v.loose_i64()).unwrap_or(0);
                let name = msg.get("name").map(|v| v.loose_str()).unwrap_or_default();
                let text = msg.get("text").map(|v| v.loose_str()).unwrap_or_default();
                self.chat.push(ChatMessage {
                    id: String::new(),
                    user_id: if clid != 0 { clid.to_string() } else { name.clone() },
                    nickname: if name.is_empty() { "Кто-то".into() } else { name.clone() },
                    text: if text.is_empty() { name.clone() } else { text.clone() },
                    kind: ChatKind::Poke,
                    at: 0,
                    event: Some("poke"),
                    detail: if text.is_empty() { None } else { Some(text) },
                    own: None,
                });
                self.emit_state();
            }
            "client" => {
                let id = msg.get("clid").map(|v| v.loose_i64()).unwrap_or(0);
                let name = msg.get("name").map(|v| v.loose_str()).unwrap_or_default();
                let groups = msg.get("groups").map(|v| v.loose_str()).unwrap_or_default();
                let Some(prev) = self.clients.get(&id).cloned() else { return };
                let mut u = prev.clone();
                if !name.is_empty() {
                    u.nickname = name;
                }
                if !groups.is_empty() {
                    u.server_groups = groups.clone();
                }
                if !groups.is_empty() && groups != prev.server_groups {
                    let detail = groups.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                    let own = prev.channel_id == self.own_cid;
                    self.chat.push(ChatMessage {
                        id: String::new(),
                        user_id: id.to_string(),
                        nickname: prev.nickname.clone(),
                        text: "server".into(),
                        kind: ChatKind::System,
                        at: 0,
                        event: Some("group"),
                        detail: Some(if detail.is_empty() { "—".into() } else { detail }),
                        own: Some(own),
                    });
                }
                self.clients.insert(id, u);
                self.emit_state();
            }
            "disconnect" => {
                self.clients.clear();
                self.channel_names.clear();
                self.own_cid = 0;
                self.own_channel_name = String::new();
                self.emit_state();
            }
            _ => {} // hello / pong
        }
    }

    fn push_event(&mut self, nick: &str, uid: &str, event: &'static str, own: bool, detail: &str) {
        self.chat.push(sys_row(uid, nick, nick, event, detail, own));
    }

    pub fn build_state(&self) -> OverlayState {
        let mut st = OverlayState::empty("ts3", "live", "TS3 plugin");
        st.server_name = self.server_name.clone();
        st.channel_name = if self.own_channel_name.is_empty() {
            self.channel_names.get(&self.own_cid).cloned().unwrap_or_default()
        } else {
            self.own_channel_name.clone()
        };
        st.channel_id = if self.own_cid > 0 { self.own_cid.to_string() } else { String::new() };
        st.self_id = if self.own_id > 0 { Some(self.own_id.to_string()) } else { None };
        st.users = self
            .clients
            .values()
            .filter(|c| self.own_cid > 0 && c.channel_id == self.own_cid)
            .cloned()
            .collect();
        st.groups = self.groups.clone();
        st.chat = self.chat.rows().to_vec();
        st.updated_at = now_ms();
        st
    }

    fn emit_state(&mut self) {
        (self.emit)(self.build_state());
    }
}

/* ------------------------------------------------------------------ */
/* connector: plugin first, ClientQuery fallback                        */
/* ------------------------------------------------------------------ */

enum ConnEvent {
    Line(Json),
    Active,
    Idle,
    FallbackTimer,
    QueryState(Box<OverlayState>),
}

pub struct Ts3WithPluginConnector {
    stop: Arc<AtomicBool>,
    plugin_active: Arc<AtomicBool>,
    bridge_port: u16,
}

impl Ts3WithPluginConnector {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        host: &str,
        port: i64,
        api_key: &str,
        bridge_port: u16,
        idle_after_ms: u64,
        fallback_delay_ms: u64,
        emit: Emit,
    ) -> Ts3WithPluginConnector {
        let stop = Arc::new(AtomicBool::new(false));
        let plugin_active = Arc::new(AtomicBool::new(false));
        let (bridge, line_rx, active_rx, idle_rx) =
            PluginBridge::start(bridge_port, idle_after_ms).expect("plugin bridge bind");
        let (evt_tx, evt_rx) = channel::<ConnEvent>();

        forward(line_rx, evt_tx.clone(), ConnEvent::Line, stop.clone());
        forward(active_rx, evt_tx.clone(), |_| ConnEvent::Active, stop.clone());
        forward(idle_rx, evt_tx.clone(), |_| ConnEvent::Idle, stop.clone());

        // lazy fallback timer (v1.4.6: give the plugin a head start so both
        // data paths are never active at once)
        {
            let tx = evt_tx.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(fallback_delay_ms));
                if !stop.load(Ordering::SeqCst) {
                    let _ = tx.send(ConnEvent::FallbackTimer);
                }
            });
        }

        {
            let stop = stop.clone();
            let plugin_active = plugin_active.clone();
            let host = host.to_string();
            let api_key = api_key.to_string();
            let evt_tx = evt_tx.clone();
            thread::spawn(move || {
                machine_loop(evt_rx, evt_tx, bridge, host, port, api_key, emit, stop, plugin_active);
            });
        }

        Ts3WithPluginConnector { stop, plugin_active, bridge_port: bridge_port }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn bridge_active(&self) -> bool {
        self.plugin_active.load(Ordering::SeqCst)
    }

    pub fn bridge_port(&self) -> u16 {
        self.bridge_port
    }
}

fn forward<T: Send + 'static>(
    rx: Receiver<T>,
    tx: Sender<ConnEvent>,
    map: impl Fn(T) -> ConnEvent + Send + 'static,
    stop: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(v) => {
                    if tx.send(map(v)).is_err() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
    });
}

fn ensure_query(
    query_stop: &mut Option<Arc<AtomicBool>>,
    evt_tx: &Sender<ConnEvent>,
    host: &str,
    port: i64,
    api_key: &str,
    stop: &Arc<AtomicBool>,
) {
    if query_stop.is_some() || stop.load(Ordering::SeqCst) {
        return;
    }
    let qstop = Arc::new(AtomicBool::new(false));
    *query_stop = Some(qstop.clone());
    let tx = evt_tx.clone();
    let host = host.to_string();
    let api_key = api_key.to_string();
    thread::spawn(move || {
        Ts3Connector::run(host, port, api_key, qstop, move |st| {
            let _ = tx.send(ConnEvent::QueryState(Box::new(st)));
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn machine_loop(
    evt_rx: Receiver<ConnEvent>,
    evt_tx: Sender<ConnEvent>,
    bridge: PluginBridge,
    host: String,
    port: i64,
    api_key: String,
    emit: Emit,
    stop: Arc<AtomicBool>,
    plugin_active: Arc<AtomicBool>,
) {
    let mut using_plugin = false;
    let mut plugin_state: Option<PluginState> = None;
    let mut hold_until: u64 = 0;
    let mut query_live = false;
    let mut query_stop: Option<Arc<AtomicBool>> = None;

    while !stop.load(Ordering::SeqCst) {
        let Ok(ev) = evt_rx.recv_timeout(Duration::from_millis(500)) else { continue };
        match ev {
            ConnEvent::Active => {
                if !using_plugin {
                    using_plugin = true;
                    plugin_active.store(true, Ordering::SeqCst);
                    if let Some(s) = query_stop.take() {
                        s.store(true, Ordering::SeqCst);
                    }
                    plugin_state = Some(PluginState::new(emit.clone()));
                    println!("[ts3] plugin bridge active — ClientQuery connector paused");
                    bridge.send(&obj(vec![("cmd", Json::s("snapshot"))]));
                }
            }
            ConnEvent::Line(msg) => {
                if using_plugin {
                    if let Some(ps) = plugin_state.as_mut() {
                        ps.ingest(&msg);
                    }
                }
            }
            ConnEvent::Idle => {
                if using_plugin {
                    using_plugin = false;
                    plugin_active.store(false, Ordering::SeqCst);
                    hold_until = now_ms() + 6000;
                    query_live = false;
                    println!("[ts3] plugin bridge idle — falling back to ClientQuery");
                }
                ensure_query(&mut query_stop, &evt_tx, &host, port, &api_key, &stop);
            }
            ConnEvent::FallbackTimer => {
                if !using_plugin {
                    ensure_query(&mut query_stop, &evt_tx, &host, port, &api_key, &stop);
                }
            }
            ConnEvent::QueryState(st) => {
                if using_plugin {
                    continue;
                }
                if st.status == "live" {
                    query_live = true;
                }
                if !query_live {
                    if let Some(ps) = plugin_state.as_ref() {
                        if now_ms() < hold_until {
                            emit(ps.build_state());
                            continue;
                        }
                    }
                }
                plugin_state = None;
                emit(*st);
            }
        }
    }
    bridge.stop();
    if let Some(s) = query_stop.take() {
        s.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn capture() -> (Emit, Arc<Mutex<Vec<OverlayState>>>) {
        let out: Arc<Mutex<Vec<OverlayState>>> = Arc::new(Mutex::new(Vec::new()));
        let out2 = out.clone();
        let emit: Emit = Arc::new(move |st| out2.lock().unwrap().push(st));
        (emit, out)
    }

    fn snapshot_line() -> Json {
        Json::parse(
            r#"{"e":"snapshot","server":"Eternals","own":100,"owncid":7,"owncname":"Home","clients":[
                {"id":100,"name":"Я","cid":7,"groups":"442","cname":"Home"},
                {"id":101,"name":"А","cid":7,"groups":"","cname":"Home"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn snapshot_roster_and_whisper() {
        let (emit, out) = capture();
        let mut ps = PluginState::new(emit);
        ps.ingest(&snapshot_line());
        ps.ingest(&Json::parse(r#"{"e":"talk","clid":101,"s":1,"w":1,"schid":1}"#).unwrap());
        let st = ps.build_state();
        assert_eq!(st.server_name, "Eternals");
        assert_eq!(st.channel_name, "Home");
        assert_eq!(st.channel_id, "7");
        assert_eq!(st.users.len(), 2);
        let a = st.users.iter().find(|u| u.nickname == "А").unwrap();
        assert!(a.talking && a.whispering);
        assert!(out.lock().unwrap().len() >= 2);
    }

    #[test]
    fn own_move_reanchors_channel() {
        let (emit, _out) = capture();
        let mut ps = PluginState::new(emit);
        ps.ingest(&snapshot_line());
        ps.ingest(&Json::parse(r#"{"e":"move","clid":100,"from":7,"to":9,"name":"Я","groups":"442","cname":"AFK"}"#).unwrap());
        let st = ps.build_state();
        assert_eq!(st.channel_id, "9");
        assert_eq!(st.channel_name, "AFK");
        assert_eq!(st.users.len(), 1); // only Я on the new channel
    }

    #[test]
    fn flood_rule_silent_on_mass_resync() {
        let (emit, out) = capture();
        let mut ps = PluginState::new(emit);
        ps.ingest(&snapshot_line());
        let _ = ps.build_state();
        let before = out.lock().unwrap().len();
        // mass snapshot: 9 additions out of 2 previous → over the 30%/5 cap
        let mut clients = String::from(
            r#"[{"id":100,"name":"Я","cid":7,"groups":"442","cname":"Home"},{"id":101,"name":"А","cid":7,"groups":"","cname":"Home"}"#,
        );
        for i in 0..9 {
            clients.push_str(&format!(r#",{{"id":{},"name":"X{}","cid":7,"groups":"","cname":"Home"}}"#, 200 + i, i));
        }
        clients.push(']');
        let big = Json::parse(&format!(
            r#"{{"e":"snapshot","server":"Eternals","own":100,"owncid":7,"owncname":"Home","clients":{clients}}}"#
        ))
        .unwrap();
        ps.ingest(&big);
        let after = out.lock().unwrap().len();
        // only the state emit, no chat rows added
        let st = ps.build_state();
        assert_eq!(st.chat.len(), 0, "mass snapshot must not announce joins: {:?}", st.chat);
        let _ = (before, after);
    }
}
