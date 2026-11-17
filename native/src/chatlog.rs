//! Rolling chat/event log with tail-scan dedupe (same rules as the TS app).

use crate::model::{now_ms, ChatKind, ChatMessage};

const TAIL: usize = 12;
const DEDUPE_MS: u64 = 5000;
const CAP: usize = 60;

pub struct ChatLog {
    rows: Vec<ChatMessage>,
    seq: u64,
    prefix: char,
}

impl ChatLog {
    pub fn new(prefix: char) -> ChatLog {
        ChatLog { rows: Vec::new(), seq: 0, prefix }
    }

    /// Push a message (id/at assigned here). Dedupe-merges into the recent tail.
    pub fn push(&mut self, msg: ChatMessage) {
        let now = now_ms();
        // tail-scan dedupe
        let start = self.rows.len().saturating_sub(TAIL);
        for i in (start..self.rows.len()).rev() {
            let prev = &self.rows[i];
            if now - prev.at > DEDUPE_MS {
                break;
            }
            let same = prev.text == msg.text
                && prev.user_id == msg.user_id
                && prev.event == msg.event
                && prev.detail == msg.detail;
            if same {
                let kind = if msg.kind.rank() > prev.kind.rank() { msg.kind } else { prev.kind };
                let nickname = if !prev.nickname.is_empty() && prev.nickname != "?" {
                    prev.nickname.clone()
                } else {
                    msg.nickname.clone()
                };
                let own = prev.own.or(msg.own);
                let row = &mut self.rows[i];
                row.kind = kind;
                row.nickname = nickname;
                row.own = own;
                return;
            }
        }
        self.seq += 1;
        let mut row = msg;
        row.id = format!("{}{}", self.prefix, self.seq);
        row.at = now;
        self.rows.push(row);
        if self.rows.len() > CAP {
            let excess = self.rows.len() - CAP;
            self.rows.drain(0..excess);
        }
    }

    pub fn rows(&self) -> &[ChatMessage] {
        &self.rows
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }
}

/// Convenience builder for system event rows.
pub fn sys_row(user_id: &str, nick: &str, text: &str, event: &'static str, detail: &str, own: bool) -> ChatMessage {
    ChatMessage {
        id: String::new(),
        user_id: user_id.to_string(),
        nickname: nick.to_string(),
        text: text.to_string(),
        kind: ChatKind::System,
        at: 0,
        event: Some(event),
        detail: if detail.is_empty() { None } else { Some(detail.to_string()) },
        own: Some(own),
    }
}
