//! Shared state model — the exact JSON contract of the web overlay.

use crate::json::{obj, Json};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub type SourceKind = &'static str; // "mock" | "ts3" | "ts6"
pub type StatusCode = &'static str; // connecting | live | waiting-auth | offline | error

#[derive(Debug, Clone, PartialEq)]
pub struct UserState {
    pub id: String,
    pub nickname: String,
    pub channel_id: i64,
    pub talking: bool,
    pub whispering: bool,
    pub input_muted: bool,
    pub output_muted: bool,
    pub away: bool,
    pub away_message: String,
    pub priority_speaker: bool,
    pub channel_commander: bool,
    pub is_self: bool,
    pub is_query: bool,
    pub avatar_hash: String,
    pub server_groups: String,
    /// Only on rows from `OverlayState.whisperers`: the channel this person
    /// whispers FROM (they are not in your channel).
    pub whisper_channel: Option<String>,
}

impl UserState {
    pub fn new(id: impl Into<String>, nickname: impl Into<String>) -> UserState {
        UserState {
            id: id.into(),
            nickname: nickname.into(),
            channel_id: 0,
            talking: false,
            whispering: false,
            input_muted: false,
            output_muted: false,
            away: false,
            away_message: String::new(),
            priority_speaker: false,
            channel_commander: false,
            is_self: false,
            is_query: false,
            avatar_hash: String::new(),
            server_groups: String::new(),
            whisper_channel: None,
        }
    }

    pub fn to_json(&self) -> Json {
        obj(vec![
            ("id", Json::s(self.id.clone())),
            ("nickname", Json::s(self.nickname.clone())),
            ("channelId", Json::i(self.channel_id)),
            ("talking", Json::b(self.talking)),
            ("whispering", Json::b(self.whispering)),
            ("inputMuted", Json::b(self.input_muted)),
            ("outputMuted", Json::b(self.output_muted)),
            ("away", Json::b(self.away)),
            ("awayMessage", Json::s(self.away_message.clone())),
            ("prioritySpeaker", Json::b(self.priority_speaker)),
            ("channelCommander", Json::b(self.channel_commander)),
            ("isSelf", Json::b(self.is_self)),
            ("isQuery", Json::b(self.is_query)),
            ("avatarHash", Json::s(self.avatar_hash.clone())),
            ("serverGroups", Json::s(self.server_groups.clone())),
        ]
        .into_iter()
        .chain(
            self.whisper_channel
                .as_ref()
                .map(|c| ("whisperChannel", Json::s(c.clone())))
                .into_iter(),
        )
        .collect())
    }
}

/// How long a finished whisper stays in the state (the UI fades it out).
pub const WHISPER_GRACE_MS: u64 = 1200;

pub type Emit = Arc<dyn Fn(OverlayState) + Send + Sync>;

/// Who whispered recently — including people from other channels.
/// Mirrors `whisperHits` in the TS connectors (note + grace expiry).
#[derive(Default)]
pub struct WhisperHits {
    hits: std::collections::HashMap<String, (u64, bool)>, // id -> (at, active)
}

impl WhisperHits {
    /// Record whisper activity. Returns true when the state must be re-emitted.
    pub fn note(&mut self, id: &str, whispering: bool) -> bool {
        if id.is_empty() {
            return false;
        }
        let now = now_ms();
        // prune expired while we are here
        self.hits
            .retain(|_, (at, active)| *active || now.saturating_sub(*at) <= WHISPER_GRACE_MS);
        let prev = self.hits.get(id).copied();
        if whispering {
            self.hits.insert(id.to_string(), (now, true));
            if self.hits.len() > 64 {
                let cutoff = now.saturating_sub(60_000);
                self.hits.retain(|_, (at, active)| *active || *at >= cutoff);
            }
            return !matches!(prev, Some((_, true)));
        }
        if matches!(prev, Some((_, true))) {
            self.hits.insert(id.to_string(), (now, false));
            return true;
        }
        false
    }

