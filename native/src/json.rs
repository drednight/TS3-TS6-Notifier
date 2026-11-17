//! Minimal JSON value + parser + serializer (zero external crates).

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn s(v: impl Into<String>) -> Json { Json::Str(v.into()) }
    pub fn n(v: impl Into<f64>) -> Json { Json::Num(v.into()) }
    pub fn i(v: i64) -> Json { Json::Num(v as f64) }
    pub fn b(v: bool) -> Json { Json::Bool(v) }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self { Json::Str(s) => Some(s), _ => None }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self { Json::Bool(b) => Some(*b), _ => None }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self { Json::Num(n) => Some(*n), _ => None }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self { Json::Num(n) => Some(*n as i64), _ => None }
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self { Json::Arr(a) => Some(a), _ => None }
    }
    /// string | number -> String (matches the TS `str()` helper).
    pub fn loose_str(&self) -> String {
        match self {
            Json::Str(s) => s.clone(),
            Json::Num(n) => fmt_num(*n),
            Json::Bool(b) => if *b { "1" } else { "0" }.into(),
            _ => String::new(),
        }
    }
    /// number | string(number) -> i64 (matches the TS `num()` helper).
    pub fn loose_i64(&self) -> i64 {
        match self {
            Json::Num(n) => *n as i64,
            Json::Str(s) => s.trim().parse::<i64>().unwrap_or(0),
            _ => 0,
        }
    }

    pub fn to_string(&self) -> String {
        let mut out = String::new();
        write_compact(self, &mut out);
        out
    }

    pub fn to_string_pretty(&self) -> String {
        let mut out = String::new();
        write_pretty(self, 0, &mut out);
        out
    }

    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser { b: text.as_bytes(), i: 0 };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() { return Err(format!("trailing data at {}", p.i)); }
        Ok(v)
    }
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{}", n);
        s
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => { let _ = write!(out, "\\u{:04x}", c as u32); }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_compact(v: &Json, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(n) => out.push_str(&fmt_num(*n)),
        Json::Str(s) => write_str(s, out),
        Json::Arr(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 { out.push(','); }
                write_compact(item, out);
            }
            out.push(']');
        }
        Json::Obj(items) => {
            out.push('{');
            for (i, (k, val)) in items.iter().enumerate() {
                if i > 0 { out.push(','); }
                write_str(k, out);
                out.push(':');
                write_compact(val, out);
            }
            out.push('}');
        }
    }
}

fn write_pretty(v: &Json, depth: usize, out: &mut String) {
    match v {
        Json::Arr(a) if !a.is_empty() => {
            out.push_str("[\n");
            for (i, item) in a.iter().enumerate() {
                if i > 0 { out.push_str(",\n"); }
                for _ in 0..depth + 1 { out.push_str("  "); }
                write_pretty(item, depth + 1, out);
            }
            out.push('\n');
            for _ in 0..depth { out.push_str("  "); }
            out.push(']');
        }
        Json::Obj(items) if !items.is_empty() => {
            out.push_str("{\n");
            for (i, (k, val)) in items.iter().enumerate() {
                if i > 0 { out.push_str(",\n"); }
                for _ in 0..depth + 1 { out.push_str("  "); }
                write_str(k, out);
                out.push_str(": ");
                write_pretty(val, depth + 1, out);
            }
            out.push('\n');
            for _ in 0..depth { out.push_str("  "); }
            out.push('}');
        }
        other => write_compact(other, out),
    }
}

