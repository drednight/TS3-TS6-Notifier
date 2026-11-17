//! TeamSpeak 3 data source via the built-in ClientQuery plugin (TCP telnet,
//! default 127.0.0.1:25639). Port of the TS Ts3Connector.

use crate::chatlog::{sys_row, ChatLog};
use crate::diag::diag;
use crate::model::{now_ms, ChatKind, ChatMessage, OverlayState, ServerGroup, UserState};
use crate::wire::{f_bool, f_get, f_has, f_int, f_str, ts3_escape, ts3_parse_fields, Ts3Fields};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub type EmitFn = Box<dyn Fn(OverlayState) + Send>;

enum Job {
    /// raw command + optional response sink (None = fire and forget)
    Cmd(String, Option<Sender<Result<Ts3Response, String>>>),
    Refresh,
}

struct ChannelInfo {
    #[allow(dead_code)]
    id: i64,
    #[allow(dead_code)]
    parent_id: i64,
    name: String,
}

/// Read the TS3 ClientQuery API key from clientquery.ini (same locations).
pub fn read_ts3_api_key() -> String {
    let mut candidates: Vec<String> = Vec::new();
    if cfg!(windows) {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        candidates.push(format!("{appdata}\\TS3Client\\clientquery.ini"));
    } else {
        if let Ok(home) = std::env::var("HOME") {
            candidates.push(format!("{home}/.ts3client/clientquery.ini"));
            candidates.push(format!("{home}/snap/teamspeak3/current/.ts3client/clientquery.ini"));
        }
    }
    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("api_key") {
                    let rest = rest.trim_start().strip_prefix('=').unwrap_or("").trim();
                    let val = rest.trim_matches(|c| c == '"' || c == '\'');
                    if !val.is_empty() {
                        return val.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

pub struct Ts3Response {
    pub items: Vec<Ts3Fields>,
    pub error_id: i64,
    pub error_message: String,
}

struct Ts3Session {
    host: String,
    port: i64,
    api_key: String,
    stop: Arc<AtomicBool>,
    emit: EmitFn,

    clients: HashMap<i64, UserState>,
    group_mem: HashMap<i64, (String, String)>,
    announced_once: bool,
    channels: HashMap<i64, ChannelInfo>,
    groups: Vec<ServerGroup>,
    chat: ChatLog,
    own_clid_by_handler: HashMap<i64, i64>,
    own_clid: i64,
    own_cid: i64,
    server_name: String,
    active_schandler_id: i64,
    cur_status: &'static str,
    cur_detail: String,
    auth_failed: bool,
    greeting_seen: bool,
    refresh_fails: i32,
    talk_diag_skipped: i32,
    refresh_pending_at: u64,
}

impl Ts3Session {
    fn publish(&mut self, status: &'static str, detail: &str) {
        self.cur_status = status;
        self.cur_detail = detail.to_string();
        self.emit_state();
    }

    fn emit_state(&mut self) {
        (self.emit)(self.build_state());
    }

    fn build_state(&self) -> OverlayState {
        let mut st = OverlayState::empty("ts3", self.cur_status, &self.cur_detail);
        st.server_name = self.server_name.clone();
        st.channel_name = self.channels.get(&self.own_cid).map(|c| c.name.clone()).unwrap_or_default();
        st.channel_id = if self.own_cid > 0 { self.own_cid.to_string() } else { String::new() };
        st.self_id = if self.own_clid > 0 { Some(self.own_clid.to_string()) } else { None };
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

    fn push_chat(&mut self, msg: ChatMessage) {
        self.chat.push(msg);
        self.emit_state();
    }

    /* ---------------- event handling ---------------- */

    fn handle_event(&mut self, line: &str) {
        let space = line.find(' ').unwrap_or(line.len());
        let name = &line[..space];
        // diagnostics: talk flaps sampled 1-in-10
        if name == "notifytalkstatuschange" {
            self.talk_diag_skipped += 1;
            if self.talk_diag_skipped % 10 == 1 {
                let suffix = if self.talk_diag_skipped > 1 { " …" } else { "" };
                diag("ts3", &format!("{}{}", &line[..line.len().min(240)], suffix));
            }
        } else {
            diag("ts3", &line[..line.len().min(240)]);
        }
        let fields = ts3_parse_fields(if space < line.len() { &line[space + 1..] } else { "" });
        let schandlerid = f_int(&fields, "schandlerid");
        let in_scope = schandlerid <= 0 || schandlerid == self.active_schandler_id;

        match name {
            "notifytalkstatuschange" => {
                let clid = f_int(&fields, "clid");
                let talking = f_int(&fields, "status") == 1;
                let whisper = talking && ((f_has(&fields, "targetmode") && f_int(&fields, "targetmode") != 0) || f_bool(&fields, "whisper"));
                if !in_scope {
                    if talking && schandlerid > 0 && self.own_clid_by_handler.get(&schandlerid) == Some(&clid) {
                        self.active_schandler_id = schandlerid;
                        self.schedule_refresh();
                    }
                    return;
                }
                if let Some(user) = self.clients.get_mut(&clid) {
                    if user.talking != talking || user.whispering != whisper {
                        user.talking = talking;
                        user.whispering = whisper;
                        self.emit_state();
                    }
                }
            }
            "notifytextmessage" => {
                // chat is accepted from every server tab
                let target_mode = f_int(&fields, "targetmode");
                let kind = match target_mode {
                    1 => ChatKind::Private,
                    3 => ChatKind::Server,
                    _ => ChatKind::Channel,
                };
                let text = f_str(&fields, "msg");
                if text.is_empty() {
                    return;
                }
                let user_id_num = f_int(&fields, "invokerid");
                let user_id = if user_id_num != 0 { user_id_num.to_string() } else { f_str(&fields, "clid") };
                let nickname = {
                    let invoker = f_str(&fields, "invokername");
                    if !invoker.is_empty() {
                        invoker
                    } else {
                        self.clients
                            .get(&user_id.parse::<i64>().unwrap_or(0))
                            .map(|u| u.nickname.clone())
                            .unwrap_or_else(|| "?".into())
                    }
                };
                self.push_chat(ChatMessage {
                    id: String::new(),
                    user_id,
                    nickname,
                    text,
                    kind,
                    at: 0,
                    event: None,
                    detail: None,
                    own: None,
                });
            }
            "notifyclientupdated" => {
                if !in_scope {
                    return;
                }
                let clid = f_int(&fields, "clid");
                let Some(user) = self.clients.get(&clid).cloned() else { return };
                let mut u = user.clone();
                if f_has(&fields, "client_nickname") {
                    u.nickname = f_str(&fields, "client_nickname");
                }
                if f_has(&fields, "client_input_muted") || f_has(&fields, "client_is_muted") {
                    u.input_muted = f_bool(&fields, "client_input_muted") || f_bool(&fields, "client_is_muted");
                }
                if f_has(&fields, "client_output_muted") {
                    u.output_muted = f_bool(&fields, "client_output_muted");
                }
                if f_has(&fields, "client_away") {
                    u.away = f_bool(&fields, "client_away");
                }
                if f_has(&fields, "client_away_message") {
                    u.away_message = f_str(&fields, "client_away_message");
                }
                if f_has(&fields, "client_is_priority_speaker") {
                    u.priority_speaker = f_bool(&fields, "client_is_priority_speaker");
                }
                if f_has(&fields, "client_flag_talking") {
                    u.talking = f_bool(&fields, "client_flag_talking");
                }
                self.clients.insert(clid, u.clone());
                // group membership diffs → sys events
                let sg = if f_has(&fields, "client_servergroups") { Some(f_str(&fields, "client_servergroups")) } else { None };
                let cg = if f_has(&fields, "client_channel_group_id") {
                    let name = f_str(&fields, "client_channel_group_name");
                    Some(if name.is_empty() { f_int(&fields, "client_channel_group_id").to_string() } else { name })
                } else {
                    None
                };
                let prev = self.group_mem.get(&clid).cloned().unwrap_or_default();
                if let Some(ref sg) = sg {
                    if *sg != prev.0 {
                        let detail = sg.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                        let own = user.channel_id == self.own_cid;
                        self.push_chat(sys_row(&clid.to_string(), &user.nickname, "server", "group", &detail, own).with_fallback_detail("—"));
                    }
                }
                if let Some(ref cg) = cg {
                    if *cg != prev.1 {
                        let own = user.channel_id == self.own_cid;
                        self.push_chat(sys_row(&clid.to_string(), &user.nickname, "channel", "group", &cg, own));
                    }
                }
                self.group_mem.insert(clid, (sg.unwrap_or(prev.0), cg.unwrap_or(prev.1)));
                self.emit_state();
            }
            "notifyclientmoved" => {
                if !in_scope {
                    return;
                }
                let clid = f_int(&fields, "clid");
                let to = f_int(&fields, "ctid");
                let Some(user) = self.clients.get(&clid).cloned() else {
                    self.schedule_refresh();
                    return;
                };
                let detail = self.channels.get(&to).map(|c| c.name.clone()).unwrap_or_default();
                if user.is_self {
                    self.own_cid = to;
                    self.push_chat(sys_row(&clid.to_string(), &user.nickname, &user.nickname, "move", &detail, true));
                } else if !user.is_query && clid != self.own_clid {
                    // v1.3.1: every move on the server
                    let own = to == self.own_cid || user.channel_id == self.own_cid;
                    self.push_chat(sys_row(&clid.to_string(), &user.nickname, &user.nickname, "move", &detail, own));
                }
                self.schedule_refresh();
            }
            "notifyclientleftview" => {
                if !in_scope {
                    return;
                }
                let clid = f_int(&fields, "clid");
                let user = self.clients.get(&clid).cloned();
                let nick = {
                    let n = f_str(&fields, "client_nickname");
                    if n.is_empty() { user.as_ref().map(|u| u.nickname.clone()).unwrap_or_default() } else { n }
                };
                let was_ours = user.as_ref().map(|u| u.channel_id == self.own_cid).unwrap_or(false);
                if !nick.is_empty() && !user.as_ref().map(|u| u.is_query).unwrap_or(false) && clid != self.own_clid {
                    let detail = if was_ours {
                        user.as_ref().and_then(|u| self.channels.get(&u.channel_id)).map(|c| c.name.clone()).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    self.push_chat(sys_row(&clid.to_string(), &nick, &nick, "leave", &detail, was_ours));
                }
                if self.clients.remove(&clid).is_some() {
                    self.emit_state();
                }
                self.schedule_refresh();
            }
            "notifycliententerview" => {
                if !in_scope {
                    return;
                }
                let clid = f_int(&fields, "clid");
                let nick = f_str(&fields, "client_nickname");
                let is_query = f_int(&fields, "client_type") == 1;
                if !nick.is_empty() && !is_query && clid != self.own_clid {
                    let ctid = f_int(&fields, "ctid");
                    let in_our = ctid > 0 && ctid == self.own_cid;
                    let detail = if in_our { self.channels.get(&ctid).map(|c| c.name.clone()).unwrap_or_default() } else { String::new() };
                    self.push_chat(sys_row(&clid.to_string(), &nick, &nick, "join", &detail, in_our));
                }
                self.schedule_refresh();
            }
            "notifyclientpoke" => {
                if !in_scope {
                    return;
                }
                let nick = {
                    let n = f_str(&fields, "invokername");
                    if n.is_empty() { "Кто-то".into() } else { n }
                };
                let msg = f_str(&fields, "msg");
                let text = if msg.is_empty() { nick.clone() } else { msg.clone() };
                self.push_chat(ChatMessage {
                    id: String::new(),
                    user_id: f_int(&fields, "invokerid").to_string(),
                    nickname: nick,
                    text,
                    kind: ChatKind::Poke,
                    at: 0,
                    event: Some("poke"),
                    detail: if msg.is_empty() { None } else { Some(msg) },
                    own: None,
                });
            }
            "notifychannelcreated" | "notifychanneledited" | "notifychanneldeleted" | "notifychannelmoved"
            | "notifyserverupdated" | "notifyserveredited" | "notifyconnectstatuschange" => {
                if in_scope {
                    self.schedule_refresh();
                }
            }
            "notifycurrentserverconnectionchanged" => {
                let hid = f_int(&fields, "schandlerid");
                if hid > 0 && hid != self.active_schandler_id {
                    self.active_schandler_id = hid;
                    self.schedule_refresh();
                }
            }
            _ => {}
        }
    }

    fn schedule_refresh(&mut self) {
        self.refresh_pending_at = now_ms() + 600;
    }
}

impl ChatMessage {
    fn with_fallback_detail(mut self, fallback: &str) -> ChatMessage {
        if self.detail.as_deref().unwrap_or("").is_empty() {
            self.detail = Some(fallback.to_string());
        }
        self
    }
}

/* ---------------- connection plumbing ---------------- */

struct Link {
    stream: TcpStream,
    lines: Receiver<String>,
    _reader: thread::JoinHandle<()>,
}

fn open_link(host: &str, port: i64) -> Result<Link, String> {
    let addr = format!("{host}:{port}");
    let stream = TcpStream::connect_timeout(
        &addr.parse().map_err(|e| format!("bad address: {e}"))?,
        Duration::from_millis(5000),
    )
    .map_err(|e| format!("ClientQuery connect: {e}"))?;
    let _ = stream.set_nodelay(true);
    let reader_stream = stream.try_clone().map_err(|e| e.to_string())?;
    let (tx, rx) = channel::<String>();
    let reader = thread::spawn(move || {
        let mut buf = BufReader::new(reader_stream);
        let mut line = String::new();
        loop {
            line.clear();
            match buf.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.len() > 1_000_000 {
                        break;
                    }
                    let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
                    if tx.send(trimmed).is_err() {
                        break;
                    }
                }
            }
        }
    });
    Ok(Link { stream, lines: rx, _reader: reader })
}

fn split_glued(line: &str) -> Vec<String> {
    // Some builds glue banner + first response — split at embedded "error id=".
    if let Some(idx) = line.find("error id=") {
        if idx > 0 {
            let mut out = vec![line[..idx].to_string()];
            out.extend(split_glued(&line[idx..]));
            return out;
        }
    }
    vec![line.to_string()]
}

impl Ts3Session {
    /// Send a command and collect the response (sequential command model).
    fn command(&mut self, link: &mut Link, cmd: &str, timeout_ms: u64) -> Result<Ts3Response, String> {
        let full = format!("{cmd}\n");
        link.stream.write_all(full.as_bytes()).map_err(|e| format!("write: {e}"))?;
        let deadline = now_ms() + timeout_ms;
        let mut data_lines: Vec<String> = Vec::new();
        loop {
            let remaining = deadline.saturating_sub(now_ms());
            if remaining == 0 {
                return Err(format!("ClientQuery command timeout: {}", cmd.split(' ').next().unwrap_or("")));
            }
            match link.lines.recv_timeout(Duration::from_millis(remaining.min(200))) {
                Ok(line) => {
                    for piece in split_glued(&line) {
                        if piece.is_empty() {
                            continue;
                        }
                        if piece.starts_with("TS3 Client") || piece.starts_with("selected schandlerid=") {
                            self.greeting_seen = true;
                        }
                        if piece.starts_with("notify") {
                            self.handle_event(&piece);
                            continue;
                        }
                        if piece.starts_with("selected schandlerid=") {
                            continue;
                        }
                        if piece.starts_with("error id=") {
                            let fields = ts3_parse_fields(&piece);
                            let mut resp = Ts3Response {
                                items: Vec::new(),
                                error_id: f_int(&fields, "id"),
                                error_message: f_str(&fields, "msg"),
                            };
                            if !data_lines.is_empty() {
                                let joined = data_lines.join(" ");
                                for item in joined.split('|') {
                                    if !item.trim().is_empty() {
                                        resp.items.push(ts3_parse_fields(item.trim()));
                                    }
                                }
                            }
                            return Ok(resp);
                        }
                        data_lines.push(piece);
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err("connection closed".into()),
            }
        }
    }

    fn safe(&mut self, link: &mut Link, cmd: &str, timeout_ms: u64) -> Ts3Response {
        match self.command(link, cmd, timeout_ms) {
            Ok(r) => r,
            Err(msg) => Ts3Response { items: Vec::new(), error_id: -1, error_message: msg },
        }
    }

    fn try_auth(&mut self, link: &mut Link) -> &'static str {
        let cmd = format!("auth apikey={}", ts3_escape(&self.api_key));
        match self.command(link, &cmd, 8000) {
            Ok(r) => {
                if r.error_id == 0 {
                    "ok"
                } else {
                    "error"
                }
            }
            Err(msg) => {
                if msg.contains("timeout") {
                    "timeout"
                } else {
                    "error"
                }
            }
        }
    }

    fn probe_link(&mut self, link: &mut Link) -> bool {
        match self.command(link, "whoami", 5000) {
            Ok(_) => true,
            Err(msg) => !msg.contains("timeout"),
        }
    }

    fn find_active_handler(&mut self, link: &mut Link) {
        let list = self.safe(link, "serverconnectionhandlerlist", 8000);
        let mut ids: Vec<i64> = list.items.iter().map(|i| f_int(i, "schandlerid")).filter(|n| *n > 0).collect();
        if ids.is_empty() {
            ids.push(1);
        }
        self.own_clid_by_handler.clear();
        let fallback = *ids.first().unwrap_or(&1);
        for hid in ids {
            let use_res = self.safe(link, &format!("use schandlerid={hid}"), 8000);
            if use_res.error_id != 0 {
                continue;
            }
            let who = self.safe(link, "whoami", 8000);
            if who.error_id != 0 || who.items.is_empty() {
                continue;
            }
            let clid = f_int(&who.items[0], "clid");
            if clid <= 0 {
                continue;
            }
            self.own_clid_by_handler.insert(hid, clid);
            let hw = self.safe(link, &format!("clientvariable clid={clid} client_input_hardware"), 8000);
            if let Some(item) = hw.items.first() {
                if f_bool(item, "client_input_hardware") {
                    self.active_schandler_id = hid;
                    return;
                }
            }
        }
        self.active_schandler_id = fallback;
        self.safe(link, &format!("use schandlerid={fallback}"), 8000);
    }

    fn full_refresh(&mut self, link: &mut Link) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        self.safe(link, &format!("use schandlerid={}", self.active_schandler_id), 8000);
        let mut who = self.safe(link, "whoami", 8000);
        if who.error_id != 0 || who.items.is_empty() {
            self.find_active_handler(link);
            self.safe(link, &format!("use schandlerid={}", self.active_schandler_id), 8000);
            who = self.safe(link, "whoami", 8000);
        }
        if who.error_id != 0 || who.items.is_empty() {
            self.clients.clear();
            self.own_clid = 0;
            self.own_cid = 0;
            self.publish("offline", "not connected to a TeamSpeak server");
            return;
        }
        let self_item = who.items[0].clone();
        self.own_clid = f_int(&self_item, "clid");
        self.own_cid = f_int(&self_item, "cid");
        self.own_clid_by_handler.insert(self.active_schandler_id, self.own_clid);

        let server = self.safe(link, "servervariable virtualserver_name", 8000);
        if let Some(item) = server.items.first() {
            self.server_name = f_str(item, "virtualserver_name");
        }

        let ch_res = self.safe(link, "channellist", 20000);
        self.channels.clear();
        for item in &ch_res.items {
            let id = f_int(item, "cid");
            if id <= 0 {
                continue;
            }
            self.channels.insert(id, ChannelInfo { id, parent_id: f_int(item, "pid"), name: f_str(item, "channel_name") });
        }

        let mut cl_res = self.safe(link, "clientlist -uid -away -voice -groups", 20000);
        if cl_res.error_id != 0 {
            cl_res = self.safe(link, "clientlist -uid -away -voice", 20000);
        }

        let mut clients = HashMap::new();
        for item in &cl_res.items {
            let id = f_int(item, "clid");
            if id <= 0 {
                continue;
            }
            let input_muted = f_bool(item, "client_input_muted")
                || (f_get(item, "client_input_hardware").is_some() && !f_bool(item, "client_input_hardware"))
                || f_bool(item, "client_is_muted");
            let output_muted = f_bool(item, "client_output_muted")
                || (f_get(item, "client_output_hardware").is_some() && !f_bool(item, "client_output_hardware"));
            let mut u = UserState::new(id.to_string(), f_str(item, "client_nickname"));
            u.channel_id = f_int(item, "cid");
            u.talking = f_bool(item, "client_flag_talking");
            u.input_muted = input_muted;
            u.output_muted = output_muted;
            u.away = f_bool(item, "client_away");
            u.away_message = f_str(item, "client_away_message");
            u.priority_speaker = f_bool(item, "client_is_priority_speaker");
            u.channel_commander = f_bool(item, "client_is_channel_commander");
            u.is_self = id == self.own_clid;
            u.is_query = f_int(item, "client_type") == 1;
            u.server_groups = f_str(item, "client_servergroups");
            clients.insert(id, u);
        }

        self.read_avatars(link, &mut clients);

        // diff-announce missed joins/leaves (after the first silent baseline)
        if self.announced_once {
            for (id, u) in &clients {
                if self.clients.contains_key(id) || u.is_query || u.is_self || u.nickname.is_empty() {
                    continue;
                }
                let detail = if u.channel_id == self.own_cid {
                    self.channels.get(&u.channel_id).map(|c| c.name.clone()).unwrap_or_default()
                } else {
                    String::new()
                };
                self.chat.push(sys_row(&id.to_string(), &u.nickname, &u.nickname, "join", &detail, u.channel_id == self.own_cid));
            }
            for (id, u) in &self.clients {
                if clients.contains_key(id) || u.is_query || u.is_self || u.nickname.is_empty() {
                    continue;
                }
                let detail = if u.channel_id == self.own_cid {
                    self.channels.get(&u.channel_id).map(|c| c.name.clone()).unwrap_or_default()
                } else {
                    String::new()
                };
                self.chat.push(sys_row(&id.to_string(), &u.nickname, &u.nickname, "leave", &detail, u.channel_id == self.own_cid));
            }
        } else {
            self.announced_once = true;
        }
        self.clients = clients;

        let gr_res = self.safe(link, "servergrouplist", 8000);
        if gr_res.error_id == 0 {
            self.groups = gr_res
                .items
                .iter()
                .map(|i| ServerGroup { id: f_int(i, "sgid"), name: f_str(i, "name"), icon_id: f_int(i, "iconid") })
                .collect();
        }

        self.refresh_fails = 0;
        self.publish("live", "");
    }

    fn read_avatars(&mut self, link: &mut Link, clients: &mut HashMap<i64, UserState>) {
        let ids: Vec<i64> = clients.keys().copied().take(240).collect();
        for chunk in ids.chunks(40) {
            let blocks: Vec<String> = chunk.iter().map(|id| format!("clid={id} client_flag_avatar")).collect();
            let res = self.safe(link, &format!("clientvariable {}", blocks.join("|")), 8000);
            for item in &res.items {
                let id = f_int(item, "clid");
                let hash = f_str(item, "client_flag_avatar");
                if let Some(u) = clients.get_mut(&id) {
                    if !hash.is_empty() {
                        u.avatar_hash = hash;
                    }
                }
            }
        }
    }

    /* ---------------- main session loop ---------------- */

    fn run_session(&mut self, link: &mut Link) -> Result<(), String> {
        // greeting
        let deadline = now_ms() + 2500;
        while !self.greeting_seen && now_ms() < deadline {
            match link.lines.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => {
                    for piece in split_glued(&line) {
                        if piece.starts_with("TS3 Client") || piece.starts_with("selected schandlerid=") {
                            self.greeting_seen = true;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err("connection closed".into()),
            }
        }

        if self.api_key.is_empty() {
            let auto = read_ts3_api_key();
            if !auto.is_empty() {
                self.api_key = auto;
            }
        }

        if !self.api_key.is_empty() {
            let mut auth = self.try_auth(link);
            if auth == "timeout" {
                thread::sleep(Duration::from_millis(700));
                auth = self.try_auth(link);
            }
            if auth == "error" {
                self.auth_failed = true;
                return Err("ClientQuery auth failed — неверный API key (проверьте настройки)".into());
            }
            if auth == "timeout" && !self.probe_link(link) {
                return Err(
                    "ClientQuery не отвечает. Перезапустите TeamSpeak 3 и проверьте плагин Client Query (Настройки → Плагины)"
                        .into(),
                );
            }
        } else if !self.probe_link(link) {
            return Err(
                "ClientQuery не отвечает. Перезапустите TeamSpeak 3 и проверьте плагин Client Query (Настройки → Плагины)"
                    .into(),
            );
        }

        self.command(link, "clientnotifyregister schandlerid=0 event=any", 8000)?;
        self.find_active_handler(link);
        self.full_refresh(link);
        self.refresh_fails = 0;

        // event + timer loop
        let mut next_refresh = now_ms() + 45_000;
        let mut next_keepalive = now_ms() + 20_000;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            let now = now_ms();
            if self.refresh_pending_at > 0 && now >= self.refresh_pending_at {
                self.refresh_pending_at = 0;
                self.full_refresh(link);
            }
            if now >= next_refresh {
                next_refresh = now + 45_000;
                self.full_refresh(link);
            }
            if now >= next_keepalive {
                next_keepalive = now + 20_000;
                self.safe(link, "version", 6000);
            }
            match link.lines.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    for piece in split_glued(&line) {
                        if piece.is_empty() {
                            continue;
                        }
                        if piece.starts_with("notify") {
                            self.handle_event(&piece);
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err("connection closed".into()),
            }
        }
    }
}

pub struct Ts3Connector;

impl Ts3Connector {
    /// Blocking run: connects, maintains and emits states until `stop`.
    pub fn run(host: String, port: i64, api_key: String, stop: Arc<AtomicBool>, emit: impl Fn(OverlayState) + Send + 'static) {
        let emit: EmitFn = Box::new(emit);
        let mut session = Ts3Session {
            host,
            port,
            api_key,
            stop: stop.clone(),
            emit,
            clients: HashMap::new(),
            group_mem: HashMap::new(),
            announced_once: false,
            channels: HashMap::new(),
            groups: Vec::new(),
            chat: ChatLog::new('c'),
            own_clid_by_handler: HashMap::new(),
            own_clid: 0,
            own_cid: 0,
            server_name: String::new(),
            active_schandler_id: 1,
            cur_status: "connecting",
            cur_detail: String::new(),
            auth_failed: false,
            greeting_seen: false,
            refresh_fails: 0,
            talk_diag_skipped: 0,
            refresh_pending_at: 0,
        };

        let mut attempt = 0i32;
        while !stop.load(Ordering::SeqCst) {
            session.greeting_seen = false;
            session.auth_failed = false;
            session.publish("connecting", "");
            match open_link(&session.host, session.port) {
                Ok(mut link) => match session.run_session(&mut link) {
                    Ok(()) => return,
                    Err(msg) => {
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        session.publish(if session.auth_failed { "waiting-auth" } else { "offline" }, &msg);
                    }
                },
                Err(msg) => {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    session.publish(if session.auth_failed { "waiting-auth" } else { "offline" }, &msg);
                }
            }
            attempt += 1;
            let delay = if session.auth_failed {
                15000
            } else {
                3000 * 2i64.pow((attempt - 1).min(4) as u32)
            };
            let delay = delay.min(30000) as u64;
            let wake = now_ms() + delay;
            while !stop.load(Ordering::SeqCst) && now_ms() < wake {
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
}
