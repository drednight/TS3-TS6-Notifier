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
    /// последнее talk-событие: (момент, talking, whispering) — свежее событие
    /// главнее данных снимка (гонка в обе стороны), дальше решает флаг снимка
    talk_event_at: HashMap<i64, (u64, bool, bool)>,
    /// отложенные лёгкие refresh после подключения (~1.5 и ~4 с)
    refresh_followups: Vec<u64>,
    /// до этого момента join/leave-строки не пишутся (всплеск при подключении)
    join_quiet_until: u64,
    /// следующий refresh — новая точка отсчёта (подключение / смена сервера)
    baseline_pending: bool,
    /// сколько notify-строк получено за сессию (доходят ли события вообще)
    events_seen: u64,
    events_stat_base: u64,
    /// когда в следующий раз перепроверить регистрацию на события
    notify_recheck_at: u64,
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
        self.events_seen += 1;
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
                // Ревью 2026-11 (v2.0.32): status=1 «говорит», status=2 «говорит
                // при выключенном микрофоне» (SDK) — НЕ шёпот. Шёпот — только
                // isreceivedwhisper (поле notifytalkstatuschange по документации
                // ClientQuery; старое имя whisper оставлено для совместимости).
                let talking = f_int(&fields, "status") >= 1;
                let whisper = talking && (f_bool(&fields, "isreceivedwhisper") || f_bool(&fields, "whisper"));
                if !in_scope {
                    if talking && schandlerid > 0 && self.own_clid_by_handler.get(&schandlerid) == Some(&clid) {
                        self.active_schandler_id = schandlerid;
                        self.schedule_refresh(true);
                    }
                    return;
                }
                self.talk_event_at.insert(clid, (now_ms(), talking, whisper));
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
                // Ревью 2026-11 (v2.0.32): устройства недоступны и «только
                // вывод» — как в clientlist, складываем в mute-бейджи сразу,
                // а не через лёгкий refresh.
                if f_has(&fields, "client_input_hardware") && !f_bool(&fields, "client_input_hardware") {
                    u.input_muted = true;
                }
                if f_has(&fields, "client_output_hardware") && !f_bool(&fields, "client_output_hardware") {
                    u.output_muted = true;
                }
                if f_has(&fields, "client_outputonly_muted") && f_bool(&fields, "client_outputonly_muted") {
                    u.output_muted = true;
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
                if f_has(&fields, "client_input_hardware") && !f_bool(&fields, "client_input_hardware") {
                    u.input_muted = true;
                }
                if f_has(&fields, "client_output_hardware") && !f_bool(&fields, "client_output_hardware") {
                    u.output_muted = true;
                }
                if f_has(&fields, "client_outputonly_muted") && f_bool(&fields, "client_outputonly_muted") {
                    u.output_muted = true;
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
                let quiet = now_ms() < self.join_quiet_until;
                if is_new && !quiet && !nick.is_empty() && !is_query && clid != self.own_clid {
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
            "notifyconnectstatuschange" => {
                if in_scope {
                    // новое подключение: всплеск enterview/диффов — не новости
                    self.join_quiet_until = now_ms() + 5000;
                    self.baseline_pending = true;
                    self.schedule_refresh(true);
                    // Ревью 2026-11 (v2.0.32): список клиентов дозаполняется
                    // после подключения — два лёгких догоняющих refresh.
                    let now = now_ms();
                    self.refresh_followups.push(now + 1500);
                    self.refresh_followups.push(now + 4000);
                }
            }
            "notifychannelcreated" | "notifychanneledited" | "notifychanneldeleted" | "notifychannelmoved"
            | "notifyserverupdated" | "notifyserveredited" => {
                if in_scope {
                    self.schedule_refresh(true);
                }
            }
            "notifycurrentserverconnectionchanged" => {
                let hid = f_int(&fields, "schandlerid");
                if hid > 0 && hid != self.active_schandler_id {
                    self.active_schandler_id = hid;
                    self.join_quiet_until = now_ms() + 5000;
                    self.baseline_pending = true;
                    self.schedule_refresh(true);
                }
            }
            _ => {}
        }
    }

    /// Замер длительности full_refresh'ей + строка в диагностику раз в минуту.
    /// Итоговые флаги говорения для клиента: свежее событие (≤3 с) главнее
    /// (гонка снимка и событий в обе стороны); иначе — флаг из clientlist,
    /// который самовосстанавливает пропущенные события.
    fn talk_flags(&self, id: i64, flag_talking: bool, prev: Option<&UserState>, now: u64) -> (bool, bool) {
        if let Some((t, talking, whispering)) = self.talk_event_at.get(&id) {
            if now.saturating_sub(*t) < 3000 {
                return (*talking, *whispering);
            }
        }
        // whisper из снимка не известен — гасим вместе с речью
        let whispering = prev.map(|u| u.whispering).unwrap_or(false) && flag_talking;
        (flag_talking, whispering)
    }

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
                        "refresh: {} calls/min ({} heavy), total {} ms, max {} ms, notify events {}",
                        self.refresh_stat_calls, self.refresh_stat_heavy, self.refresh_stat_ms, self.refresh_stat_max,
                        self.events_seen.saturating_sub(self.events_stat_base)
                    ),
                );
            }
            self.refresh_stat_at = now;
            self.events_stat_base = self.events_seen;
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