struct Parser<'a> { b: &'a [u8], i: usize }

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        match *self.b.get(self.i).ok_or("unexpected end")? {
            b'n' => self.lit("null", Json::Null),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'"' => Ok(Json::Str(self.string()?)),
            b'[' => self.array(),
            b'{' => self.object(),
            _ => self.number(),
        }
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("bad literal at {}", self.i))
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let c = *self.b.get(self.i).ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or("bad escape")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let cp = self.hex4()?;
                            // surrogate pair
                            if (0xD800..0xDC00).contains(&cp) {
                                if self.b.get(self.i) == Some(&b'\\') && self.b.get(self.i + 1) == Some(&b'u') {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    let c = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    out.push(char::from_u32(c).unwrap_or('\u{fffd}'));
                                } else {
                                    out.push('\u{fffd}');
                                }
                            } else {
                                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                            }
                        }
                        _ => return Err("bad escape".into()),
                    }
                }
                _ => {
                    // UTF-8 continuation: copy raw bytes of the codepoint
                    let start = self.i - 1;
                    let len = utf8_len(c);
                    self.i = start + len;
                    let slice = self.b.get(start..self.i).ok_or("bad utf8")?;
                    out.push_str(std::str::from_utf8(slice).map_err(|_| "bad utf8")?);
                }
            }
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let s = std::str::from_utf8(self.b.get(self.i..self.i + 4).ok_or("bad \\u")?)
            .map_err(|_| "bad \\u")?;
        self.i += 4;
        u32::from_str_radix(s, 16).map_err(|_| "bad \\u".into())
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
            self.i += 1;
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad number")?;
        s.parse::<f64>().map(Json::Num).map_err(|_| format!("bad number '{s}'"))
    }
    fn array(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut out = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b']') { self.i += 1; return Ok(Json::Arr(out)); }
        loop {
            self.ws();
            out.push(self.value()?);
            self.ws();
            match *self.b.get(self.i).ok_or("unterminated array")? {
                b',' => self.i += 1,
                b']' => { self.i += 1; return Ok(Json::Arr(out)); }
                _ => return Err(format!("bad array at {}", self.i)),
            }
        }
    }
    fn object(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut out: Vec<(String, Json)> = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b'}') { self.i += 1; return Ok(Json::Obj(out)); }
        loop {
            self.ws();
            if *self.b.get(self.i).ok_or("unterminated object")? != b'"' {
                return Err(format!("bad key at {}", self.i));
            }
            let key = self.string()?;
            self.ws();
            if *self.b.get(self.i).ok_or("bad object")? != b':' {
                return Err(format!("expected ':' at {}", self.i));
            }
            self.i += 1;
            self.ws();
            let val = self.value()?;
            // last duplicate wins (like JS object spread would not, but JSON.parse in JS keeps the LAST pair — match that)
            out.retain(|(k, _)| k != &key);
            out.push((key, val));
            self.ws();
            match *self.b.get(self.i).ok_or("unterminated object")? {
                b',' => self.i += 1,
                b'}' => { self.i += 1; return Ok(Json::Obj(out)); }
                _ => return Err(format!("bad object at {}", self.i)),
            }
        }
    }
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 { 1 } else if b >> 5 == 0b110 { 2 } else if b >> 4 == 0b1110 { 3 } else { 4 }
}

/// Tiny JSON-object builder used by the wire layer.
pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let src = r#"{"a":1,"b":"x\ny","c":[true,null,2.5],"d":{"e":"\u041f"}} "#;
        let v = Json::parse(src.trim()).unwrap();
        assert_eq!(v.get("a").unwrap().as_i64(), Some(1));
        assert_eq!(v.get("b").unwrap().as_str(), Some("x\ny"));
        assert_eq!(v.get("d").unwrap().get("e").unwrap().as_str(), Some("П"));
        let out = v.to_string();
        assert_eq!(Json::parse(&out).unwrap(), v);
    }

    #[test]
    fn loose_helpers() {
        let v = Json::parse(r#"{"clid":"42","s":1,"name":"x"}"#).unwrap();
        assert_eq!(v.get("clid").unwrap().loose_i64(), 42);
        assert_eq!(v.get("s").unwrap().loose_str(), "1");
        assert_eq!(v.get("name").unwrap().loose_str(), "x");
    }

    #[test]
    fn pretty_is_two_space() {
        let v = obj(vec![("a", Json::i(1))]);
        assert_eq!(v.to_string_pretty(), "{\n  \"a\": 1\n}");
    }
}
