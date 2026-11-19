//! Diagnostics ring buffer — raw TS3/TS6 traffic with secrets redacted.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const DIAG_LIMIT: usize = 400;
static DIAG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn hhmmss_local() -> String {
    // Local wall-clock without external crates: format UTC time as a fallback
    // when no TZ database is consulted (the desktop build can refine this).
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let day_secs = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day_secs / 3600, (day_secs % 3600) / 60, day_secs % 60)
}

pub fn hhmmss(ms: u64) -> String {
    let day_secs = (ms / 1000) % 86_400;
    format!("{:02}:{:02}:{:02}", day_secs / 3600, (day_secs % 3600) / 60, day_secs % 60)
}

pub fn local_stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
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
    format!("{:02}.{:02}.{}, {}", d, m, y, hhmmss(secs * 1000))
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
}
