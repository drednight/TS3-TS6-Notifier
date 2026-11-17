//! AppConfig — the exact JSON schema of the Node ConfigStore (interchangeable
//! config.json), with the same merge semantics: unknown keys are dropped,
//! null/undefined never overwrite, objects merge recursively.

use crate::json::Json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct TsEndpoint {
    pub host: String,
    pub port: i64,
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub theme: String,
    pub accent: String,
    pub nick_color: String,
    pub msg_color: String,
    pub talk_outline: String,
    pub whisper_delay: f64,
    pub glow: f64,
    pub opacity: f64,
    pub blur: f64,
    pub radius: f64,
    pub scale: f64,
    pub font_size: f64,
    pub corner: String,
    pub margin_x: f64,
    pub margin_y: f64,
    pub language: String,
    pub show_header: bool,
    pub show_server_name: bool,
    pub show_user_count: bool,
    pub show_banner: bool,
    pub show_avatars: bool,
    pub show_badges: bool,
    pub show_self: bool,
    pub hide_query: bool,
    pub animations: bool,
    pub eq_style: String,
    pub talking_first: bool,
    pub only_talking: bool,
    pub compact: bool,
    pub hide_empty: bool,
    pub show_whispers: bool,
    pub show_chat: bool,
    pub chat_delay: f64,
    pub chat_private: bool,
    pub show_unread_badge: bool,
    pub show_events: bool,
    pub show_ev_join: bool,
    pub show_ev_leave: bool,
    pub show_ev_move: bool,
    pub show_ev_poke: bool,
    pub show_ev_chat: bool,
    pub events_count: f64,
    pub events_delay: f64,
    pub ev_scope_join: String,
    pub ev_scope_leave: String,
    pub ev_scope_move: String,
    pub ev_scope_group: String,
    pub ev_scope_sys: String,
    pub ev_scope_chat: String,
}

