//! Diagnostics ring buffer — raw TS3/TS6 traffic with secrets redacted.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const DIAG_LIMIT: usize = 400;
static DIAG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Смещение местного времени от UTC в секундах. Раньше диагностика и лента
/// показывали UTC под видом местного времени («сбилось время»).
/// Windows: GetLocalTime − GetSystemTime (пояс и летнее время учтены).
/// В тестах на Linux (в т.ч. RUSTFLAGS=--cfg tsn_win32) — заглушка 0: символы
/// kernel32 не линкуются вне Windows; на Windows всегда настоящий вызов.
#[cfg(windows)]
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

#[cfg(not(windows))]
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

/// Truncate to at most `max` BYTES at a char boundary — safe log snippets.
/// (Byte-slicing UTF-8 text at a fixed index panics on multi-byte nicks.)
pub fn snip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn hhmmss(ms: u64) -> String {
    hms_with_offset((ms / 1000) as i64, local_offset_secs())
}

pub fn local_stamp() -> String {
    let utc = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let secs = (utc + local_offset_secs()).max(0) as u64;
    let day = secs / 86_400;
    // Civil date from days-since-epoch (Howard Hinnant's algorithm).
    let z = day as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:02}.{:02}.{}, {}", d, m, y, hms_with_offset(secs as i64, 0))
}

/// Redact `apikey=...` style secrets (same rule as the TS implementation).
pub fn redact(line: &str) -> String {
    let lower = line.to_lowercase();
    for key in ["apikey", "api_key", "token", "password", "passwd"] {
        if let Some(pos) = lower.find(key) {
            // find "= " or ": " after the key
            let rest = &line[pos + key.len()..];
            if let Some(eq) = rest.find(['=', ':']) {
                let mut val_start = pos + key.len() + eq + 1;
                while val_start < line.len() && line.as_bytes()[val_start].is_ascii_whitespace() {
                    val_start += 1;
                }
                let mut val_end = val_start;
                let bytes = line.as_bytes();
                while val_end < bytes.len() && !bytes[val_end].is_ascii_whitespace() {
                    val_end += 1;
                }
                let mut out = String::with_capacity(line.len());
                out.push_str(&line[..val_start]);
                out.push_str("***");
                out.push_str(&line[val_end..]);
                return out;
            }
        }
    }
    line.to_string()
}

pub fn diag(scope: &str, line: &str) {
    let entry = format!("{} [{}] {}", hhmmss_local(), scope, redact(line));
    let mut guard = DIAG.lock().unwrap();
    guard.push(entry);
    if guard.len() > DIAG_LIMIT {
        let excess = guard.len() - DIAG_LIMIT;
        guard.drain(0..excess);
    }
}

pub fn lines_for(scope_tag: &str) -> Vec<String> {
    DIAG.lock().unwrap().iter().filter(|l| l.contains(scope_tag)).cloned().collect()
}

pub fn other_lines() -> Vec<String> {
    DIAG.lock()
        .unwrap()
        .iter()
        .filter(|l| !l.contains("[ts3]") && !l.contains("[plugin]") && !l.contains("[ts6]"))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets() {
        assert_eq!(redact("auth apikey=abc123"), "auth apikey=***");
        assert_eq!(redact("token: secret-value here"), "token: *** here");
        assert_eq!(redact("plain line"), "plain line");
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
