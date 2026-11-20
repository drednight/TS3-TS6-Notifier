#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Патч поверх TS3-TS6 Notifier v2.0.32 (ветка arena/763b595a-ts3-ts6-notifier,
коммит 8a0456e). Запускать из КОРНЯ репозитория:

    python ts3-fix-2.0.33.py --check     # только проверить, что все места найдены
    python ts3-fix-2.0.33.py             # применить

Правки применяются «всё или ничего»: если хоть один фрагмент не найден или
найден не то количество раз — ни один файл не меняется.

Что чинит:
  1. ts3.rs  — ClientQuery завершает строки парой "\\n\\r"; каждая следующая
     строка приходила с ведущим '\r', `starts_with("notify")` не срабатывал и
     ВСЕ события молча терялись (остался только опрос раз в 15 с).
  2. ts3.rs  — ошибка в ответе на clientnotifyregister раньше игнорировалась;
     теперь логируется, есть запасная регистрация и самолечение, если за
     сессию не пришло ни одного события.
  3. ts3.rs  — всплеск «зашёл на сервер» при подключении (join-строки на каждого):
     тихое окно после подключения, смена сервера = новая точка отсчёта,
     flood-правило для массовых изменений.
  4. ts3.rs  — hardware-флаги из enterview попадают в mute-бейджи сразу.
  5. diag.rs — время в диагностике и в ленте показывалось в UTC под видом
     местного; теперь учитывается пояс и летнее время (Windows).
  6. overlay.rs — события колеса мыши считаются по монотонным часам.