impl Default for Appearance {
    fn default() -> Appearance {
        Appearance {
            theme: "glass".into(),
            accent: "#7c6cff".into(),
            nick_color: "#e8ecff".into(),
            msg_color: "#b6b6bd".into(),
            talk_outline: "#57e389".into(),
            whisper_delay: 4.0,
            glow: 0.7,
            opacity: 0.72,
            blur: 18.0,
            radius: 16.0,
            scale: 1.0,
            font_size: 13.0,
            corner: "tr".into(),
            margin_x: 18.0,
            margin_y: 18.0,
            language: "auto".into(),
            show_header: true,
            show_server_name: true,
            show_user_count: true,
            show_banner: true,
            show_avatars: true,
            show_badges: true,
            show_self: true,
            hide_query: true,
            animations: true,
            eq_style: "bars".into(),
            talking_first: true,
            only_talking: false,
            compact: false,
            hide_empty: false,
            show_whispers: true,
            show_chat: true,
            chat_delay: 8.0,
            chat_private: true,
            show_unread_badge: true,
            show_events: true,
            show_ev_join: true,
            show_ev_leave: true,
            show_ev_move: true,
            show_ev_poke: true,
            show_ev_chat: true,
            events_count: 5.0,
            events_delay: 20.0,
            ev_scope_join: "all".into(),
            ev_scope_leave: "all".into(),
            ev_scope_move: "all".into(),
            ev_scope_group: "all".into(),
            ev_scope_sys: "all".into(),
            ev_scope_chat: "all".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DesktopPrefs {
    pub click_through: bool,
    pub autostart: bool,
    pub visible: bool,
    pub seen: bool,
    pub display: String,
    pub elevate: bool,
    pub overlay_window: bool,
    pub port: i64,
}

impl Default for DesktopPrefs {
    fn default() -> DesktopPrefs {
        DesktopPrefs {
            click_through: true,
            autostart: false,
            visible: true,
            seen: false,
            display: "primary".into(),
            elevate: true,
            overlay_window: true,
            port: 48911,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassesCfg {
    pub enabled: bool,
    pub map: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub source: String,
    pub ts3: TsEndpoint,
    pub ts6: TsEndpoint,
    pub appearance: Appearance,
    pub desktop: DesktopPrefs,
    pub classes: ClassesCfg,
}

impl Default for AppConfig {
    fn default() -> AppConfig {
        let mut map = Vec::new();
        for (id, key) in [
            ("442", "archer"), ("444", "priest"), ("445", "warrior"), ("446", "mage"),
            ("447", "werewolf"), ("448", "druid"), ("449", "assassin"), ("450", "shaman"),
            ("451", "ghost"), ("452", "reaper"), ("453", "paladin"), ("454", "gunslinger"),
            ("455", "guardian"), ("456", "mystic"), ("502", "bard"), ("503", "blood"),
            ("506", "monkey"),
        ] {
            map.push((id.to_string(), key.to_string()));
        }
        AppConfig {
            source: "mock".into(),
            ts3: TsEndpoint { host: "127.0.0.1".into(), port: 25639, api_key: String::new() },
            ts6: TsEndpoint { host: "127.0.0.1".into(), port: 5899, api_key: String::new() },
            appearance: Appearance::default(),
            desktop: DesktopPrefs {
                click_through: true,
                autostart: false,
                visible: true,
                seen: false,
                display: "primary".into(),
                elevate: true,
                overlay_window: true,
                port: 48911,
            },
            classes: ClassesCfg { enabled: false, map },
        }
    }
}

/* ---------------- JSON mapping ---------------- */

fn pick_str(obj: &Json, key: &str, fallback: &str) -> String {
    obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| fallback.to_string())
}

fn pick_f64(obj: &Json, key: &str, fallback: f64) -> f64 {
    match obj.get(key) {
        Some(Json::Num(n)) => *n,
        Some(Json::Str(s)) => s.parse().unwrap_or(fallback),
        _ => fallback,
    }
}

fn pick_bool(obj: &Json, key: &str, fallback: bool) -> bool {
    obj.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback)
}

pub fn ts_endpoint_from_json(obj: &Json, base: &TsEndpoint) -> TsEndpoint {
    TsEndpoint {
        host: pick_str(obj, "host", &base.host),
        port: pick_f64(obj, "port", base.port as f64) as i64,
        api_key: pick_str(obj, "apiKey", &base.api_key),
    }
}

pub fn appearance_from_json(obj: &Json) -> Appearance {
    // Unknown keys never leak: only fields present in Appearance are read.
    let d = Appearance::default();
    if !matches!(obj, Json::Obj(_)) {
        return d;
    }
    Appearance {
        theme: pick_str(obj, "theme", &d.theme),
        accent: pick_str(obj, "accent", &d.accent),
        nick_color: pick_str(obj, "nickColor", &d.nick_color),
        msg_color: pick_str(obj, "msgColor", &d.msg_color),
        talk_outline: pick_str(obj, "talkOutline", &d.talk_outline),
        whisper_delay: pick_f64(obj, "whisperDelay", d.whisper_delay),
        glow: pick_f64(obj, "glow", d.glow),
        opacity: pick_f64(obj, "opacity", d.opacity),
        blur: pick_f64(obj, "blur", d.blur),
        radius: pick_f64(obj, "radius", d.radius),
        scale: pick_f64(obj, "scale", d.scale),
        font_size: pick_f64(obj, "fontSize", d.font_size),
        corner: pick_str(obj, "corner", &d.corner),
        margin_x: pick_f64(obj, "marginX", d.margin_x),
        margin_y: pick_f64(obj, "marginY", d.margin_y),
        language: pick_str(obj, "language", &d.language),
        show_header: pick_bool(obj, "showHeader", d.show_header),
        show_server_name: pick_bool(obj, "showServerName", d.show_server_name),
        show_user_count: pick_bool(obj, "showUserCount", d.show_user_count),
        show_banner: pick_bool(obj, "showBanner", d.show_banner),
        show_avatars: pick_bool(obj, "showAvatars", d.show_avatars),
        show_badges: pick_bool(obj, "showBadges", d.show_badges),
        show_self: pick_bool(obj, "showSelf", d.show_self),
        hide_query: pick_bool(obj, "hideQuery", d.hide_query),
        animations: pick_bool(obj, "animations", d.animations),
        eq_style: pick_str(obj, "eqStyle", &d.eq_style),
        talking_first: pick_bool(obj, "talkingFirst", d.talking_first),
        only_talking: pick_bool(obj, "onlyTalking", d.only_talking),
        compact: pick_bool(obj, "compact", d.compact),
        hide_empty: pick_bool(obj, "hideEmpty", d.hide_empty),
        show_whispers: pick_bool(obj, "showWhispers", d.show_whispers),
        show_chat: pick_bool(obj, "showChat", d.show_chat),
        chat_delay: pick_f64(obj, "chatDelay", d.chat_delay),
        chat_private: pick_bool(obj, "chatPrivate", d.chat_private),
        show_unread_badge: pick_bool(obj, "showUnreadBadge", d.show_unread_badge),
        show_events: pick_bool(obj, "showEvents", d.show_events),
        show_ev_join: pick_bool(obj, "showEvJoin", d.show_ev_join),
        show_ev_leave: pick_bool(obj, "showEvLeave", d.show_ev_leave),
        show_ev_move: pick_bool(obj, "showEvMove", d.show_ev_move),
        show_ev_poke: pick_bool(obj, "showEvPoke", d.show_ev_poke),
        show_ev_chat: pick_bool(obj, "showEvChat", d.show_ev_chat),
        events_count: pick_f64(obj, "eventsCount", d.events_count),
        events_delay: pick_f64(obj, "eventsDelay", d.events_delay),
        ev_scope_join: pick_str(obj, "evScopeJoin", &d.ev_scope_join),
        ev_scope_leave: pick_str(obj, "evScopeLeave", &d.ev_scope_leave),
        ev_scope_move: pick_str(obj, "evScopeMove", &d.ev_scope_move),
        ev_scope_group: pick_str(obj, "evScopeGroup", &d.ev_scope_group),
        ev_scope_sys: pick_str(obj, "evScopeSys", &d.ev_scope_sys),
        ev_scope_chat: pick_str(obj, "evScopeChat", &d.ev_scope_chat),
    }
}

pub fn appearance_to_json(a: &Appearance) -> Json {
    crate::json::obj(vec![
        ("theme", Json::s(a.theme.clone())),
        ("accent", Json::s(a.accent.clone())),
        ("nickColor", Json::s(a.nick_color.clone())),
        ("msgColor", Json::s(a.msg_color.clone())),
        ("talkOutline", Json::s(a.talk_outline.clone())),
        ("whisperDelay", Json::n(a.whisper_delay)),
        ("glow", Json::n(a.glow)),
        ("opacity", Json::n(a.opacity)),
        ("blur", Json::n(a.blur)),
        ("radius", Json::n(a.radius)),
        ("scale", Json::n(a.scale)),
        ("fontSize", Json::n(a.font_size)),
        ("corner", Json::s(a.corner.clone())),
        ("marginX", Json::n(a.margin_x)),
        ("marginY", Json::n(a.margin_y)),
        ("language", Json::s(a.language.clone())),
        ("showHeader", Json::b(a.show_header)),
        ("showServerName", Json::b(a.show_server_name)),
        ("showUserCount", Json::b(a.show_user_count)),
        ("showBanner", Json::b(a.show_banner)),
        ("showAvatars", Json::b(a.show_avatars)),
        ("showBadges", Json::b(a.show_badges)),
        ("showSelf", Json::b(a.show_self)),
        ("hideQuery", Json::b(a.hide_query)),
        ("animations", Json::b(a.animations)),
        ("eqStyle", Json::s(a.eq_style.clone())),
        ("talkingFirst", Json::b(a.talking_first)),
        ("onlyTalking", Json::b(a.only_talking)),
        ("compact", Json::b(a.compact)),
        ("hideEmpty", Json::b(a.hide_empty)),
        ("showWhispers", Json::b(a.show_whispers)),
        ("showChat", Json::b(a.show_chat)),
        ("chatDelay", Json::n(a.chat_delay)),
        ("chatPrivate", Json::b(a.chat_private)),
        ("showUnreadBadge", Json::b(a.show_unread_badge)),
        ("showEvents", Json::b(a.show_events)),
        ("showEvJoin", Json::b(a.show_ev_join)),
        ("showEvLeave", Json::b(a.show_ev_leave)),
        ("showEvMove", Json::b(a.show_ev_move)),
        ("showEvPoke", Json::b(a.show_ev_poke)),
        ("showEvChat", Json::b(a.show_ev_chat)),
        ("eventsCount", Json::n(a.events_count)),
        ("eventsDelay", Json::n(a.events_delay)),
        ("evScopeJoin", Json::s(a.ev_scope_join.clone())),
        ("evScopeLeave", Json::s(a.ev_scope_leave.clone())),
        ("evScopeMove", Json::s(a.ev_scope_move.clone())),
        ("evScopeGroup", Json::s(a.ev_scope_group.clone())),
        ("evScopeSys", Json::s(a.ev_scope_sys.clone())),
        ("evScopeChat", Json::s(a.ev_scope_chat.clone())),
    ])
}

impl AppConfig {
    pub fn from_json(raw: &Json) -> AppConfig {
        let base = AppConfig::default();
        if !matches!(raw, Json::Obj(_)) {
            return base;
        }
        let desktop = raw.get("desktop").unwrap_or(&Json::Null);
        let classes = raw.get("classes").unwrap_or(&Json::Null);
        let mut map = base.classes.map.clone();
        if let Some(Json::Obj(items)) = classes.get("map") {
            map = items
                .iter()
                .filter(|(_, v)| v.as_str().is_some() && v.as_str() != Some("none"))
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect();
        }
        AppConfig {
            source: pick_str(raw, "source", &base.source),
            ts3: raw.get("ts3").map(|v| ts_endpoint_from_json(v, &base.ts3)).unwrap_or(base.ts3),
            ts6: raw.get("ts6").map(|v| ts_endpoint_from_json(v, &base.ts6)).unwrap_or(base.ts6),
            appearance: raw.get("appearance").map(appearance_from_json).unwrap_or_default(),
            desktop: DesktopPrefs {
                click_through: pick_bool(desktop, "clickThrough", base.desktop.click_through),
                autostart: pick_bool(desktop, "autostart", base.desktop.autostart),
                visible: pick_bool(desktop, "visible", base.desktop.visible),
                seen: pick_bool(desktop, "seen", base.desktop.seen),
                display: pick_str(desktop, "display", &base.desktop.display),
                elevate: pick_bool(desktop, "elevate", base.desktop.elevate),
                overlay_window: pick_bool(desktop, "overlayWindow", base.desktop.overlay_window),
                port: pick_f64(desktop, "port", base.desktop.port as f64) as i64,
            },
            classes: ClassesCfg {
                enabled: pick_bool(classes, "enabled", base.classes.enabled),
                map,
            },
        }
    }

    pub fn to_json(&self) -> Json {
        let mut map_pairs = Vec::new();
        for (k, v) in &self.classes.map {
            map_pairs.push((k.clone(), Json::s(v.clone())));
        }
        crate::json::obj(vec![
            ("source", Json::s(self.source.clone())),
            ("ts3", crate::json::obj(vec![
                ("host", Json::s(self.ts3.host.clone())),
                ("port", Json::n(self.ts3.port as f64)),
                ("apiKey", Json::s(self.ts3.api_key.clone())),
            ])),
            ("ts6", crate::json::obj(vec![
                ("host", Json::s(self.ts6.host.clone())),
                ("port", Json::n(self.ts6.port as f64)),
                ("apiKey", Json::s(self.ts6.api_key.clone())),
            ])),
            ("appearance", appearance_to_json(&self.appearance)),
            ("desktop", crate::json::obj(vec![
                ("clickThrough", Json::b(self.desktop.click_through)),
                ("autostart", Json::b(self.desktop.autostart)),
                ("visible", Json::b(self.desktop.visible)),
                ("seen", Json::b(self.desktop.seen)),
                ("display", Json::s(self.desktop.display.clone())),
                ("elevate", Json::b(self.desktop.elevate)),
                ("overlayWindow", Json::b(self.desktop.overlay_window)),
                ("port", Json::n(self.desktop.port as f64)),
            ])),
            ("classes", crate::json::obj(vec![
                ("enabled", Json::b(self.classes.enabled)),
                ("map", Json::Obj(map_pairs)),
            ])),
        ])
    }

    /// Merge a patch the way the TS store does (only known keys, no nulls).
    /// Returns true when the connector must restart (source / endpoints changed).
    pub fn update_from(&mut self, patch: &Json) -> bool {
        let before = format!("{:?}|{:?}|{:?}", self.source, self.ts3, self.ts6);
        if let Json::Obj(items) = patch {
            for (key, value) in items {
                if matches!(value, Json::Null) {
                    continue;
                }
                match key.as_str() {
                    "source" => {
                        if let Some(s) = value.as_str() {
                            if matches!(s, "mock" | "ts3" | "ts6") {
                                self.source = s.to_string();
                            }
                        }
                    }
                    "ts3" => self.ts3 = ts_endpoint_from_json(value, &self.ts3),
                    "ts6" => self.ts6 = ts_endpoint_from_json(value, &self.ts6),
                    "appearance" => self.appearance = appearance_from_json(value),
                    "desktop" => {
                        self.desktop = DesktopPrefs {
                            click_through: pick_bool(value, "clickThrough", self.desktop.click_through),
                            autostart: pick_bool(value, "autostart", self.desktop.autostart),
                            visible: pick_bool(value, "visible", self.desktop.visible),
                            seen: pick_bool(value, "seen", self.desktop.seen),
                            display: pick_str(value, "display", &self.desktop.display),
                            elevate: pick_bool(value, "elevate", self.desktop.elevate),
                            overlay_window: pick_bool(value, "overlayWindow", self.desktop.overlay_window),
                            port: pick_f64(value, "port", self.desktop.port as f64) as i64,
                        };
                    }
                    "classes" => {
                        self.classes.enabled = pick_bool(value, "enabled", self.classes.enabled);
                        if let Some(map) = value.get("map") {
                            if let Json::Obj(map_items) = map {
                                for (k, v) in map_items {
                                    let val = match v.as_str() {
                                        Some("none") => None,
                                        Some(s) => Some(s.to_string()),
                                        None => None,
                                    };
                                    self.classes.map.retain(|(mk, _)| mk != k);
                                    if let Some(val) = val {
                                        self.classes.map.push((k.clone(), val));
                                    }
                                }
                            }
                        }
                    }
                    _ => {} // unknown keys are dropped
                }
            }
        }
        let after = format!("{:?}|{:?}|{:?}", self.source, self.ts3, self.ts6);
        before != after
    }
}

/* ---------------- store ---------------- */

pub struct ConfigStore {
    path: PathBuf,
    config: AppConfig,
}

impl ConfigStore {
    pub fn load(data_dir: &Path) -> ConfigStore {
        let path = data_dir.join("config.json");
        let config = fs::read_to_string(&path)
            .ok()
            .and_then(|text| Json::parse(&text).ok())
            .map(|raw| AppConfig::from_json(&raw))
            .unwrap_or_default();
        ConfigStore { path, config }
    }

    pub fn get(&self) -> &AppConfig {
        &self.config
    }

    pub fn update(&mut self, patch: &Json) -> bool {
        let restart = self.config.update_from(patch);
        self.save();
        restart
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(&self.path, self.config.to_json().to_string_pretty());
    }
}

/// Shared store handle.
pub type SharedStore = Mutex<ConfigStore>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_drops_unknown_keys_and_nulls() {
        let mut cfg = AppConfig::default();
        let patch = Json::parse(r#"{"appearance":{"glow":0.5,"bogus":1,"accent":null},"source":"ts3","junk":true}"#).unwrap();
        let restart = cfg.update_from(&patch);
        assert!(restart);
        assert_eq!(cfg.source, "ts3");
        assert_eq!(cfg.appearance.glow, 0.5);
        assert_eq!(cfg.appearance.accent, "#7c6cff"); // null never overwrites
    }

    #[test]
    fn config_roundtrip_shape() {
        let cfg = AppConfig::default();
        let text = cfg.to_json().to_string_pretty();
        let back = AppConfig::from_json(&Json::parse(&text).unwrap());
        assert_eq!(cfg, back);
    }

    #[test]
    fn appearance_only_known_fields() {
        let a = appearance_from_json(&Json::parse(r#"{"theme":"neon","hax":"x"}"#).unwrap());
        assert_eq!(a.theme, "neon");
    }
}
