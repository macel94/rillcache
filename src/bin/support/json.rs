//! Small, bounded JSON reader for benchmark evidence and report manifests.

use std::collections::BTreeMap;

pub(super) type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Json {
    Object(BTreeMap<String, Json>),
    Array(Vec<Json>),
    String(String),
    Number(String),
    Bool(bool),
    Null,
}

impl Json {
    pub(super) fn get(&self, key: &str) -> Result<&Self> {
        match self {
            Self::Object(map) => map.get(key).ok_or_else(|| format!("missing field {key}")),
            _ => Err("expected JSON object".into()),
        }
    }

    pub(super) fn text(&self) -> Result<&str> {
        match self {
            Self::String(s) => Ok(s),
            _ => Err("expected JSON string".into()),
        }
    }

    pub(super) fn integer(&self) -> Result<u64> {
        match self {
            Self::Number(s) => s.parse().map_err(|_| "expected unsigned integer".into()),
            _ => Err("expected JSON number".into()),
        }
    }

    pub(super) fn number(&self) -> Result<f64> {
        match self {
            Self::Number(s) => s.parse().map_err(|_| "invalid JSON number".into()),
            _ => Err("expected JSON number".into()),
        }
    }

    pub(super) fn boolean(&self) -> Result<bool> {
        match self {
            Self::Bool(value) => Ok(*value),
            _ => Err("expected JSON boolean".into()),
        }
    }

    pub(super) fn array(&self) -> Result<&[Self]> {
        match self {
            Self::Array(values) => Ok(values),
            _ => Err("expected JSON array".into()),
        }
    }
}

