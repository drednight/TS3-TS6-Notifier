//! v2.0.34: автоопределение источника данных.
//!
//! Правило выбора: у кого выше оценка доступности; при равенстве — TS3.
//! TS3 получает 3, если ClientQuery отвечает и в приветствии видна активная
//! сессия (`selected schandlerid > 0`), и 2 — если просто отвечает.
//! TS6 — 2, если порт Remote Apps принимает соединение. 0 — недоступен.
//! Если доступны оба — сессионный TS3 важнее, иначе живой TS6 важнее
//! «пустого» TS3, в котором даже не выбран сервер.

use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Ts3,
    Ts6,
}

/// Оценка доступности TS3 по приветствию ClientQuery.
/// 3 — есть активная сессия (`selected schandlerid=N`, N > 0); 2 — просто жив
/// (соединение уже установлено — этого достаточно, даже без приветствия).
pub fn ts3_greeting_score(greeting: &str) -> u8 {
    for line in greeting.split(['\n', '\r']) {
        if let Some(rest) = line.trim().strip_prefix("selected schandlerid=") {
            if rest.trim().parse::<i64>().unwrap_or(0) > 0 {
                return 3;
            }
        }
    }
    2
}

/// Выбор источника по оценкам; равенство — в пользу TS3 (проверенный путь).
pub fn pick_source(ts3_score: u8, ts6_score: u8) -> Source {
    if ts3_score >= ts6_score {
        Source::Ts3
    } else {
        Source::Ts6
    }
}

/// Проба TS3: короткое соединение к ClientQuery + чтение приветствия.
pub fn probe_ts3(host: &str, port: i64) -> u8 {
    let Ok(addr) = format!("{host}:{port}").to_socket_addrs() else { return 0 };
    let mut stream = None;
    for a in addr {
        if let Ok(s) = TcpStream::connect_timeout(&a, Duration::from_millis(500)) {
            stream = Some(s);
            break;
        }
    }
    let Some(mut stream) = stream else { return 0 };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
    let mut buf = [0u8; 1024];
    let mut acc = String::new();
    for _ in 0..3 {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.contains('\n') {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    ts3_greeting_score(&acc)
}

/// Проба TS6: порт Remote Apps принимает соединение — источник жив.
pub fn probe_ts6(host: &str, port: i64) -> u8 {
    let Ok(addr) = format!("{host}:{port}").to_socket_addrs() else { return 0 };
    for a in addr {
        if TcpStream::connect_timeout(&a, Duration::from_millis(500)).is_ok() {
            return 2;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_score_prefers_live_session() {
        assert_eq!(ts3_greeting_score("TS3 Client\nselected schandlerid=3\n"), 3);
        assert_eq!(ts3_greeting_score("\rTS3 Client\r\nselected schandlerid=0\r\n"), 2);
        assert_eq!(ts3_greeting_score("TS3 Client\n"), 2);
        assert_eq!(ts3_greeting_score(""), 2);
    }

    #[test]
    fn pick_prefers_session_then_ts3_on_tie() {
        assert_eq!(pick_source(3, 2), Source::Ts3);
        assert_eq!(pick_source(2, 2), Source::Ts3, "tie goes to the battle-tested path");
        assert_eq!(pick_source(0, 2), Source::Ts6, "live TS6 beats an idle TS3");
        assert_eq!(pick_source(2, 0), Source::Ts3);
        assert_eq!(pick_source(0, 0), Source::Ts3, "nothing up: TS3 retries internally");
    }

    #[test]
    fn probe_sees_a_local_listener() {
        use std::net::TcpListener;
        use std::io::Write;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port() as i64;
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let _ = s.write_all(b"TS3 Client\r\nselected schandlerid=7\r\n");
                std::thread::sleep(Duration::from_millis(300));
            }
        });
        assert_eq!(probe_ts3("127.0.0.1", port), 3);
        assert_eq!(probe_ts6("127.0.0.1", port), 2);
        // закрытый порт — недоступен
        assert_eq!(probe_ts6("127.0.0.1", 1), 0);
    }
}
