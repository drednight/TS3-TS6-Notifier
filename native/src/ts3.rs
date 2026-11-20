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

pub type EmitFn = crate::model::Emit;

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
    refresh_heavy: bool,
    refresh_first_at: u64,
    refresh_stat_at: u64,
    refresh_stat_calls: u32,
    refresh_stat_heavy: u32,
    refresh_stat_ms: u64,
    refresh_stat_max: u64,
    whispers: Arc<crate::model::WhisperBook>,
}

impl Ts3Session {
    fn publish(&mut self, status: &'static str, detail: &str) {
        self.cur_status = status;
        self.cur_detail = detail.to_string();
        self.emit_state();
    }

    fn emit_state(&mut self) {
        crate::model::WhisperBook::publish(&self.whispers, self.build_state());
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
        // Outside whisperers (another channel) — v1.4.12 parity.
        let roster: std::collections::HashSet<String> = st.users.iter().map(|u| u.id.clone()).collect();
        let own = self.own_clid.to_string();
        let ids = self.whispers.ids(&own, &|id| roster.contains(id));
        st.whisperers = ids
            .iter()
            .filter_map(|id| {
                let key: i64 = id.parse().unwrap_or(0);
                let base = self.clients.get(&key).cloned();
                let cname = base
                    .as_ref()
                    .and_then(|u| self.channels.get(&u.channel_id))
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                crate::model::outside_whisperer(base, id, cname)
            })
            .collect();
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
                diag("ts3", &format!("{}{}", crate::diag::snip(line, 240), suffix));
            }
        } else {
            diag("ts3", crate::diag::snip(line, 240));
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
                        self.schedule_refresh(true);
                    }
                    return;
                }
                if let Some(user) = self.clients.get_mut(&clid) {
                    if user.talking != talking || user.whispering != whisper {
                        user.talking = talking;
                        user.whispering = whisper;
                        self.emit_state();
                    }
                } else if talking && clid != 0 && clid != self.own_clid {
                    // Рецензия 2026-11: talk для неизвестного клиента раньше
                    // отбрасывался — говорящий не светился до следующего
                    // refresh. Заглушка светится сейчас, имя долечит refresh.
                    let mut u = UserState::new(clid.to_string(), clid.to_string());
                    u.channel_id = self.own_cid;
                    u.talking = true;
                    u.whispering = whisper;
                    self.clients.insert(clid, u);
                    self.schedule_refresh(false);
                    self.emit_state();
                }
                // A whisper may come from another channel — note it even when
                // the speaker never appears on the roster.
                if clid != self.own_clid {
                    crate::model::WhisperBook::note(&self.whispers, &clid.to_string(), whisper);
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
                    self.schedule_refresh(false);
                    return;
                };
                let detail = self.channels.get(&to).map(|c| c.name.clone()).unwrap_or_default();
                let from = user.channel_id;
                if user.is_self {
                    self.own_cid = to;
                    self.push_chat(sys_row(&clid.to_string(), &user.nickname, &user.nickname, "move", &detail, true));
                } else if !user.is_query && clid != self.own_clid {
                    // v1.3.1: every move on the server
                    let own = to == self.own_cid || from == self.own_cid;
                    self.push_chat(sys_row(&clid.to_string(), &user.nickname, &user.nickname, "move", &detail, own));
                }
                // Рецензия 2026-11: move обновляем локально, refresh не нужен.
                if let Some(u) = self.clients.get_mut(&clid) {
                    u.channel_id = to;
                }
                self.emit_state();
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
            }
            "notifycliententerview" => {
                if !in_scope {
                    return;
                }
                let clid = f_int(&fields, "clid");
                let nick = f_str(&fields, "client_nickname");
                let is_query = f_int(&fields, "client_type") == 1;
                let ctid = f_int(&fields, "ctid");
                // Рецензия 2026-11: enterview несёт достаточно полей — клиент
                // создаётся/обновляется локально, без тяжёлого refresh.
                let is_new = !self.clients.contains_key(&clid);
                let mut u = self
                    .clients
                    .get(&clid)
                    .cloned()
                    .unwrap_or_else(|| UserState::new(clid.to_string(), nick.clone()));
                if !nick.is_empty() {
                    u.nickname = nick.clone();
                }
                if ctid > 0 {
                    u.channel_id = ctid;
                }
                u.is_query = is_query;
                u.is_self = clid == self.own_clid;
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
                if f_has(&fields, "client_flag_talking") {
                    u.talking = f_bool(&fields, "client_flag_talking");
                }
                if f_has(&fields, "client_is_priority_speaker") {
                    u.priority_speaker = f_bool(&fields, "client_is_priority_speaker");
                }
                if f_has(&fields, "client_is_channel_commander") {
                    u.channel_commander = f_bool(&fields, "client_is_channel_commander");
                }
                if f_has(&fields, "client_servergroups") {
                    let sg = f_str(&fields, "client_servergroups");
                    if !sg.is_empty() {
                        u.server_groups = sg;
                    }
                }
                if f_has(&fields, "client_flag_avatar") {
                    let hash = f_str(&fields, "client_flag_avatar");
                    if !hash.is_empty() {
                        u.avatar_hash = hash;
                    }
                }
                self.clients.insert(clid, u);
                if is_new && !nick.is_empty() && !is_query && clid != self.own_clid {
                    let in_our = ctid > 0 && ctid == self.own_cid;
                    let detail = if in_our { self.channels.get(&ctid).map(|c| c.name.clone()).unwrap_or_default() } else { String::new() };
                    self.push_chat(sys_row(&clid.to_string(), &nick, &nick, "join", &detail, in_our));
                }
                self.emit_state();
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
                    self.schedule_refresh(true);
                }
            }
            "notifycurrentserverconnectionchanged" => {
                let hid = f_int(&fields, "schandlerid");
                if hid > 0 && hid != self.active_schandler_id {
                    self.active_schandler_id = hid;
                    self.schedule_refresh(true);
                }
            }
            _ => {}
        }
    }

    /// Замер длительности full_refresh'ей + строка в диагностику раз в минуту.
    fn note_refresh_stats(&mut self, dt: u64, heavy: bool) {
        self.refresh_stat_calls += 1;
        self.refresh_stat_heavy += if heavy { 1 } else { 0 };
        self.refresh_stat_ms += dt;
        self.refresh_stat_max = self.refresh_stat_max.max(dt);
        let now = now_ms();
        if now.saturating_sub(self.refresh_stat_at) > 60_000 {
            if self.refresh_stat_calls > 0 {
                diag(
                    "ts3",
                    &format!(
                        "refresh: {} calls/min ({} heavy), total {} ms, max {} ms",
                        self.refresh_stat_calls, self.refresh_stat_heavy, self.refresh_stat_ms, self.refresh_stat_max
                    ),
                );
            }
            self.refresh_stat_at = now;
            self.refresh_stat_calls = 0;
            self.refresh_stat_heavy = 0;
            self.refresh_stat_ms = 0;
            self.refresh_stat_max = 0;
        }
    }

    /// Поставить refresh в очередь. `heavy` = полный набор (channellist,
    /// группы, сервер, аватары новых); иначе — лёгкий (use/whoami/clientlist).
    /// Дебаунс без голодания: шаг 250 мс, но не позже 1.5 с от ПЕРВОГО события.
    fn schedule_refresh(&mut self, heavy: bool) {
        let now = now_ms();
        if self.refresh_pending_at == 0 {
            self.refresh_first_at = now;
        }
        self.refresh_heavy |= heavy;
        self.refresh_pending_at = (now + 250).min(self.refresh_first_at + 1500);
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
        let list = self.safe(link, "serverconnectionhandlerlist", 2500);
        let mut ids: Vec<i64> = list.items.iter().map(|i| f_int(i, "schandlerid")).filter(|n| *n > 0).collect();
        if ids.is_empty() {
            ids.push(1);
        }
        self.own_clid_by_handler.clear();
        let fallback = *ids.first().unwrap_or(&1);
        for hid in ids {
            let use_res = self.safe(link, &format!("use schandlerid={hid}"), 2500);
            if use_res.error_id != 0 {
                continue;
            }
            let who = self.safe(link, "whoami", 2500);
            if who.error_id != 0 || who.items.is_empty() {
                continue;
            }
            let clid = f_int(&who.items[0], "clid");
            if clid <= 0 {
                continue;
            }
            self.own_clid_by_handler.insert(hid, clid);
            let hw = self.safe(link, &format!("clientvariable clid={clid} client_input_hardware"), 2500);
            if let Some(item) = hw.items.first() {
                if f_bool(item, "client_input_hardware") {
                    self.active_schandler_id = hid;
                    return;
                }
            }
        }
        self.active_schandler_id = fallback;
        self.safe(link, &format!("use schandlerid={fallback}"), 2500);
    }

    /// Лёгкий refresh: ровно то, что нужно, чтобы показать людей.
    fn light_refresh(&mut self, link: &mut Link) {
        self.refresh(link, false);
    }

    /// Тяжёлый refresh: полный набор (channellist, группы, сервер, аватары).
    fn full_refresh(&mut self, link: &mut Link) {
        self.refresh(link, true);
    }

    /// Сплит light/heavy (ревью 2026-11, пункты 1–2):
    ///  * light — use/whoami/clientlist -voice -away, этого хватает для ростера;
    ///  * heavy — дополнительно servervariable/channellist/-groups/аватары
    ///    новых клиентов/servergrouplist: на старте, по событиям channel*/
    ///    server* и раз в несколько минут.
    fn refresh(&mut self, link: &mut Link, heavy: bool) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let t0 = now_ms();
        self.safe(link, &format!("use schandlerid={}", self.active_schandler_id), 2500);
        let mut who = self.safe(link, "whoami", 2500);
        if who.error_id != 0 || who.items.is_empty() {
            self.find_active_handler(link);
            self.safe(link, &format!("use schandlerid={}", self.active_schandler_id), 2500);
            who = self.safe(link, "whoami", 2500);
        }
        if who.error_id != 0 || who.items.is_empty() {
            self.clients.clear();
            self.own_clid = 0;
            self.own_cid = 0;
            self.publish("offline", "not connected to a TeamSpeak server");
            // Рецензия 2026-11: раньше повтор случался только по событию или
            // через 45 с — «offline» мог висеть минутами. Пробуем снова быстро.
            self.schedule_refresh(false);
            self.note_refresh_stats(now_ms().saturating_sub(t0), heavy);
            return;
        }
        let self_item = who.items[0].clone();
        self.own_clid = f_int(&self_item, "clid");
        self.own_cid = f_int(&self_item, "cid");
        self.own_clid_by_handler.insert(self.active_schandler_id, self.own_clid);

        if heavy {
            let server = self.safe(link, "servervariable virtualserver_name", 2500);
            if let Some(item) = server.items.first() {
                self.server_name = f_str(item, "virtualserver_name");
            }

            let ch_res = self.safe(link, "channellist", 3000);
            self.channels.clear();
            for item in &ch_res.items {
                let id = f_int(item, "cid");
                if id <= 0 {
                    continue;
                }
                self.channels.insert(id, ChannelInfo { id, parent_id: f_int(item, "pid"), name: f_str(item, "channel_name") });
            }
        }

        // light: только голос/отсутствие — ровно то, что светится в ростере.
        let cl_res = if heavy {
            let r = self.safe(link, "clientlist -uid -away -voice -groups", 3000);
            if r.error_id != 0 {
                self.safe(link, "clientlist -uid -away -voice", 3000)
            } else {
                r
            }
        } else {
            self.safe(link, "clientlist -voice -away", 3000)
        };

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
            if let Some(prev_u) = self.clients.get(&id) {
                // Рецензия 2026-11: гонка «снимок затирает события» — пока
                // шёл clientlist/аватары, talk/mute-события уже обновили
                // клиентов; старый снимок не должен их откатывать.
                u.talking = prev_u.talking;
                u.whispering = prev_u.whispering;
                // light-запрос не тянет -groups/аватары — переносим из прежнего
                // состояния, чтобы иконки не гасли между heavy-refresh'ами.
                if !f_has(item, "client_servergroups") || u.server_groups.is_empty() {
                    u.server_groups = prev_u.server_groups.clone();
                }
                if !f_has(item, "client_is_priority_speaker") {
                    u.priority_speaker = prev_u.priority_speaker;
                }
                if !f_has(item, "client_is_channel_commander") {
                    u.channel_commander = prev_u.channel_commander;
                }
                if !f_has(item, "client_away_message") {
                    u.away_message = prev_u.away_message.clone();
                }
                u.avatar_hash = prev_u.avatar_hash.clone();
            }
            clients.insert(id, u);
        }

        // Аватары — только для НОВЫХ клиентов (ревью: не гонять clientvariable
        // пачками на каждого). Старые хеши перенесены выше.
        if heavy {
            let new_ids: Vec<i64> = clients.keys().copied().filter(|id| !self.clients.contains_key(id)).take(240).collect();
            self.read_avatars(link, &mut clients, &new_ids);
        }

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

        if heavy {
            let gr_res = self.safe(link, "servergrouplist", 2500);
            if gr_res.error_id == 0 {
                self.groups = gr_res
                    .items
                    .iter()
                    .map(|i| ServerGroup { id: f_int(i, "sgid"), name: f_str(i, "name"), icon_id: f_int(i, "iconid") })
                    .collect();
            }
        }

        self.refresh_fails = 0;
        self.publish("live", "");
        self.note_refresh_stats(now_ms().saturating_sub(t0), heavy);
    }

    fn read_avatars(&mut self, link: &mut Link, clients: &mut HashMap<i64, UserState>, ids: &[i64]) {
        for chunk in ids.chunks(40) {
            let blocks: Vec<String> = chunk.iter().map(|id| format!("clid={id} client_flag_avatar")).collect();
            let res = self.safe(link, &format!("clientvariable {}", blocks.join("|")), 2500);
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

        self.command(link, "clientnotifyregister schandlerid=0 event=any", 2500)?;
        self.find_active_handler(link);
        self.full_refresh(link);
        self.refresh_fails = 0;

        // event + timer loop
        let mut next_refresh = now_ms() + 45_000; // лёгкая страховка
        let mut next_heavy = now_ms() + 300_000; // полный набор раз в 5 мин
        let mut next_keepalive = now_ms() + 20_000;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            let now = now_ms();
            if self.refresh_pending_at > 0 && now >= self.refresh_pending_at {
                let heavy = self.refresh_heavy;
                self.refresh_pending_at = 0;
                self.refresh_heavy = false;
                self.refresh(link, heavy);
            }
            if now >= next_refresh {
                next_refresh = now + 45_000;
                self.light_refresh(link);
            }
            if now >= next_heavy {
                next_heavy = now + 300_000;
                self.full_refresh(link);
            }
            if now >= next_keepalive {
                next_keepalive = now + 20_000;
                self.safe(link, "version", 2500);
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
    pub fn run(host: String, port: i64, api_key: String, stop: Arc<AtomicBool>, emit: impl Fn(OverlayState) + Send + Sync + 'static) {
        let emit: EmitFn = Arc::new(emit);
        let mut session = Ts3Session {
            host,
            port,
            api_key,
            stop: stop.clone(),
            whispers: crate::model::WhisperBook::new(emit.clone()),
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
            refresh_heavy: false,
            refresh_first_at: 0,
            refresh_stat_at: 0,
            refresh_stat_calls: 0,
            refresh_stat_heavy: 0,
            refresh_stat_ms: 0,
            refresh_stat_max: 0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WhisperBook;

    fn session_for_test() -> Ts3Session {
        Ts3Session {
            stop: Arc::new(AtomicBool::new(false)),
            host: "127.0.0.1".to_string(),
            port: 10011,
            api_key: String::new(),
            whispers: WhisperBook::new(Arc::new(|_| {})),
            emit: Arc::new(|_| {}),
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
            refresh_heavy: false,
            refresh_first_at: 0,
            refresh_stat_at: 0,
            refresh_stat_calls: 0,
            refresh_stat_heavy: 0,
            refresh_stat_ms: 0,
            refresh_stat_max: 0,
        }
    }

    /// Ревью 2026-11: enterview создаёт/обновляет клиента ЛОКАЛЬНО.
    #[test]
    fn enterview_creates_user_locally() {
        let mut s = session_for_test();
        s.own_cid = 3;
        s.handle_event("notifycliententerview clid=5 ctid=3 client_nickname=Alfa client_type=0 client_away=1 client_servergroups=16,20");
        let u = s.clients.get(&5).expect("user created without refresh");
        assert_eq!(u.nickname, "Alfa");
        assert_eq!(u.channel_id, 3);
        assert!(u.away);
        assert_eq!(u.server_groups, "16,20");
        assert!(s.chat.rows().iter().any(|c| c.event == Some("join")), "join row expected");
    }

    /// Ревью 2026-11: moved обновляет канал ЛОКАЛЬНО.
    #[test]
    fn moved_updates_channel_locally() {
        let mut s = session_for_test();
        s.own_cid = 3;
        s.handle_event("notifycliententerview clid=5 ctid=3 client_nickname=Alfa client_type=0");
        s.handle_event("notifyclientmoved clid=5 ctid=7");
        assert_eq!(s.clients.get(&5).map(|u| u.channel_id), Some(7), "channel updated without refresh");
        assert!(s.chat.rows().iter().any(|c| c.event == Some("move")), "move row expected");
        assert_eq!(s.refresh_pending_at, 0, "local move must not queue a refresh");
    }

    /// leftview удаляет клиента локально.
    #[test]
    fn leftview_removes_user_locally() {
        let mut s = session_for_test();
        s.own_cid = 3;
        s.handle_event("notifycliententerview clid=5 ctid=3 client_nickname=Alfa client_type=0");
        s.handle_event("notifyclientleftview clid=5 client_nickname=Alfa");
        assert!(s.clients.is_empty());
        assert!(s.chat.rows().iter().any(|c| c.event == Some("leave")), "leave row expected");
        assert_eq!(s.refresh_pending_at, 0, "local leave must not queue a refresh");
    }

    /// schedule_refresh: дебаунс без голодания + sticky-флаг heavy.
    #[test]
    fn schedule_refresh_marks_heavy_and_debounces() {
        let mut s = session_for_test();
        s.schedule_refresh(false);
        assert!(!s.refresh_heavy);
        assert!(s.refresh_pending_at > 0);
        s.schedule_refresh(true);
        assert!(s.refresh_heavy, "heavy sticks while pending");
    }
}
