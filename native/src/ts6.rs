//! TeamSpeak 5/6 data source via the Remote Apps WebSocket API
//! (ws://127.0.0.1:5899). Port of the TS Ts6Connector.

use crate::chatlog::ChatLog;
use crate::diag::diag;
use crate::json::Json;
use crate::model::{now_ms, ChatKind, ChatMessage, OverlayState, ServerGroup, UserState};
use crate::wsframe::{read_frame, ws_accept, write_frame, Frame};
use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub type EmitFn = crate::model::Emit;
pub type SaveKeyFn = Arc<dyn Fn(String) + Send + Sync>;

const APP_IDENTIFIER: &str = "dev.tsnotifier.overlay";
const APP_VERSION: &str = "1.0.0";
const APP_NAME: &str = "TS3-TS6 Notifier";
const APP_DESCRIPTION: &str = "TeamSpeak overlay: channel roster, live speaking indicators, icons";

#[derive(Debug, Clone)]
struct ChannelInfo {
    id: i64,
    parent_id: i64,
    name: String,
}

enum SessionEnd {
    Closed,
    Resync,
}

struct Ts6Session {
    host: String,
    port: i64,
    api_key: String,
    save_key: SaveKeyFn,
    stop: Arc<AtomicBool>,
    emit: EmitFn,

    clients: HashMap<i64, UserState>,
    channels: HashMap<i64, ChannelInfo>,
    own_by_connection: HashMap<i64, i64>,
    group_infos: HashMap<i64, ServerGroup>,
    chat: ChatLog,
    connection_id: i64,
    preferred_connection_id: i64,
    own_client_id: i64,
    own_channel_id: i64,
    server_name: String,
    authenticated: bool,
    auth_denied: bool,
    fast_reconnect: bool,
    resync_requested: bool,
    announced_once: bool,
    noisy_diag_skipped: i32,
    cur_status: &'static str,
    cur_detail: String,
    whispers: Arc<crate::model::WhisperBook>,
}

impl Ts6Session {
    fn publish(&mut self, status: &'static str, detail: &str) {
        self.cur_status = status;
        self.cur_detail = detail.to_string();
        self.emit_state();
    }

    fn emit_state(&mut self) {
        crate::model::WhisperBook::publish(&self.whispers, self.build_state());
    }