pub(super) fn quoted(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                write!(out, "\\u{:04x}", c as u32).expect("String write");
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(super) fn parse(text: &str) -> Result<Json> {
    if text.len() > 2 * 1024 * 1024 {
        return Err("JSON exceeds 2 MiB".into());
    }
    let mut reader = Reader {
        bytes: text.as_bytes(),
        at: 0,
        nodes: 0,
    };
    let value = reader.value(0)?;
    reader.space();
    if reader.at != reader.bytes.len() {
        return Err("trailing JSON data".into());
    }
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    nodes: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while self
            .peek()
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json> {
        self.nodes += 1;
        if depth > 32 || self.nodes > 100_000 {
            return Err("JSON complexity limit exceeded".into());
        }
        self.space();
        match self.peek() {
            Some(b'{') => {
                self.at += 1;
                self.space();
                let mut values = BTreeMap::new();
                if self.take(b'}') {
                    return Ok(Json::Object(values));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.space();
                    if !self.take(b':') {
                        return Err("expected JSON colon".into());
                    }
                    let value = self.value(depth + 1)?;
                    if values.insert(key, value).is_some() {
                        return Err("duplicate JSON key".into());
                    }
                    self.space();
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err("expected JSON object separator".into());
                    }
                }
                Ok(Json::Object(values))
            }
            Some(b'[') => {
                self.at += 1;
                self.space();
                let mut values = Vec::new();
                if self.take(b']') {
                    return Ok(Json::Array(values));
                }
                loop {
                    values.push(self.value(depth + 1)?);
                    self.space();
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err("expected JSON array separator".into());
                    }
                }
                Ok(Json::Array(values))
            }
            Some(b'"') => self.string().map(Json::String),
            Some(b'-' | b'0'..=b'9') => self.numeric(),
            _ => {
                for (literal, value) in [
                    (b"true".as_slice(), Json::Bool(true)),
                    (b"false", Json::Bool(false)),
                    (b"null", Json::Null),
                ] {
                    if self.bytes[self.at..].starts_with(literal) {
                        self.at += literal.len();
                        return Ok(value);
                    }
                }
                Err("invalid JSON value".into())
            }
        }
    }

    fn digits(&mut self) -> Result<()> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        if start == self.at {
            Err("missing JSON number digits".into())
        } else {
            Ok(())
        }
    }

    fn numeric(&mut self) -> Result<Json> {
        let start = self.at;
        self.take(b'-');
        if !self.take(b'0') {
            self.digits()?;
        }
        if self.take(b'.') {
            self.digits()?;
        }
        if self.take(b'e') || self.take(b'E') {
            if !self.take(b'+') {
                self.take(b'-');
            }
            self.digits()?;
        }
        let number =
            std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| "invalid number")?;
        if !number.parse::<f64>().is_ok_and(f64::is_finite) {
            return Err("nonfinite JSON number".into());
        }
        Ok(Json::Number(number.into()))
    }

    fn hex(&mut self) -> Result<u32> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = self
                .peek()
                .and_then(|b| char::from(b).to_digit(16))
                .ok_or("invalid Unicode escape")?;
            self.at += 1;
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String> {
        if !self.take(b'"') {
            return Err("expected JSON string".into());
        }
        let mut bytes = Vec::new();
        loop {
            let c = self.peek().ok_or("unterminated JSON string")?;
            self.at += 1;
            match c {
                b'"' => {
                    return String::from_utf8(bytes)
                        .map_err(|_| "invalid UTF-8 JSON string".into());
                }
                b'\\' => {
                    let escape = self.peek().ok_or("truncated JSON escape")?;
                    self.at += 1;
                    match escape {
                        b'"' | b'\\' | b'/' => bytes.push(escape),
                        b'b' => bytes.push(8),
                        b'f' => bytes.push(12),
                        b'n' => bytes.push(b'\n'),
                        b'r' => bytes.push(b'\r'),
                        b't' => bytes.push(b'\t'),
                        b'u' => {
                            let first = self.hex()?;
                            let code = if (0xd800..=0xdbff).contains(&first) {
                                if !self.take(b'\\') || !self.take(b'u') {
                                    return Err("missing low surrogate".into());
                                }
                                let last = self.hex()?;
                                if !(0xdc00..=0xdfff).contains(&last) {
                                    return Err("invalid low surrogate".into());
                                }
                                0x10000 + ((first - 0xd800) << 10) + last - 0xdc00
                            } else {
                                first
                            };
                            let character = char::from_u32(code).ok_or("invalid Unicode scalar")?;
                            bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        _ => return Err("invalid JSON escape".into()),
                    }
                }
                0..=31 => return Err("unescaped JSON control character".into()),
                _ => bytes.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json_without_losing_large_integer_precision() {
        let json = parse(
            r#"{"n":18446744073709551615,"s":"\ud83d\ude80é","f":-1.25e+3,"a":[true,false,null]}"#,
        )
        .unwrap();
        assert_eq!(json.get("n").unwrap().integer().unwrap(), u64::MAX);
        assert_eq!(json.get("s").unwrap().text().unwrap(), "🚀é");
        assert_eq!(json.get("f").unwrap().number().unwrap(), -1250.0);
        for text in ["\"a\\\n\0é🚀", "", "[not code]"] {
            assert_eq!(parse(&quoted(text)).unwrap().text().unwrap(), text);
        }
    }

    #[test]
    fn rejects_ambiguous_malformed_and_unbounded_json() {
        for text in [
            "",
            "01",
            "-01",
            "1.",
            "1e",
            "1e9999",
            "NaN",
            "+1",
            "[1,]",
            "{}[]",
            "truefalse",
            "{\"a\":1,\"a\":2}",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "\"\\ud800\\u0041\"",
            "\"\n\"",
            "\"\\x01\"",
            "{",
            "[",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
        assert!(parse(&format!("{}0{}", "[".repeat(40), "]".repeat(40))).is_err());
        assert!(parse(&" ".repeat(2 * 1024 * 1024 + 1)).is_err());
    }

    #[test]
    fn deterministic_arbitrary_json_never_panics() {
        let mut random = 9u32;
        for length in 0..1024 {
            let bytes: Vec<_> = (0..length % 257)
                .map(|_| {
                    random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                    ((random >> 24) as u8) & 127
                })
                .collect();
            let _ = parse(std::str::from_utf8(&bytes).unwrap());
        }
    }
}