/// ClientQuery завершает строки парой "\n\r" (не "\r\n"): read_line режет по
/// '\n', поэтому КАЖДАЯ следующая строка приходит с ведущим '\r'. Без обрезки
/// с обеих сторон `starts_with("notify")` не срабатывал, и все события
/// молча терялись — оставался только опрос раз в 15 с.
fn clean_line(raw: &str) -> String {
    raw.trim_matches(|c| c == '\r' || c == '\n').to_string()
}

/// Писать ли в ленту join/leave по разнице снимков: не на первом снимке, не
/// при смене сервера/клиента, не в «тихом окне» после подключения и не при
/// массовом изменении (flood-правило v1.4.7, как в plugin.rs).
fn diff_announce_allowed(announced_once: bool, baseline: bool, quiet: bool, mass: bool) -> bool {
    announced_once && !baseline && !quiet && !mass
}

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
                    let trimmed = clean_line(&line);
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

    /// Регистрация на события ClientQuery. Раньше ответ с ошибкой молча
    /// игнорировался (проверялась только доставка команды).
    fn register_notifications(&mut self, link: &mut Link) -> Result<(), String> {
        let res = self.command(link, "clientnotifyregister schandlerid=0 event=any", 2500)?;
        if res.error_id == 0 {
            diag("ts3", "clientnotifyregister schandlerid=0 event=any: ok");
            return Ok(());
        }
        diag(
            "ts3",
            &format!("clientnotifyregister schandlerid=0 failed: id={} msg={}", res.error_id, res.error_message),
        );
        // запасной путь: каждая вкладка отдельно
        let list = self.safe(link, "serverconnectionhandlerlist", 2500);
        let mut ids: Vec<i64> = list.items.iter().map(|i| f_int(i, "schandlerid")).filter(|n| *n > 0).collect();
        if ids.is_empty() {
            ids.push(1);
        }
        for hid in ids {
            let r = self.safe(link, &format!("clientnotifyregister schandlerid={hid} event=any"), 2500);
            diag("ts3", &format!("clientnotifyregister schandlerid={hid}: id={}", r.error_id));
        }
        Ok(())
    }

    fn find_active_handler(&mut self, link: &mut Link) {
        // Ревью 2026-11 (v2.0.32): один запрос currentschandlerid вместо
        // обхода всех вкладок (по 3–4 запроса на каждую), как в референсе.
        let cur = self.safe(link, "currentschandlerid", 2500);
        if let Some(item) = cur.items.first() {
            let hid = f_int(item, "schandlerid").max(f_int(item, "id"));
            if hid > 0 {
                self.active_schandler_id = hid;
                return;
            }
        }
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
            // Ревью 2026-11 (v2.0.32): НЕ стираем готовый ростер — при
            // переподключении меняем только статус, без мигания «connecting»
            // поверх пустого списка.
            self.publish("offline", "not connected to a TeamSpeak server");
            // Рецензия 2026-11: раньше повтор случался только по событию или
            // через 45 с — «offline» мог висеть минутами. Пробуем снова быстро.
            self.schedule_refresh(false);
            self.note_refresh_stats(now_ms().saturating_sub(t0), heavy);
            return;
        }
        let self_item = who.items[0].clone();
        let prev_own_clid = self.own_clid;
        self.own_clid = f_int(&self_item, "clid");
        self.own_cid = f_int(&self_item, "cid");
        // другой клиент/сервер или первый успех после offline — новая точка
        // отсчёта: список людей «появляется» целиком, это не события входа
        let mut baseline = std::mem::take(&mut self.baseline_pending) || prev_own_clid != self.own_clid;
        self.own_clid_by_handler.insert(self.active_schandler_id, self.own_clid);

        if heavy {
            let server = self.safe(link, "servervariable virtualserver_name", 2500);
            if let Some(item) = server.items.first() {
                let name = f_str(item, "virtualserver_name");
                if !self.server_name.is_empty() && !name.is_empty() && name != self.server_name {
                    baseline = true;
                }
                self.server_name = name;
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
                // Рецензия 2026-11 (v2.0.28): гонка «снимок затирает события».
                // v2.0.31: свежее talk-событие (≤3 с) главнее снимка; иначе
                // говорение берётся из флага client_flag_talking — так
                // пропущенные события долечиваются следующим refresh'ем.
                let (talking, whispering) = self.talk_flags(id, u.talking, Some(prev_u), now_ms());
                u.talking = talking;
                u.whispering = whispering;
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
        if baseline {
            self.join_quiet_until = now_ms() + 5000;
        }
        // flood-правило: массовое изменение списка и всплеск сразу после
        // подключения — не новости входа/выхода
        let added_n = clients.keys().copied().filter(|id| !self.clients.contains_key(id)).count();
        let removed_n = self.clients.keys().copied().filter(|id| !clients.contains_key(id)).count();
        let mass_limit = 5f64.max(self.clients.len() as f64 * 0.3);
        let mass = (added_n as f64) > mass_limit || (removed_n as f64) > mass_limit;
        let announce = diff_announce_allowed(self.announced_once, baseline, now_ms() < self.join_quiet_until, mass);
        if announce {
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
        self.publish("live", "ClientQuery");
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

        self.events_seen = 0;
        self.events_stat_base = 0;
        self.join_quiet_until = now_ms() + 5000;
        self.register_notifications(link)?;
        self.find_active_handler(link);
        self.full_refresh(link);
        self.refresh_fails = 0;
        self.notify_recheck_at = now_ms() + 45_000;

        // event + timer loop
        let mut next_refresh = now_ms() + 15_000; // лёгкая страховка (15 с)
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
            if !self.refresh_followups.is_empty() {
                let now = now_ms();
                let mut i = 0;
                while i < self.refresh_followups.len() {
                    if now >= self.refresh_followups[i] {
                        self.refresh_followups.remove(i);
                        self.light_refresh(link);
                    } else {
                        i += 1;
                    }
                }
            }
            if now >= next_refresh {
                next_refresh = now + 15_000;
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
            // страховка регистрации: ни одного notify-события за сессию —
            // регистрируемся заново (раз в минуту) и пишем это в диагностику
            if self.events_seen == 0 && self.cur_status == "live" && now >= self.notify_recheck_at {
                self.notify_recheck_at = now + 60_000;
                let hid = self.active_schandler_id;
                let r = self.safe(link, &format!("clientnotifyregister schandlerid={hid} event=any"), 2500);
                diag("ts3", &format!("no notify events yet - re-register schandlerid={hid}: id={}", r.error_id));
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
            talk_event_at: HashMap::new(),
            refresh_followups: Vec::new(),
            join_quiet_until: 0,
            baseline_pending: false,
            events_seen: 0,
            events_stat_base: 0,
            notify_recheck_at: 0,
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
            talk_event_at: HashMap::new(),
            refresh_followups: Vec::new(),
            join_quiet_until: 0,
            baseline_pending: false,
            events_seen: 0,
            events_stat_base: 0,
            notify_recheck_at: 0,
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



    /// Ревью 2026-11: шёпот — только isreceivedwhisper; status=2 — говорит
    /// при выключенном микрофоне (не шёпот).
    #[test]
    fn talk_status_semantics_per_sdk() {
        let mut s = session_for_test();
        s.own_cid = 3;
        s.handle_event("notifycliententerview clid=5 ctid=3 client_nickname=Alfa client_type=0");
        // status=2 + isreceivedwhisper=0: говорящий, но не шёпот
        s.handle_event("notifytalkstatuschange clid=5 status=2 isreceivedwhisper=0");
        let u = s.clients.get(&5).unwrap();
        assert!(u.talking, "status=2 means talking with muted mic");
        assert!(!u.whispering, "status=2 must NOT be whisper");
        // status=1 + isreceivedwhisper=1: шёпот
        s.handle_event("notifytalkstatuschange clid=5 status=1 isreceivedwhisper=1");
        let u = s.clients.get(&5).unwrap();
        assert!(u.talking && u.whispering);
        // status=0: тишина
        s.handle_event("notifytalkstatuschange clid=5 status=0 isreceivedwhisper=0");
        let u = s.clients.get(&5).unwrap();
        assert!(!u.talking && !u.whispering);
    }

    /// Ревью 2026-11: notifyclientupdated разбирает hardware/outputonly_muted.
    #[test]
    fn clientupdated_folds_hardware_and_outputonly() {
        let mut s = session_for_test();
        s.own_cid = 3;
        s.handle_event("notifycliententerview clid=5 ctid=3 client_nickname=Alfa client_type=0");
        s.handle_event("notifyclientupdated clid=5 client_input_hardware=0");
        assert!(s.clients.get(&5).unwrap().input_muted, "input hardware off = muted badge");
        s.handle_event("notifyclientupdated clid=5 client_output_hardware=0");
        assert!(s.clients.get(&5).unwrap().output_muted, "output hardware off = muted badge");
        s.handle_event("notifyclientupdated clid=5 client_outputonly_muted=1");
        assert!(s.clients.get(&5).unwrap().output_muted, "outputonly_muted = muted badge");
    }

    /// connectstatuschange планирует догоняющие лёгкие refresh (~1.5 и ~4 с).
    #[test]
    fn connect_event_schedules_followup_refreshes() {
        let mut s = session_for_test();
        assert!(s.refresh_followups.is_empty());
        s.handle_event("notifyconnectstatuschange schandlerid=1 status=8");
        assert_eq!(s.refresh_followups.len(), 2, "two followups expected");
        assert!(s.refresh_pending_at > 0, "immediate heavy refresh queued too");
    }

    /// v2.0.31: свежее talk-событие главнее снимка; иначе флаг снимка
    /// самовосстанавливает пропущенные события (говорение не «залипает»).
    #[test]
    fn talk_flags_prefer_recent_event_then_snapshot_flag() {
        let mut s = session_for_test();
        let mut prev = UserState::new("5", "Alfa");
        // свежее событие (0.5 с назад): говорение = значение события, не снимка
        s.talk_event_at.insert(5, (10_000, true, true));
        let (t, w) = s.talk_flags(5, false, Some(&prev), 10_500);
        assert!(t && w, "recent talk event must beat a stale snapshot flag");
        // старое событие (5 с назад): решает флаг снимка — и true, и false
        s.talk_event_at.insert(5, (5_000, true, true));
        let (t, _) = s.talk_flags(5, false, Some(&prev), 10_500);
        assert!(!t, "stale event must not stick; snapshot flag self-heals");
        let (t, _) = s.talk_flags(5, true, Some(&prev), 10_500);
        assert!(t, "snapshot flag recovers talking when events were missed");
        // без событий вообще — работает флаг
        let (t, w) = s.talk_flags(9, true, Some(&prev), 10_500);
        assert!(t && !w, "flag drives talking; whisper only lingers with prev && talking");
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

    /// Строки ClientQuery заканчиваются на "\n\r": после read_line следующая
    /// строка начинается с '\r'. Регрессия «события не обрабатывались».
    #[test]
    fn clean_line_strips_leading_cr() {
        assert_eq!(clean_line("\rnotifytalkstatuschange clid=1 status=1\n"), "notifytalkstatuschange clid=1 status=1");
        assert_eq!(clean_line("\rerror id=0 msg=ok\n"), "error id=0 msg=ok");
        assert_eq!(clean_line("TS3 Client\n"), "TS3 Client");
        let pieces = split_glued(&clean_line("\rerror id=0 msg=ok\n"));
        assert_eq!(pieces, vec!["error id=0 msg=ok".to_string()]);
    }

    /// enterview в «тихом окне» после подключения создаёт клиента, но не пишет join.
    #[test]
    fn enterview_is_silent_during_connect_burst() {
        let mut s = session_for_test();
        s.own_cid = 3;
        s.join_quiet_until = now_ms() + 10_000;
        s.handle_event("notifycliententerview clid=5 ctid=3 client_nickname=Alfa client_type=0");
        assert!(s.clients.contains_key(&5), "user is created");
        assert!(!s.chat.rows().iter().any(|c| c.event == Some("join")), "no join row in the quiet window");
        s.join_quiet_until = 0;
        s.handle_event("notifycliententerview clid=6 ctid=3 client_nickname=Bravo client_type=0");
        assert!(
            s.chat.rows().iter().any(|c| c.event == Some("join") && c.nickname == "Bravo"),
            "join row after the window"
        );
    }

    #[test]
    fn diff_announce_rules() {
        assert!(diff_announce_allowed(true, false, false, false));
        assert!(!diff_announce_allowed(false, false, false, false), "first snapshot is a silent baseline");
        assert!(!diff_announce_allowed(true, true, false, false), "server/client change is a new baseline");
        assert!(!diff_announce_allowed(true, false, true, false), "quiet window after connect");
        assert!(!diff_announce_allowed(true, false, false, true), "mass change is not news");
    }

    /// Hardware-флаги из enterview попадают в mute-бейджи сразу.
    #[test]
    fn enterview_folds_hardware_flags() {
        let mut s = session_for_test();
        s.own_cid = 3;
        s.handle_event("notifycliententerview clid=5 ctid=3 client_nickname=Alfa client_type=0 client_input_hardware=0 client_output_hardware=1");
        let u = s.clients.get(&5).unwrap();
        assert!(u.input_muted && !u.output_muted);
    }

    /// Сквозная регрессия: фальшивый ClientQuery с настоящими терминаторами
    /// "\n\r" шлёт notifytalkstatuschange — говорящий должен засветиться
    /// сразу, а не через опрос раз в 15 с.
    #[test]
    fn talk_event_over_nr_wire_reaches_state() {
        use std::net::TcpListener;
        use std::sync::Mutex;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port() as i64;
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let reader = BufReader::new(sock.try_clone().unwrap());
            let _ = sock.write_all(
                b"TS3 Client\n\rWelcome to the TeamSpeak 3 ClientQuery interface\n\rselected schandlerid=1\n\r",
            );
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let cmd = line.trim().to_string();
                let name = cmd.split(' ').next().unwrap_or("");
                let body = match name {
                    "whoami" => "clid=5 cid=3\n\r",
                    "currentschandlerid" => "schandlerid=1\n\r",
                    "servervariable" => "virtualserver_name=Srv\n\r",
                    "channellist" => "cid=3 pid=0 channel_name=Home\n\r",
                    "clientlist" => "clid=5 cid=3 client_nickname=Me client_type=0 client_flag_talking=0|clid=6 cid=3 client_nickname=Bob client_type=0 client_flag_talking=0\n\r",
                    "servergrouplist" => "sgid=1 name=Admin iconid=0\n\r",
                    _ => "",
                };
                if sock.write_all(format!("{body}error id=0 msg=ok\n\r").as_bytes()).is_err() {
                    break;
                }
                if name == "servergrouplist" {
                    thread::sleep(Duration::from_millis(200));
                    let _ = sock.write_all(b"notifytalkstatuschange schandlerid=1 status=1 isreceivedwhisper=0 clid=6\n\r");
                }
            }
        });
        let states: Arc<Mutex<Vec<OverlayState>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = states.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        thread::spawn(move || {
            Ts3Connector::run("127.0.0.1".to_string(), port, "k".to_string(), stop2, move |st| {
                sink.lock().unwrap().push(st);
            });
        });
        let mut seen = false;
        for _ in 0..80 {
            thread::sleep(Duration::from_millis(50));
            let all = states.lock().unwrap();
            if all.iter().any(|st| st.users.iter().any(|u| u.nickname == "Bob" && u.talking)) {
                seen = true;
                break;
            }
        }
        stop.store(true, Ordering::SeqCst);
        assert!(seen, "notifytalkstatuschange must light up Bob within 4 s (wire lines end with \\n\\r)");
    }
}