    fn build_state(&self) -> OverlayState {
        let mut group_map: HashMap<i64, ServerGroup> = self.group_infos.clone();
        for c in self.clients.values() {
            for id in c.server_groups.split(',').filter_map(|s| s.trim().parse::<i64>().ok()) {
                if id > 0 {
                    group_map.entry(id).or_insert_with(|| ServerGroup { id, name: String::new(), icon_id: 0 });
                }
            }
        }
        let mut groups: Vec<ServerGroup> = group_map.into_values().collect();
        groups.sort_by_key(|g| g.id);
        let mut st = OverlayState::empty("ts6", self.cur_status, &self.cur_detail);
        st.server_name = self.server_name.clone();
        st.channel_name = self.channels.get(&self.own_channel_id).map(|c| c.name.clone()).unwrap_or_default();
        st.channel_id = if self.own_channel_id > 0 { self.own_channel_id.to_string() } else { String::new() };
        st.self_id = if self.own_client_id > 0 { Some(self.own_client_id.to_string()) } else { None };
        st.users = self
            .clients
            .values()
            .filter(|c| self.own_channel_id > 0 && c.channel_id == self.own_channel_id)
            .cloned()
            .collect();
        st.groups = groups;
        st.chat = self.chat.rows().to_vec();
        // Outside whisperers (another channel) — v1.4.12 parity.
        let roster: std::collections::HashSet<String> = st.users.iter().map(|u| u.id.clone()).collect();
        let own = self.own_client_id.to_string();
        let wids = self.whispers.ids(&own, &|id| roster.contains(id));
        st.whisperers = wids
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

    fn push_feed(&mut self, user: &UserState, event: &'static str, detail: &str, own: bool) {
        if user.is_query || user.nickname.is_empty() {
            return;
        }
        self.chat.push(crate::chatlog::sys_row(&user.id, &user.nickname, &user.nickname, event, detail, own));
        self.emit_state();
    }

    fn push_chat(&mut self, msg: ChatMessage) {
        self.chat.push(msg);
        self.emit_state();
    }

    fn resync(&mut self, reason: &str, writer: &Arc<Mutex<TcpStream>>) {
        if self.resync_requested {
            return;
        }
        self.resync_requested = true;
        println!("[ts6] resync: {reason}");
        if let Ok(guard) = writer.lock() {
            let _ = guard.shutdown(std::net::Shutdown::Both);
        }
    }

    fn note_active_connection(&mut self, conn_id: i64, reason: &str, writer: &Arc<Mutex<TcpStream>>) {
        if conn_id <= 0 || conn_id == self.connection_id {
            return;
        }
        self.preferred_connection_id = conn_id;
        self.fast_reconnect = true;
        self.resync(&format!("{reason}: switch to connection {conn_id}"), writer);
    }

    fn own_client_id_for(&self, conn_id: i64) -> i64 {
        self.own_by_connection.get(&conn_id).copied().unwrap_or(0)
    }

    /* ---------------- messages ---------------- */

    fn handle_message(&mut self, raw: &str, writer: &Arc<Mutex<TcpStream>>) {
        let Ok(root) = Json::parse(raw) else {
            return;
        };
        let ty = root.get("type").map(|v| v.loose_str()).unwrap_or_default();
        if ty.is_empty() {
            return;
        }
        let Some(payload) = root.get("payload").cloned() else { return };
        // TS6 wraps some events: {"type":"proto.ServerEvent","payload":{...}}
        if ty.starts_with("proto.") {
            let conn_id = payload.get("connectionId").map(|v| v.loose_i64()).unwrap_or(0);
            if let Json::Obj(items) = &payload {
                for (name, value) in items {
                    if name == "connectionId" {
                        continue;
                    }
                    if let Json::Obj(sub) = value {
                        let mut merged = sub.clone();
                        if conn_id != 0 {
                            merged.retain(|(k, _)| k != "connectionId");
                            merged.push(("connectionId".to_string(), Json::i(conn_id)));
                        }
                        self.dispatch(name, &Json::Obj(merged), writer);
                    }
                }
            }
            return;
        }
        self.dispatch(&ty, &payload, writer);
    }

    fn dispatch(&mut self, ty: &str, payload: &Json, writer: &Arc<Mutex<TcpStream>>) {
        let conn_id = payload.get("connectionId").map(|v| v.loose_i64()).unwrap_or(0);
        let foreign = conn_id != 0 && self.connection_id != 0 && conn_id != self.connection_id;

        // diagnostics: talk/updated sampled 1-in-10
        let noisy = ty.to_lowercase().contains("talk") || ty.to_lowercase().contains("updated");
        let raw = payload.to_string();
        let short = &raw[..raw.len().min(200)];
        if noisy {
            self.noisy_diag_skipped += 1;
            if self.noisy_diag_skipped % 10 == 1 {
                diag("ts6", &format!("{ty} {short}"));
            }
        } else {
            diag("ts6", &format!("{ty} {short}"));
        }

        match ty {
            "auth" => self.handle_auth(payload, writer),
            "channels" => {
                if foreign {
                    return;
                }
                self.channels.clear();
                if let Some(Json::Obj(_)) = payload.get("info") {
                    self.parse_channel_infos(payload.get("info").unwrap());
                }
                self.emit_state();
            }
            "channelCreated" | "channelEdited" | "channelPropertiesUpdated" => {
                if foreign {
                    return;
                }
                let id = payload.get("channelId").or_else(|| payload.get("id")).map(|v| v.loose_i64()).unwrap_or(0);
                let props = payload.get("properties");
                if id > 0 {
                    if let Some(props) = props {
                        let prev = self.channels.get(&id);
                        self.channels.insert(
                            id,
                            ChannelInfo {
                                id,
                                parent_id: payload
                                    .get("parentId")
                                    .map(|v| v.loose_i64())
                                    .filter(|v| *v != 0)
                                    .or_else(|| prev.map(|p| p.parent_id))
                                    .unwrap_or(0),
                                name: props
                                    .get("name")
                                    .map(|v| v.loose_str())
                                    .filter(|s| !s.is_empty())
                                    .or_else(|| prev.map(|p| p.name.clone()))
                                    .unwrap_or_default(),
                            },
                        );
                        self.emit_state();
                    }
                }
            }
            "channelDeleted" => {
                let id = payload.get("channelId").map(|v| v.loose_i64()).unwrap_or(0);
                if id > 0 && self.channels.remove(&id).is_some() {
                    self.emit_state();
                }
            }
            "clientMoved" => {
                // A move on another server tab can mean our own client became active there.
                let client_id = payload.get("clientId").map(|v| v.loose_i64()).unwrap_or(0);
                if foreign {
                    if client_id != 0 && self.own_client_id_for(conn_id) == client_id {
                        self.note_active_connection(conn_id, "self moved", writer);
                    }
                    return;
                }
                let new_channel = payload
                    .get("newChannelId")
                    .or_else(|| payload.get("channelToId"))
                    .map(|v| v.loose_i64())
                    .unwrap_or(0);
                let old_channel = payload
                    .get("oldChannelId")
                    .or_else(|| payload.get("channelFromId"))
                    .map(|v| v.loose_i64())
                    .unwrap_or(0);
                if client_id == 0 {
                    return;
                }
                let existing = self.clients.get(&client_id).cloned();
                if new_channel <= 0 {
                    if let Some(u) = existing {
                        self.push_feed(&u, "leave", "", false);
                        self.clients.remove(&client_id);
                    }
                } else if let Some(mut u) = existing {
                    if !u.is_query {
                        let own = new_channel == self.own_channel_id || u.channel_id == self.own_channel_id;
                        let detail = self.channels.get(&new_channel).map(|c| c.name.clone()).unwrap_or_default();
                        self.push_feed(&u, "move", &detail, own);
                    }
                    u.channel_id = new_channel;
                    self.clients.insert(client_id, u);
                    if client_id == self.own_client_id {
                        self.own_channel_id = new_channel;
                    }
                } else if old_channel <= 0 {
                    self.resync("client join without properties", writer);
                    return;
                } else {
                    let mut u = UserState::new(client_id.to_string(), "");
                    u.channel_id = old_channel;
                    self.clients.insert(client_id, u);
                    self.resync("client move of unknown client", writer);
                    return;
                }
                self.emit_state();
            }
            "clientPropertiesUpdated" | "clientUpdated" => {
                if foreign {
                    return;
                }
                let client_id = payload.get("clientId").map(|v| v.loose_i64()).unwrap_or(0);
                let Some(props) = payload.get("properties") else { return };
                if client_id == 0 {
                    return;
                }
                let existing = self.clients.get(&client_id).cloned().unwrap_or_else(|| {
                    let mut u = UserState::new(client_id.to_string(), "");
                    u.channel_id = 0;
                    u.is_self = client_id == self.own_client_id;
                    u
                });
                self.clients.insert(client_id, apply_properties(&existing, props));
                self.emit_state();
            }
            "talkStatusChanged" => {
                let client_id = payload.get("clientId").map(|v| v.loose_i64()).unwrap_or(0);
                let status = payload.get("status").map(|v| v.loose_i64()).unwrap_or(0);
                let talking = status >= 1;
                let whisper = talking
                    && (status >= 2
                        || payload.get("whisper").map(|v| v.loose_bool()).unwrap_or(false)
                        || payload.get("isWhisper").map(|v| v.loose_bool()).unwrap_or(false)
                        || (payload.get("targetMode").is_some()
                            && payload.get("targetMode").map(|v| v.loose_i64()).unwrap_or(0) != 0));
                if foreign {
                    if talking && client_id != 0 && self.own_client_id_for(conn_id) == client_id {
                        self.note_active_connection(conn_id, "self talk", writer);
                    }
                    return;
                }
                if let Some(u) = self.clients.get_mut(&client_id) {
                    if u.talking != talking || u.whispering != whisper {
                        u.talking = talking;
                        u.whispering = whisper;
                        self.emit_state();
                    }
                }
                // A whisper may come from another channel — note it even when
                // the speaker never appears on the roster.
                if client_id != self.own_client_id {
                    crate::model::WhisperBook::note(&self.whispers, &client_id.to_string(), whisper);
                }
            }
            "clientSelfPropertyUpdated" => {
                let flag = payload.get("flag").map(|v| v.loose_str()).unwrap_or_default();
                let value = payload.get("newValue").map(|v| v.loose_bool()).unwrap_or(false);
                if foreign {
                    if flag.eq_ignore_ascii_case("inputhardware") && value {
                        self.note_active_connection(conn_id, "inputHardware", writer);
                    }
                    return;
                }
                let Some(me) = self.clients.get(&self.own_client_id).cloned() else { return };
                let mut u = me;
                match flag.as_str() {
                    "flagTalking" => u.talking = value,
                    "inputMuted" | "isMuted" | "inputDeactivated" => u.input_muted = value,
                    "outputMuted" | "outputOnlyMuted" => u.output_muted = value,
                    "away" => u.away = value,
                    _ => return,
                }
                self.clients.insert(self.own_client_id, u);
                self.emit_state();
            }
            "connectStatusChanged" => {
                let status = payload.get("status").map(|v| v.loose_i64()).unwrap_or(0);
                if status == 0 {
                    if conn_id == self.connection_id || self.connection_id == 0 {
                        self.clients.clear();
                        self.channels.clear();
                        self.own_client_id = 0;
                        self.own_channel_id = 0;
                        self.connection_id = 0;
                        self.publish("offline", "not connected to a TeamSpeak server");
                    }
                    return;
                }
                if status == 4 {
                    let empty = self.clients.is_empty() || self.own_client_id == 0;
                    if self.connection_id == 0 || empty {
                        self.preferred_connection_id = if conn_id != 0 { conn_id } else { self.preferred_connection_id };
                        self.fast_reconnect = true;
                        self.resync("connectStatus=4", writer);
                    }
                }
            }
            "serverPropertiesUpdated" => {
                if foreign {
                    return;
                }
                if let Some(props) = payload.get("properties") {
                    let name = props.get("name").map(|v| v.loose_str()).unwrap_or_default();
                    if !name.is_empty() {
                        self.server_name = name;
                        self.emit_state();
                    }
                }
            }
            "clientJoined" | "clientConnected" | "clientEnteredView" | "clientOnline" => {
                if !foreign {
                    let client_id = payload
                        .get("clientId")
                        .or_else(|| payload.get("id"))
                        .map(|v| v.loose_i64())
                        .unwrap_or(0);
                    let props = payload.get("properties").unwrap_or(payload);
                    let nick = props
                        .get("nickname")
                        .or_else(|| props.get("name"))
                        .map(|v| v.loose_str())
                        .unwrap_or_default();
                    let channel_id = payload
                        .get("channelId")
                        .or_else(|| payload.get("newChannelId"))
                        .or_else(|| props.get("channelId"))
                        .map(|v| v.loose_i64())
                        .unwrap_or(0);
                    if client_id > 0 && !nick.is_empty() && client_id != self.own_client_id {
                        let in_our = channel_id == self.own_channel_id;
                        let detail = if in_our {
                            self.channels.get(&channel_id).map(|c| c.name.clone()).unwrap_or_default()
                        } else {
                            String::new()
                        };
                        self.chat.push(crate::chatlog::sys_row(
                            &client_id.to_string(),
                            &nick,
                            &nick,
                            "join",
                            &detail,
                            in_our,
                        ));
                    }
                }
                self.resync(&format!("event {ty}"), writer);
            }
            "clientChannelChanged" | "clientSwitchedChannel" => {
                if foreign {
                    return;
                }
                let client_id = payload
                    .get("clientId")
                    .or_else(|| payload.get("id"))
                    .map(|v| v.loose_i64())
                    .unwrap_or(0);
                let to = payload
                    .get("newChannelId")
                    .or_else(|| payload.get("toChannelId"))
                    .or_else(|| payload.get("channelId"))
                    .or_else(|| payload.get("properties").and_then(|p| p.get("channelId")))
                    .map(|v| v.loose_i64())
                    .unwrap_or(0);
                if let Some(user) = self.clients.get(&client_id).cloned() {
                    if client_id == self.own_client_id {
                        self.own_channel_id = if to != 0 { to } else { self.own_channel_id };
                    } else {
                        let own = to == self.own_channel_id || user.channel_id == self.own_channel_id;
                        let detail = self.channels.get(&to).map(|c| c.name.clone()).unwrap_or_default();
                        self.push_feed(&user, "move", &detail, own);
                    }
                }
                self.resync(&format!("event {ty}"), writer);
            }
            "channelMoved" | "serverEdited" => {
                self.resync(&format!("event {ty}"), writer);
            }
            "clientLeft" | "clientLeftView" | "clientDisconnected" => {
                if foreign {
                    return;
                }
                let client_id = payload
                    .get("clientId")
                    .or_else(|| payload.get("id"))
                    .map(|v| v.loose_i64())
                    .unwrap_or(0);
                if let Some(existing) = self.clients.get(&client_id).cloned() {
                    if client_id != self.own_client_id {
                        let was_ours = existing.channel_id == self.own_channel_id;
                        let detail = if was_ours {
                            self.channels.get(&existing.channel_id).map(|c| c.name.clone()).unwrap_or_default()
                        } else {
                            String::new()
                        };
                        self.push_feed(&existing, "leave", &detail, was_ours);
                    }
                    self.clients.remove(&client_id);
                    self.emit_state();
                }
            }
            "neededPermissions" | "clientNeededPermissions" => {}
            _ => {
                if ty.to_lowercase().contains("group") {
                    self.ingest_group_infos(payload);
                    if let Some(props) = payload.get("properties") {
                        self.ingest_group_infos(props);
                    }
                    self.emit_state();
                    return;
                }
                let lower = ty.to_lowercase();
                if (lower.contains("message") || lower.contains("chat"))
                    && !lower.contains("perm")
                    && !lower.contains("status")
                {
                    self.handle_chat_event(payload);
                }
            }
        }
    }

    /* ---------------- chat event extraction (defensive) ---------------- */

    fn handle_chat_event(&mut self, payload: &Json) {
        // Collect nested objects; senderish marks sender/author/from/invoker.
        fn walk<'a>(obj: &'a Json, senderish: bool, depth: usize, pools: &mut Vec<(&'a Json, bool)>) {
            if depth > 3 {
                return;
            }
            let items = match obj {
                Json::Obj(items) => items,
                _ => return,
            };
            for (key, value) in items {
                let child = match value {
                    Json::Obj(_) => value,
                    _ => continue,
                };
                if pools.iter().any(|(p, _)| std::ptr::eq(*p, child)) {
                    continue;
                }
                let child_senderish = senderish
                    || ["sender", "author", "from", "invoker"]
                        .iter()
                        .any(|k| key.to_lowercase().contains(k));
                pools.push((child, child_senderish));
                walk(child, child_senderish, depth + 1, pools);
            }
        }
        let mut pools: Vec<(&Json, bool)> = vec![(payload, false)];
        walk(payload, false, 0, &mut pools);

        let pick_str = |keys: &[&str], sender_only: bool| -> String {
            for k in keys {
                for (obj, senderish) in &pools {
                    if sender_only && !*senderish {
                        continue;
                    }
                    if let Some(Json::Str(v)) = obj.get(k) {
                        if !v.trim().is_empty() {
                            return v.clone();
                        }
                    }
                }
            }
            String::new()
        };
        let pick_int = |keys: &[&str], sender_only: bool| -> i64 {
            for k in keys {
                for (obj, senderish) in &pools {
                    if sender_only && !*senderish {
                        continue;
                    }
                    let n = match obj.get(k) {
                        Some(Json::Num(n)) if *n > 0.0 => Some(*n as i64),
                        Some(Json::Str(s)) => s.parse::<i64>().ok().filter(|n| *n > 0),
                        _ => None,
                    };
                    if let Some(n) = n {
                        return n;
                    }
                }
            }
            0
        };

        let body = pick_str(&["msg", "message", "text", "content", "body", "richtext"], false);
        if body.is_empty() {
            return;
        }
        let uid = pick_int(&["invokerId", "invokerid", "senderId", "senderid", "clientId", "clid", "userId", "userid"], false)
            .max(pick_int(&["id", "from"], true));
        let mut nickname = pick_str(
            &[
                "invokerName", "invokername", "senderName", "sendername", "displayName", "displayname",
                "nickname", "userName", "username", "sender", "author", "from",
            ],
            false,
        );
        if nickname.is_empty() {
            nickname = pick_str(&["name", "nick"], true);
        }
        if nickname.is_empty() {
            nickname = self.clients.get(&uid).map(|u| u.nickname.clone()).unwrap_or_default();
        }
        let mode = pick_int(&["targetMode", "targetmode"], false);
        let kind = match mode {
            1 => ChatKind::Private,
            3 => ChatKind::Server,
            _ => ChatKind::Channel,
        };
        self.push_chat(ChatMessage {
            id: String::new(),
            user_id: uid.to_string(),
            nickname,
            text: body,
            kind,
            at: 0,
            event: None,
            detail: None,
            own: None,
        });
    }

    /* ---------------- auth + snapshots ---------------- */

    fn handle_auth(&mut self, payload: &Json, writer: &Arc<Mutex<TcpStream>>) {
        let code = payload
            .get("status")
            .and_then(|s| s.get("code"))
            .map(|v| v.loose_i64())
            .unwrap_or(0);
        if code != 0 {
            self.auth_denied = true;
            let msg = payload
                .get("status")
                .and_then(|s| s.get("message"))
                .map(|v| v.loose_str())
                .unwrap_or_default();
            self.publish("waiting-auth", &format!("authorization rejected ({})", if msg.is_empty() { code.to_string() } else { msg }));
            return;
        }
        self.auth_denied = false;

        let mut api_key = payload.get("apiKey").map(|v| v.loose_str()).unwrap_or_default();
        if api_key.is_empty() {
            api_key = payload.get("content").and_then(|c| c.get("apiKey")).map(|v| v.loose_str()).unwrap_or_default();
        }
        if api_key.is_empty() {
            api_key = payload.get("auth").and_then(|c| c.get("apiKey")).map(|v| v.loose_str()).unwrap_or_default();
        }
        if !api_key.is_empty() && api_key != self.api_key {
            self.api_key = api_key.clone();
            (self.save_key)(api_key);
            println!("[ts6] Remote Apps API key received and saved");
        }

        let connections: Vec<Json> = match payload.get("connections").and_then(|v| v.as_arr()) {
            Some(arr) => arr.to_vec(),
            None => Vec::new(),
        };
        if connections.is_empty() {
            self.resync("auth without connections", writer);
            return;
        }

        self.authenticated = true;
        let current = payload.get("currentConnectionId").map(|v| v.loose_i64()).unwrap_or(0);
        if current > 0 {
            self.preferred_connection_id = current;
        }

        self.own_by_connection.clear();
        for conn in &connections {
            let id = conn.get("id").map(|v| v.loose_i64()).unwrap_or(0);
            let own = conn.get("clientId").map(|v| v.loose_i64()).unwrap_or(0);
            if id > 0 && own > 0 {
                self.own_by_connection.insert(id, own);
            }
        }
        let chosen = connections
            .iter()
            .find(|c| c.get("id").map(|v| v.loose_i64()).unwrap_or(0) == self.preferred_connection_id)
            .or_else(|| {
                connections
                    .iter()
                    .find(|c| c.get("clientInfos").and_then(|v| v.as_arr()).map(|a| !a.is_empty()).unwrap_or(false))
            })
            .or_else(|| connections.first());
        if let Some(conn) = chosen {
            self.parse_connection(conn, writer);
        }
        self.publish("live", "");
    }

    fn parse_connection(&mut self, conn: &Json, _writer: &Arc<Mutex<TcpStream>>) {
        let prev_clients = self.clients.clone();
        self.channels.clear();
        self.clients.clear();
        self.connection_id = conn.get("id").map(|v| v.loose_i64()).unwrap_or(0);
        self.own_client_id = conn.get("clientId").map(|v| v.loose_i64()).unwrap_or(0);
        let name = conn
            .get("properties")
            .and_then(|p| p.get("name"))
            .map(|v| v.loose_str())
            .unwrap_or_default();
        if !name.is_empty() {
            self.server_name = name;
        }

        if let Some(channel_infos) = conn.get("channelInfos") {
            self.parse_channel_infos(channel_infos);
        }
        self.ingest_group_infos(conn);

        if let Some(list) = conn.get("clientInfos").and_then(|v| v.as_arr()) {
            for raw in list {
                let id = raw.get("id").map(|v| v.loose_i64()).unwrap_or(0);
                let Some(props) = raw.get("properties") else { continue };
                if id <= 0 {
                    continue;
                }
                self.ingest_group_infos(props);
                let mut u = UserState::new(id.to_string(), "");
                u.channel_id = raw.get("channelId").map(|v| v.loose_i64()).unwrap_or(0);
                u.is_self = id == self.own_client_id;
                self.clients.insert(id, apply_properties(&u, props));
            }
        }

        self.own_channel_id = self.clients.get(&self.own_client_id).map(|u| u.channel_id).unwrap_or(0);

        // Diff-announce missed events (v1.4.7 flood rule, incl. moved count).
        if self.announced_once {
            let delta = |a: &HashMap<i64, UserState>, b: &HashMap<i64, UserState>| {
                a.keys().filter(|id| !b.contains_key(id)).count()
            };
            let moved = self
                .clients
                .iter()
                .filter(|(id, u)| {
                    prev_clients
                        .get(id)
                        .map(|p| p.channel_id > 0 && p.channel_id != u.channel_id)
                        .unwrap_or(false)
                })
                .count();
            let mass = |n: usize| (n as f64) > 5f64.max(prev_clients.len() as f64 * 0.3);
            let quiet = prev_clients.is_empty()
                || mass(delta(&self.clients, &prev_clients))
                || mass(delta(&prev_clients, &self.clients))
                || mass(moved);
            if !quiet {
                for (id, u) in self.clients.clone() {
                    let prev = prev_clients.get(&id);
                    if prev.is_none() {
                        if !u.is_query && !u.is_self && !u.nickname.is_empty() {
                            let own = u.channel_id == self.own_channel_id;
                            let detail = if own {
                                self.channels.get(&u.channel_id).map(|c| c.name.clone()).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            self.push_feed(&u, "join", &detail, own);
                        }
                        continue;
                    }
                    let prev = prev.unwrap();
                    if !u.is_query && !u.nickname.is_empty() && prev.channel_id > 0 && prev.channel_id != u.channel_id {
                        let own = u.channel_id == self.own_channel_id || prev.channel_id == self.own_channel_id;
                        let detail = self.channels.get(&u.channel_id).map(|c| c.name.clone()).unwrap_or_default();
                        self.push_feed(&u, "move", &detail, own);
                    }
                }
                for (id, u) in prev_clients.iter() {
                    if self.clients.contains_key(id) || u.is_query || u.is_self || u.nickname.is_empty() {
                        continue;
                    }
                    let own = u.channel_id == self.own_channel_id;
                    let detail = if own {
                        self.channels.get(&u.channel_id).map(|c| c.name.clone()).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    self.push_feed(u, "leave", &detail, own);
                }
            }
        } else {
            self.announced_once = true;
        }
    }

    fn ingest_group_infos(&mut self, source: &Json) {
        let Json::Obj(items) = source else { return };
        for (key, value) in items {
            if !key.to_lowercase().contains("group") {
                continue;
            }
            let Some(arr) = value.as_arr() else { continue };
            for raw in arr {
                match raw {
                    Json::Num(n) => {
                        let id = *n as i64;
                        if id > 0 {
                            self.group_infos.entry(id).or_insert_with(|| ServerGroup { id, name: String::new(), icon_id: 0 });
                        }
                    }
                    Json::Str(s) => {
                        if let Ok(id) = s.parse::<i64>() {
                            if id > 0 {
                                self.group_infos.entry(id).or_insert_with(|| ServerGroup { id, name: String::new(), icon_id: 0 });
                            }
                        }
                    }
                    Json::Obj(_) => {
                        let id = raw
                            .get("id")
                            .or_else(|| raw.get("sgid"))
                            .or_else(|| raw.get("groupId"))
                            .map(|v| v.loose_i64())
                            .unwrap_or(0);
                        if id <= 0 {
                            continue;
                        }
                        let prev = self.group_infos.get(&id);
                        self.group_infos.insert(
                            id,
                            ServerGroup {
                                id,
                                name: raw
                                    .get("name")
                                    .map(|v| v.loose_str())
                                    .filter(|s| !s.is_empty())
                                    .or_else(|| prev.map(|p| p.name.clone()))
                                    .unwrap_or_default(),
                                icon_id: raw
                                    .get("iconId")
                                    .or_else(|| raw.get("iconid"))
                                    .map(|v| v.loose_i64())
                                    .filter(|v| *v != 0)
                                    .or_else(|| prev.map(|p| p.icon_id))
                                    .unwrap_or(0),
                            },
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn parse_channel_infos(&mut self, info: &Json) {
        if let Some(list) = info.get("rootChannels").and_then(|v| v.as_arr()) {
            for raw in list {
                self.add_channel(raw, 0);
            }
        }
        if let Some(Json::Obj(subs)) = info.get("subChannels") {
            for (parent, list) in subs {
                let parent_id = parent.parse::<i64>().unwrap_or(0);
                if let Some(arr) = list.as_arr() {
                    for raw in arr {
                        self.add_channel(raw, parent_id);
                    }
                }
            }
        }
    }

    fn add_channel(&mut self, ch: &Json, fallback_parent: i64) {
        let id = ch.get("id").map(|v| v.loose_i64()).unwrap_or(0);
        let Some(props) = ch.get("properties") else { return };
        if id <= 0 {
            return;
        }
        self.channels.insert(
            id,
            ChannelInfo {
                id,
                parent_id: ch
                    .get("parentId")
                    .map(|v| v.loose_i64())
                    .filter(|v| *v != 0)
                    .unwrap_or(fallback_parent),
                name: props.get("name").map(|v| v.loose_str()).unwrap_or_default(),
            },
        );
    }
}

/// Merge partial TS6 client properties into a client record.
pub fn apply_properties(client: &UserState, props: &Json) -> UserState {
    let mut u = client.clone();
    let has = |k: &str| props.get(k).is_some();
    let jstr = |k: &str| props.get(k).map(|v| v.loose_str()).unwrap_or_default();
    let jbool = |k: &str| props.get(k).map(|v| v.loose_bool()).unwrap_or(false);

    let nickname = ["nickname", "displayName", "displayname", "userName", "name"]
        .iter()
        .map(|k| jstr(k))
        .find(|s| !s.is_empty());
    if let Some(n) = nickname {
        u.nickname = n;
    }
    if has("inputMuted") || has("inputDeactivated") || has("isMuted") {
        u.input_muted = jbool("inputMuted") || jbool("inputDeactivated") || jbool("isMuted");
    }
    if has("outputMuted") || has("outputOnlyMuted") {
        u.output_muted = jbool("outputMuted") || jbool("outputOnlyMuted");
    }
    if has("flagTalking") {
        u.talking = jbool("flagTalking");
    }
    if has("flagTalkingWithWhisper") || has("isWhispering") || has("whisper") {
        let whispering = jbool("flagTalkingWithWhisper") || jbool("isWhispering") || jbool("whisper");
        u.whispering = whispering;
        if whispering {
            u.talking = true;
        }
    }
    if has("away") {
        u.away = jbool("away");
    }
    if has("awayMessage") {
        u.away_message = jstr("awayMessage");
    }
    if has("isPrioritySpeaker") {
        u.priority_speaker = jbool("isPrioritySpeaker");
    }
    if has("isChannelCommander") {
        u.channel_commander = jbool("isChannelCommander");
    }
    if has("flagAvatar") {
        u.avatar_hash = jstr("flagAvatar");
    }
    if has("serverGroups") {
        u.server_groups = jstr("serverGroups");
    }
    if let Some(ty) = props.get("type") {
        u.is_query = match ty {
            Json::Str(s) => s.to_uppercase().contains("QUERY"),
            Json::Num(n) => *n == 1.0,
            _ => false,
        };
    }
    u
}

/* ---------------- connection plumbing ---------------- */

enum SessionEvent {
    Frame(Frame),
    Disconnected,
}

fn ws_connect(host: &str, port: i64) -> Result<(TcpStream, Arc<Mutex<TcpStream>>, Receiver<SessionEvent>), String> {
    let stream = TcpStream::connect_timeout(
        &format!("{host}:{port}").parse().map_err(|e| format!("bad address: {e}"))?,
        Duration::from_millis(4000),
    )
    .map_err(|e| format!("connect: {e}"))?;
    let _ = stream.set_nodelay(true);

    // client handshake
    let key = crate::wsframe::base64_encode(&{
        let mut seed = [0u8; 16];
        let t = now_ms().to_le_bytes();
        for (i, b) in seed.iter_mut().enumerate() {
            *b = t[i % 8] ^ (i as u8).wrapping_mul(31);
        }
        seed
    });
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    let mut write_stream = stream.try_clone().map_err(|e| e.to_string())?;
    write_stream.write_all(req.as_bytes()).map_err(|e| format!("handshake write: {e}"))?;
    // read the response head
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut head = String::new();
    use std::io::BufRead;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| format!("handshake read: {e}"))?;
        if n == 0 {
            return Err("handshake: connection closed".into());
        }
        head.push_str(&line);
        if line == "\r\n" {
            break;
        }
        if head.len() > 16_384 {
            return Err("handshake: response too large".into());
        }
    }
    if !head.starts_with("HTTP/1.1 101") {
        return Err(format!("handshake rejected: {}", head.lines().next().unwrap_or("")));
    }
    if let Some(pos) = head.to_lowercase().find("sec-websocket-accept:") {
        let accept = head[pos + 21..].lines().next().unwrap_or("").trim().to_string();
        if accept != ws_accept(&key) {
            return Err("handshake: bad accept key".into());
        }
    }

    let (tx, rx) = channel::<SessionEvent>();
    let reader_stream = stream.try_clone().map_err(|e| e.to_string())?;
    thread::spawn(move || {
        let mut rs = reader_stream;
        loop {
            match read_frame(&mut rs) {
                Ok(Some(frame)) => {
                    if tx.send(SessionEvent::Frame(frame)).is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = tx.send(SessionEvent::Disconnected);
                    break;
                }
                Err(_) => {
                    let _ = tx.send(SessionEvent::Disconnected);
                    break;
                }
            }
        }
    });
    Ok((stream, Arc::new(Mutex::new(write_stream)), rx))
}

impl Ts6Session {
    fn run_session(&mut self, writer: &Arc<Mutex<TcpStream>>, rx: &Receiver<SessionEvent>) -> SessionEnd {
        self.resync_requested = false;
        self.connection_id = 0;
        self.own_client_id = 0;
        self.own_channel_id = 0;
        self.auth_denied = false;
        self.publish("connecting", "");

        // send auth
        let auth = crate::json::obj(vec![
            ("type", Json::s("auth")),
            (
                "payload",
                crate::json::obj(vec![
                    ("identifier", Json::s(APP_IDENTIFIER)),
                    ("version", Json::s(APP_VERSION)),
                    ("name", Json::s(APP_NAME)),
                    ("description", Json::s(APP_DESCRIPTION)),
                    ("content", crate::json::obj(vec![("apiKey", Json::s(self.api_key.clone()))])),
                ]),
            ),
        ]);
        if write_frame(&mut *writer.lock().unwrap(), &Frame::Text(auth.to_string()), true).is_err() {
            return SessionEnd::Closed;
        }

        let opened_at = now_ms();
        let mut auth_timeout_sent = false;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return SessionEnd::Closed;
            }
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(SessionEvent::Frame(Frame::Text(raw))) => {
                    self.handle_message(&raw, writer);
                    if self.resync_requested {
                        return SessionEnd::Resync;
                    }
                }
                Ok(SessionEvent::Frame(Frame::Ping(p))) => {
                    let _ = write_frame(&mut *writer.lock().unwrap(), &Frame::Pong(p), true);
                }
                Ok(SessionEvent::Frame(_)) => {}
                Ok(SessionEvent::Disconnected) => return SessionEnd::Closed,
                Err(RecvTimeoutError::Timeout) => {
                    if !auth_timeout_sent && self.api_key.is_empty() && !self.authenticated && now_ms() - opened_at > 3000 {
                        auth_timeout_sent = true;
                        self.publish("waiting-auth", "confirm the permission request in TeamSpeak 6 → Settings → Remote Apps");
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return SessionEnd::Closed,
            }
        }
    }
}

/// Blocking run: maintains the TS6 session until `stop`.
pub fn run(
    host: String,
    port: i64,
    api_key: String,
    save_key: SaveKeyFn,
    stop: Arc<AtomicBool>,
    emit: impl Fn(OverlayState) + Send + Sync + 'static,
) {
    let emit: EmitFn = Arc::new(emit);
    let mut session = Ts6Session {
        host,
        port,
        api_key,
        save_key,
        stop: stop.clone(),
        whispers: crate::model::WhisperBook::new(emit.clone()),
        emit,
        clients: HashMap::new(),
        channels: HashMap::new(),
        own_by_connection: HashMap::new(),
        group_infos: HashMap::new(),
        chat: ChatLog::new('c'),
        connection_id: 0,
        preferred_connection_id: 0,
        own_client_id: 0,
        own_channel_id: 0,
        server_name: String::new(),
        authenticated: false,
        auth_denied: false,
        fast_reconnect: false,
        resync_requested: false,
        announced_once: false,
        noisy_diag_skipped: 0,
        cur_status: "connecting",
        cur_detail: String::new(),
    };

    while !stop.load(Ordering::SeqCst) {
        let mut was_auth = false;
        let mut session_error = String::new();
        match ws_connect(&session.host, session.port) {
            Ok((_stream, writer, rx)) => {
                session.run_session(&writer, &rx);
                was_auth = session.authenticated;
            }
            Err(err) => session_error = err,
        }
        session.authenticated = false;
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if !was_auth && session.auth_denied {
            session.publish("waiting-auth", "permission denied — allow the app in TS6 → Settings → Remote Apps");
        } else if !session_error.is_empty() {
            session.publish("offline", &format!("TeamSpeak 6 Remote Apps unreachable ({session_error})"));
        } else {
            session.publish("connecting", "");
        }
        let delay = if session.fast_reconnect {
            250
        } else if was_auth {
            1200
        } else {
            3000
        };
        session.fast_reconnect = false;
        let wake = now_ms() + delay;
        while !stop.load(Ordering::SeqCst) && now_ms() < wake {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_properties_maps_fields() {
        let base = UserState::new("7", "");
        let props = Json::parse(
            r#"{"nickname":"Alice","inputMuted":true,"flagTalking":true,"flagTalkingWithWhisper":true,
                "serverGroups":"6,12","flagAvatar":"abc","type":"QUERY"}"#,
        )
        .unwrap();
        let u = apply_properties(&base, &props);
        assert_eq!(u.nickname, "Alice");
        assert!(u.input_muted && u.talking && u.whispering);
        assert_eq!(u.server_groups, "6,12");
        assert_eq!(u.avatar_hash, "abc");
        assert!(u.is_query);
    }

    #[test]
    fn apply_properties_partial_keeps_rest() {
        let mut base = UserState::new("7", "Bob");
        base.talking = true;
        let props = Json::parse(r#"{"away":true}"#).unwrap();
        let u = apply_properties(&base, &props);
        assert_eq!(u.nickname, "Bob");
        assert!(u.talking && u.away);
        assert!(!u.whispering);
    }

    #[test]
    fn chat_event_extraction_walks_nesting() {
        // shape varies between builds — nested sender + message objects
        let payload = Json::parse(
            r#"{"type":"chatMessageReceived","message":{"text":"hi all","targetMode":1},
                "sender":{"id":100,"name":"Alice"}}"#,
        )
        .unwrap();
        // replicate the extraction against a session-less helper path via a tiny state
        let (tx, _rx) = channel();
        let emit: EmitFn = Arc::new(move |_st| {
            let _ = tx.send(());
        });
        let mut s = Ts6Session {
            host: String::new(),
            port: 0,
            api_key: String::new(),
            save_key: Arc::new(|_| {}),
            stop: Arc::new(AtomicBool::new(false)),
            whispers: crate::model::WhisperBook::new(emit.clone()),
            emit,
            clients: HashMap::new(),
            channels: HashMap::new(),
            own_by_connection: HashMap::new(),
            group_infos: HashMap::new(),
            chat: ChatLog::new('c'),
            connection_id: 0,
            preferred_connection_id: 0,
            own_client_id: 0,
            own_channel_id: 0,
            server_name: String::new(),
            authenticated: false,
            auth_denied: false,
            fast_reconnect: false,
            resync_requested: false,
            announced_once: false,
            noisy_diag_skipped: 0,
            cur_status: "live",
            cur_detail: String::new(),
        };
        s.handle_chat_event(&payload);
        let rows = s.chat.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "hi all");
        assert_eq!(rows[0].nickname, "Alice");
        assert_eq!(rows[0].user_id, "100");
        assert_eq!(rows[0].kind, ChatKind::Private);
    }
}