"""
import sys
import pathlib

EDITS = []


def E(path, old, new, count=1):
    EDITS.append((path, old, new, count))


TS3 = "native/src/ts3.rs"
DIAG = "native/src/diag.rs"
OVERLAY = "native/src/overlay.rs"

# ---------------------------------------------------------------- ts3.rs ---

# 1. новые поля сессии
E(TS3,
  r'''    /// отложенные лёгкие refresh после подключения (~1.5 и ~4 с)
    refresh_followups: Vec<u64>,
''',
  r'''    /// отложенные лёгкие refresh после подключения (~1.5 и ~4 с)
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
''')

# инициализация полей (Ts3Connector::run и тестовая сессия)
E(TS3,
  r'''            refresh_followups: Vec::new(),
''',
  r'''            refresh_followups: Vec::new(),
            join_quiet_until: 0,
            baseline_pending: false,
            events_seen: 0,
            events_stat_base: 0,
            notify_recheck_at: 0,
''',
  count=2)

# 2. счётчик событий
E(TS3,
  r'''    fn handle_event(&mut self, line: &str) {
        let space = line.find(' ').unwrap_or(line.len());
''',
  r'''    fn handle_event(&mut self, line: &str) {
        self.events_seen += 1;
        let space = line.find(' ').unwrap_or(line.len());
''')

# 3. enterview: hardware-флаги (после mute-флагов, поэтому не затираются)
E(TS3,
  r'''                if f_has(&fields, "client_flag_talking") {
                    u.talking = f_bool(&fields, "client_flag_talking");
                }
                if f_has(&fields, "client_is_priority_speaker") {
                    u.priority_speaker = f_bool(&fields, "client_is_priority_speaker");
                }
                if f_has(&fields, "client_is_channel_commander") {
''',
  r'''                if f_has(&fields, "client_input_hardware") && !f_bool(&fields, "client_input_hardware") {
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
''')

# 4. enterview: join-строка не пишется в «тихом окне» после подключения
E(TS3,
  r'''                if is_new && !nick.is_empty() && !is_query && clid != self.own_clid {
                    let in_our = ctid > 0 && ctid == self.own_cid;
''',
  r'''                let quiet = now_ms() < self.join_quiet_until;
                if is_new && !quiet && !nick.is_empty() && !is_query && clid != self.own_clid {
                    let in_our = ctid > 0 && ctid == self.own_cid;
''')

# 5. подключение / смена вкладки — тихое окно + новая точка отсчёта
E(TS3,
  r'''                if in_scope {
                    self.schedule_refresh(true);
                    // Ревью 2026-11 (v2.0.32): список клиентов дозаполняется
''',
  r'''                if in_scope {
                    // новое подключение: всплеск enterview/диффов — не новости
                    self.join_quiet_until = now_ms() + 5000;
                    self.baseline_pending = true;
                    self.schedule_refresh(true);
                    // Ревью 2026-11 (v2.0.32): список клиентов дозаполняется
''')

E(TS3,
  r'''                if hid > 0 && hid != self.active_schandler_id {
                    self.active_schandler_id = hid;
                    self.schedule_refresh(true);
                }
''',
  r'''                if hid > 0 && hid != self.active_schandler_id {
                    self.active_schandler_id = hid;
                    self.join_quiet_until = now_ms() + 5000;
                    self.baseline_pending = true;
                    self.schedule_refresh(true);
                }
''')

# 6. диагностика: сколько notify-событий дошло
E(TS3,
  r'''                        "refresh: {} calls/min ({} heavy), total {} ms, max {} ms",
                        self.refresh_stat_calls, self.refresh_stat_heavy, self.refresh_stat_ms, self.refresh_stat_max
''',
  r'''                        "refresh: {} calls/min ({} heavy), total {} ms, max {} ms, notify events {}",
                        self.refresh_stat_calls, self.refresh_stat_heavy, self.refresh_stat_ms, self.refresh_stat_max,
                        self.events_seen.saturating_sub(self.events_stat_base)
''')

E(TS3,
  r'''            self.refresh_stat_at = now;
            self.refresh_stat_calls = 0;
''',
  r'''            self.refresh_stat_at = now;
            self.events_stat_base = self.events_seen;
            self.refresh_stat_calls = 0;
''')

# 7. ГЛАВНОЕ: строки ClientQuery заканчиваются на "\n\r"
E(TS3,
  r'''/* ---------------- connection plumbing ---------------- */
''',
  r'''/* ---------------- connection plumbing ---------------- */

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
''')

E(TS3,
  r'''                    let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
''',
  r'''                    let trimmed = clean_line(&line);
''')

# 8. регистрация на события: проверка ответа + запасной путь
E(TS3,
  r'''    fn find_active_handler(&mut self, link: &mut Link) {
''',
  r'''    /// Регистрация на события ClientQuery. Раньше ответ с ошибкой молча
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
''')

E(TS3,
  r'''        self.command(link, "clientnotifyregister schandlerid=0 event=any", 2500)?;
        self.find_active_handler(link);
        self.full_refresh(link);
        self.refresh_fails = 0;
''',
  r'''        self.events_seen = 0;
        self.events_stat_base = 0;
        self.join_quiet_until = now_ms() + 5000;
        self.register_notifications(link)?;
        self.find_active_handler(link);
        self.full_refresh(link);
        self.refresh_fails = 0;
        self.notify_recheck_at = now_ms() + 45_000;
''')

E(TS3,
  r'''            if now >= next_keepalive {
                next_keepalive = now + 20_000;
                self.safe(link, "version", 2500);
            }
''',
  r'''            if now >= next_keepalive {
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
''')

# 9. refresh: точка отсчёта (смена клиента/сервера) и flood-правило
E(TS3,
  r'''        let self_item = who.items[0].clone();
        self.own_clid = f_int(&self_item, "clid");
        self.own_cid = f_int(&self_item, "cid");
''',
  r'''        let self_item = who.items[0].clone();
        let prev_own_clid = self.own_clid;
        self.own_clid = f_int(&self_item, "clid");
        self.own_cid = f_int(&self_item, "cid");
        // другой клиент/сервер или первый успех после offline — новая точка
        // отсчёта: список людей «появляется» целиком, это не события входа
        let mut baseline = std::mem::take(&mut self.baseline_pending) || prev_own_clid != self.own_clid;
''')

E(TS3,
  r'''            let server = self.safe(link, "servervariable virtualserver_name", 2500);
            if let Some(item) = server.items.first() {
                self.server_name = f_str(item, "virtualserver_name");
            }
''',
  r'''            let server = self.safe(link, "servervariable virtualserver_name", 2500);
            if let Some(item) = server.items.first() {
                let name = f_str(item, "virtualserver_name");
                if !self.server_name.is_empty() && !name.is_empty() && name != self.server_name {
                    baseline = true;
                }
                self.server_name = name;
            }
''')

E(TS3,
  r'''        // diff-announce missed joins/leaves (after the first silent baseline)
        if self.announced_once {
''',
  r'''        // diff-announce missed joins/leaves (after the first silent baseline)
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
''')

# 10. тесты
E(TS3,
  r'''        assert!(s.refresh_heavy, "heavy sticks while pending");
    }
}
''',
  r'''        assert!(s.refresh_heavy, "heavy sticks while pending");
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
''')

# ---------------------------------------------------------------- diag.rs ---

E(DIAG,
  r'''fn hhmmss_local() -> String {
    // Local wall-clock without external crates: format UTC time as a fallback
    // when no TZ database is consulted (the desktop build can refine this).
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let day_secs = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day_secs / 3600, (day_secs % 3600) / 60, day_secs % 60)
}
''',
  r'''/// Смещение местного времени от UTC в секундах. Раньше диагностика и лента
/// показывали UTC под видом местного времени («сбилось время»).
/// Windows: GetLocalTime − GetSystemTime (пояс и летнее время учтены).
#[cfg(any(windows, tsn_win32))]
pub fn local_offset_secs() -> i64 {
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SysTime {
        year: u16,
        month: u16,
        dow: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[cfg_attr(windows, link(name = "kernel32"))]
    extern "system" {
        fn GetLocalTime(t: *mut SysTime);
        fn GetSystemTime(t: *mut SysTime);
    }
    let mut local = SysTime::default();
    let mut utc = SysTime::default();
    unsafe {
        GetLocalTime(&mut local);
        GetSystemTime(&mut utc);
    }
    let l = local.hour as i64 * 3600 + local.minute as i64 * 60 + local.second as i64;
    let u = utc.hour as i64 * 3600 + utc.minute as i64 * 60 + utc.second as i64;
    let mut d = l - u;
    if d > 14 * 3600 {
        d -= 86_400;
    } else if d < -12 * 3600 {
        d += 86_400;
    }
    // два вызова могут попасть на границу секунды — округляем до 15 минут
    (d + 450).div_euclid(900) * 900
}

#[cfg(not(any(windows, tsn_win32)))]
pub fn local_offset_secs() -> i64 {
    0
}

/// Время суток (чч:мм:сс) для UTC-секунд со сдвигом пояса.
pub fn hms_with_offset(utc_secs: i64, offset: i64) -> String {
    let day_secs = (utc_secs + offset).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day_secs / 3600, (day_secs % 3600) / 60, day_secs % 60)
}

fn hhmmss_local() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    hms_with_offset(secs as i64, local_offset_secs())
}
''')

E(DIAG,
  r'''pub fn hhmmss(ms: u64) -> String {
    let day_secs = (ms / 1000) % 86_400;
    format!("{:02}:{:02}:{:02}", day_secs / 3600, (day_secs % 3600) / 60, day_secs % 60)
}
''',
  r'''pub fn hhmmss(ms: u64) -> String {
    hms_with_offset((ms / 1000) as i64, local_offset_secs())
}
''')

E(DIAG,
  r'''pub fn local_stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let day = secs / 86_400;
''',
  r'''pub fn local_stamp() -> String {
    let utc = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let secs = (utc + local_offset_secs()).max(0) as u64;
    let day = secs / 86_400;
''')

E(DIAG,
  r'''    format!("{:02}.{:02}.{}, {}", d, m, y, hhmmss(secs * 1000))
''',
  r'''    format!("{:02}.{:02}.{}, {}", d, m, y, hms_with_offset(secs as i64, 0))
''')

E(DIAG,
  r'''        assert_eq!(redact("plain line"), "plain line");
    }
}
''',
  r'''        assert_eq!(redact("plain line"), "plain line");
    }

    /// «Сбилось время»: диагностика показывала UTC, а не местное время.
    #[test]
    fn local_time_applies_utc_offset() {
        // 08:32:51 UTC: в поясе UTC+1 → 09:32:51, в UTC+3 → 11:32:51
        let utc = 8 * 3600 + 32 * 60 + 51;
        assert_eq!(hms_with_offset(utc, 3600), "09:32:51");
        assert_eq!(hms_with_offset(utc, 3 * 3600), "11:32:51");
        // переход через полночь в обе стороны
        assert_eq!(hms_with_offset(30 * 60, -3600), "23:30:00");
        assert_eq!(hms_with_offset(23 * 3600 + 30 * 60, 3600), "00:30:00");
    }
}
''')

# ------------------------------------------------------------- overlay.rs ---

E(OVERLAY,
  r'''    fn wheel_now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
''',
  r'''    /// Монотонное время (как now_ms во всём приложении): скачок системных
    /// часов не должен ни выбрасывать, ни «залипать» события колеса.
    fn wheel_now_ms() -> u64 {
        crate::model::now_ms()
    }
''')


# ----------------------------------------------------------------- runner ---

def main():
    check_only = "--check" in sys.argv[1:]
    root = pathlib.Path(".")
    texts = {}
    crlf = {}
    problems = []

    for path, old, new, count in EDITS:
        p = root / path
        if path not in texts:
            if not p.is_file():
                problems.append(f"{path}: файл не найден (запускайте из корня репозитория)")
                texts[path] = None
                continue
            raw = p.read_bytes().decode("utf-8")
            crlf[path] = "\r\n" in raw
            texts[path] = raw
        text = texts[path]
        if text is None:
            continue
        o, n = old, new
        if crlf[path]:
            o, n = o.replace("\n", "\r\n"), n.replace("\n", "\r\n")
        found = text.count(o)
        if found != count:
            first = old.strip().splitlines()[0][:70]
            if found == 0 and text.count(n) >= 1:
                problems.append(f"{path}: уже применено? «{first}»")
            else:
                problems.append(f"{path}: ожидалось {count}, найдено {found}: «{first}»")
            continue
        texts[path] = text.replace(o, n)

    if problems:
        print("ПАТЧ НЕ ПРИМЕНЁН — ни один файл не изменён:")
        for line in problems:
            print("  -", line)
        sys.exit(1)

    if check_only:
        print(f"OK: все {len(EDITS)} правок находят свои места (файлы не менялись).")
        return

    for path, text in texts.items():
        (root / path).write_bytes(text.encode("utf-8"))
        print("изменён:", path)
    print(f"Готово: {len(EDITS)} правок. Дальше: cd native && cargo test && cargo build --release")


if __name__ == "__main__":
    main()
