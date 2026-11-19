//! TS3 ClientQuery wire-format helpers (escape / field parse / response parse).

pub fn ts3_unescape(value: &str) -> String {
    if !value.contains('\\') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('p') => out.push('|'),
            Some('/') => out.push('/'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('v') => out.push('\u{b}'),
            Some('f') => out.push('\u{c}'),
            Some('a') => out.push('\u{7}'),
            Some('b') => out.push('\u{8}'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

pub fn ts3_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            ' ' => out.push_str("\\s"),
            '|' => out.push_str("\\p"),
            other => out.push(other),
        }
    }
    out
}

pub type Ts3Fields = Vec<(String, String)>;

pub fn f_get<'a>(fields: &'a Ts3Fields, key: &str) -> Option<&'a str> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

pub fn f_str(fields: &Ts3Fields, key: &str) -> String {
    f_get(fields, key).unwrap_or("").to_string()
}

pub fn f_int(fields: &Ts3Fields, key: &str) -> i64 {
    f_get(fields, key).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0)
}

pub fn f_bool(fields: &Ts3Fields, key: &str) -> bool {
    f_get(fields, key).map(|v| v != "0").unwrap_or(false)
}

pub fn f_has(fields: &Ts3Fields, key: &str) -> bool {
    fields.iter().any(|(k, _)| k == key)
}

pub fn ts3_parse_fields(line: &str) -> Ts3Fields {
    let mut result: Ts3Fields = Vec::new();
    for token in line.split(' ') {
        if token.is_empty() {
            continue;
        }
        match token.find('=') {
            Some(eq) => result.push((token[..eq].to_string(), ts3_unescape(&token[eq + 1..]))),
            None => result.push((token.to_string(), String::new())),
        }
    }
    result
}

pub struct Ts3Response {
    pub items: Vec<Ts3Fields>,
    pub error_id: i64,
    pub error_message: String,
}

pub fn ts3_parse_response(raw: &str) -> Ts3Response {
    let mut items: Vec<Ts3Fields> = Vec::new();
    let mut error_id = 0i64;
    let mut error_message = String::new();
    let mut data_lines: Vec<&str> = Vec::new();

    for line in raw.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("error id=") {
            let fields = ts3_parse_fields(trimmed);
            error_id = f_int(&fields, "id");
            error_message = f_str(&fields, "msg");
            continue;
        }
        data_lines.push(trimmed);
    }

    if !data_lines.is_empty() {
        let joined = data_lines.join(" ");
        for item in joined.split('|') {
            if item.trim().is_empty() {
                continue;
            }
            items.push(ts3_parse_fields(item.trim()));
        }
    }

    Ts3Response { items, error_id, error_message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_roundtrip() {
        let src = "a b|c/d\\e";
        let esc = ts3_escape(src);
        assert_eq!(esc, "a\\sb\\pc\\/d\\\\e");
        assert_eq!(ts3_unescape(&esc), src);
    }

    #[test]
    fn parse_response_items() {
        let raw = "clid=1 client_nickname=A\\sB cid=5|clid=2 client_nickname=C cid=6\nerror id=0 msg=ok";
        let res = ts3_parse_response(raw);
        assert_eq!(res.error_id, 0);
        assert_eq!(res.items.len(), 2);
        assert_eq!(f_str(&res.items[0], "client_nickname"), "A B");
        assert_eq!(f_int(&res.items[1], "clid"), 2);
    }

    #[test]
    fn parse_notify_fields() {
        let fields = ts3_parse_fields("clid=12 status=1 targetmode=2 msg=hello\\sworld");
        assert_eq!(f_int(&fields, "clid"), 12);
        assert_eq!(f_bool(&fields, "status"), true);
        assert_eq!(f_str(&fields, "msg"), "hello world");
    }
}