    /// Ids to surface as outside whisperers: not on the roster, not self,
    /// active or still inside the grace window.
    pub fn ids(&self, own_id: &str, on_roster: &dyn Fn(&str) -> bool) -> Vec<String> {
        let now = now_ms();
        let mut out: Vec<String> = self
            .hits
            .iter()
            .filter(|(id, (at, active))| {
                (*active || now.saturating_sub(*at) <= WHISPER_GRACE_MS)
                    && id.as_str() != own_id
                    && !on_roster(id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        out.sort();
        out
    }

    pub fn alive_ids(&self) -> std::collections::HashSet<String> {
        let now = now_ms();
        self.hits
            .iter()
            .filter(|(_, (at, active))| *active || now.saturating_sub(*at) <= WHISPER_GRACE_MS)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Whisper book shared by a connector session: hit list + last published
/// state + the delayed cleanup re-emit (mirrors `whisperTimer` in TS).
pub struct WhisperBook {
    hits: Mutex<WhisperHits>,
    last: Mutex<Option<OverlayState>>,
    emit: Emit,
}

impl WhisperBook {
    pub fn new(emit: Emit) -> Arc<WhisperBook> {
        Arc::new(WhisperBook {
            hits: Mutex::new(WhisperHits::default()),
            last: Mutex::new(None),
            emit,
        })
    }

    /// Record whisper activity; a finished whisper schedules one cleanup
    /// re-emit so the row really leaves the state (tests wait 1.6 s).
    pub fn note(book: &Arc<Self>, id: &str, whispering: bool) {
        let ended = book.hits.lock().unwrap().note(id, whispering) && !whispering;
        if ended {
            let me = book.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(WHISPER_GRACE_MS + 200));
                let alive = me.hits.lock().unwrap().alive_ids();
                let mut guard = me.last.lock().unwrap();
                if let Some(st) = guard.as_mut() {
                    let before = st.whisperers.len();
                    st.whisperers.retain(|u| alive.contains(&u.id));
                    if st.whisperers.len() != before {
                        let st = st.clone();
                        *guard = Some(st.clone());
                        (me.emit)(st);
                    }
                }
            });
        }
    }

    /// Outside-whisperer ids (not roster, not self).
    pub fn ids(&self, own_id: &str, on_roster: &dyn Fn(&str) -> bool) -> Vec<String> {
        self.hits.lock().unwrap().ids(own_id, on_roster)
    }

    /// Store + emit a state through this book.
    pub fn publish(book: &Arc<Self>, st: OverlayState) {
        *book.last.lock().unwrap() = Some(st.clone());
        (book.emit)(st);
    }
}

/// Shape an outside whisperer row (whisperers are talking + whispering).
pub fn outside_whisperer(base: Option<UserState>, id: &str, channel_name: String) -> Option<UserState> {
    if base.as_ref().map(|u| u.is_query).unwrap_or(false) {
        return None;
    }
    let mut u = base.unwrap_or_else(|| UserState::new(id, format!("#{}", id)));
    u.id = id.to_string();
    u.talking = true;
    u.whispering = true;
    u.is_self = false;
    u.whisper_channel = Some(channel_name);
    Some(u)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChatKind { Channel, Private, Server, System, Poke }

impl ChatKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatKind::Channel => "channel",
            ChatKind::Private => "private",
            ChatKind::Server => "server",
            ChatKind::System => "system",
            ChatKind::Poke => "poke",
        }
    }
    pub fn rank(self) -> i32 {
        match self {
            ChatKind::Private => 3,
            ChatKind::Server => 2,
            ChatKind::Channel => 1,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub user_id: String,
    pub nickname: String,
    pub text: String,
    pub kind: ChatKind,
    pub at: u64,
    pub event: Option<&'static str>, // join | leave | move | poke | group
    pub detail: Option<String>,
    pub own: Option<bool>,
}

impl ChatMessage {
    pub fn to_json(&self) -> Json {
        let mut pairs = vec![
            ("id", Json::s(self.id.clone())),
            ("userId", Json::s(self.user_id.clone())),
            ("nickname", Json::s(self.nickname.clone())),
            ("text", Json::s(self.text.clone())),
            ("kind", Json::s(self.kind.as_str())),
            ("at", Json::i(self.at as i64)),
        ];
        if let Some(e) = self.event { pairs.push(("event", Json::s(e))); }
        if let Some(d) = &self.detail { pairs.push(("detail", Json::s(d.clone()))); }
        if let Some(o) = self.own { pairs.push(("own", Json::b(o))); }
        obj(pairs)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerGroup {
    pub id: i64,
    pub name: String,
    pub icon_id: i64,
}

impl ServerGroup {
    pub fn to_json(&self) -> Json {
        obj(vec![
            ("id", Json::i(self.id)),
            ("name", Json::s(self.name.clone())),
            ("iconId", Json::i(self.icon_id)),
        ])
    }
}

#[derive(Debug, Clone)]
pub struct OverlayState {
    pub source: SourceKind,
    pub status: StatusCode,
    pub detail: String,
    pub server_name: String,
    pub channel_name: String,
    pub channel_id: String,
    pub self_id: Option<String>,
    pub users: Vec<UserState>,
    /// People who whispered to you (or to your channel) from ANOTHER channel —
    /// they are not part of `users`; the overlay lists them under «Шёпот».
    pub whisperers: Vec<UserState>,
    pub groups: Vec<ServerGroup>,
    pub chat: Vec<ChatMessage>,
    pub updated_at: u64,
}

impl OverlayState {
    pub fn empty(source: SourceKind, status: StatusCode, detail: &str) -> OverlayState {
        OverlayState {
            source,
            status,
            detail: detail.into(),
            server_name: String::new(),
            channel_name: String::new(),
            channel_id: String::new(),
            self_id: None,
            users: Vec::new(),
            whisperers: Vec::new(),
            groups: Vec::new(),
            chat: Vec::new(),
            updated_at: now_ms(),
        }
    }

    pub fn to_json(&self) -> Json {
        obj(vec![
            ("source", Json::s(self.source)),
            ("status", Json::s(self.status)),
            ("detail", Json::s(self.detail.clone())),
            ("serverName", Json::s(self.server_name.clone())),
            ("channelName", Json::s(self.channel_name.clone())),
            ("channelId", Json::s(self.channel_id.clone())),
            ("selfId", match &self.self_id { Some(s) => Json::s(s.clone()), None => Json::Null }),
            ("users", Json::Arr(self.users.iter().map(|u| u.to_json()).collect())),
            ("whisperers", Json::Arr(self.whisperers.iter().map(|u| u.to_json()).collect())),
            ("groups", Json::Arr(self.groups.iter().map(|g| g.to_json()).collect())),
            ("chat", Json::Arr(self.chat.iter().map(|c| c.to_json()).collect())),
            ("updatedAt", Json::i(self.updated_at as i64)),
        ])
    }
}
