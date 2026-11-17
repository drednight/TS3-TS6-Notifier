//! TS3 icon lookup — the native core serves no embedded icon database yet
//! (the web UI falls back to letter avatars), same as a missing icon id.
#![allow(dead_code)]

pub struct Ts3Icon {
    pub mime: &'static str,
    pub data: Vec<u8>,
}

pub fn find_ts3_icon(_id: i64) -> Option<Ts3Icon> {
    None
}
