//! A small JSON reader and writer that keeps what FHIR needs kept.
//!
//! * Numbers keep their exact text. FHIR `decimal` is a decimal, not a
//!   float: `1.50` has two significant decimal places and must come back
//!   out as `1.50`. A float round trip would turn it into `1.5`.
//! * Objects keep their key order and every key, duplicates included, so
//!   the decoder can report a duplicate instead of silently keeping one.
//! * Errors carry a line and column.
//!
//! No dependencies and no recursion limits beyond [`MAX_DEPTH`], so hostile
//! input cannot overflow the stack.

use std::fmt;

/// How deeply arrays and objects may nest. Real FHIR resources stay far
/// below this; the limit only stops input built to exhaust the stack.
pub const MAX_DEPTH: usize = 256;

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// `null`
    Null,
    /// `true` or `false`
    Bool(bool),
    /// A number, exactly as written.
    Number(String),
    /// A string, unescaped.
    String(String),
    /// An array.
    Array(Vec<Json>),
    /// An object, keys in document order, duplicates kept.
    Object(Vec<(String, Json)>),
}

/// Why a document is not JSON, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    /// 1-based line.
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
    /// What was wrong.
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for JsonError {}

impl Json {
    /// Parse a complete document. Trailing content after the value is an
    /// error; a UTF-8 byte order mark at the start is skipped.
    pub fn parse(text: &str) -> Result<Json, JsonError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut p = Parser {
            src: text,
            bytes: text.as_bytes(),
            pos: 0,
        };
        p.ws();
        let v = p.value(0)?;
        p.ws();
        if p.pos < p.bytes.len() {
            return Err(p.error("unexpected content after the JSON value"));
        }
        Ok(v)
    }

    /// The value as compact JSON text.
    pub fn to_compact(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out, None, 0);
        out
    }

    /// The value as JSON text indented by two spaces, one member per line.
    pub fn to_pretty(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out, Some(2), 0);
        out
    }

    /// The members of an object, or `None`.
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(m) => Some(m),
            _ => None,
        }
    }

    /// The first member called `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// The string, or `None`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    /// A short name for the kind of value, for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Json::Null => "null",
            Json::Bool(_) => "a boolean",
            Json::Number(_) => "a number",
            Json::String(_) => "a string",
            Json::Array(_) => "an array",
            Json::Object(_) => "an object",
        }
    }

    /// Structural equality that ignores object key order. Numbers compare
    /// by text, so `1.50` and `1.5` differ, as they do in FHIR.
    pub fn same_as(&self, other: &Json) -> bool {
        match (self, other) {
            (Json::Object(a), Json::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.iter().any(|(k2, v2)| k == k2 && v.same_as(v2)))
            }
            (Json::Array(a), Json::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_as(y))
            }
            _ => self == other,
        }
    }
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> JsonError {
        let before = &self.src[..self.pos.min(self.src.len())];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        JsonError {
            line,
            column,
            message: message.to_string(),
        }
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.pos) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.src[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        match self.bytes.get(self.pos) {
            None => Err(self.error("unexpected end of input")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::String),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("expected a JSON value")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut members = Vec::new();
        self.ws();
        if self.eat("}") {
            return Ok(Json::Object(members));
        }
        loop {
            self.ws();
            if self.bytes.get(self.pos) != Some(&b'"') {
                return Err(self.error("expected a string key"));
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(":") {
                return Err(self.error("expected ':' after the key"));
            }
            self.ws();
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("}") {
                return Ok(Json::Object(members));
            }
            return Err(self.error("expected ',' or '}' in the object"));
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat("]") {
            return Ok(Json::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("]") {
                return Ok(Json::Array(items));
            }
            return Err(self.error("expected ',' or ']' in the array"));
        }
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.pos;
        let digits = |p: &mut Parser<'_>| {
            let s = p.pos;
            while let Some(b'0'..=b'9') = p.bytes.get(p.pos) {
                p.pos += 1;
            }
            p.pos > s
        };
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        if self.bytes.get(self.pos) == Some(&b'0') {
            self.pos += 1;
        } else if !digits(self) {
            return Err(self.error("expected digits"));
        }
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !digits(self) {
                return Err(self.error("expected digits after the decimal point"));
            }
        }
        if let Some(b'e' | b'E') = self.bytes.get(self.pos) {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.bytes.get(self.pos) {
                self.pos += 1;
            }
            if !digits(self) {
                return Err(self.error("expected digits in the exponent"));
            }
        }
        Ok(Json::Number(self.src[start..self.pos].to_string()))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let h = self
            .src
            .get(self.pos..self.pos + 4)
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| self.error("expected four hex digits after \\u"))?;
        self.pos += 4;
        u32::from_str_radix(h, 16).map_err(|_| self.error("bad \\u escape"))
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(&b) = self.bytes.get(self.pos) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.src[start..self.pos]);
            match self.bytes.get(self.pos) {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let c = match self.bytes.get(self.pos) {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.pos += 1;
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if !self.eat("\\u") {
                                    return Err(self.error("lone high surrogate"));
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.error("bad low surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.error("lone low surrogate"));
                            } else {
                                hi
                            };
                            out.push(
                                char::from_u32(code).ok_or_else(|| self.error("bad \\u escape"))?,
                            );
                            continue;
                        }
                        _ => return Err(self.error("unknown escape")),
                    };
                    out.push(c);
                    self.pos += 1;
                }
                Some(_) => return Err(self.error("control character in a string")),
            }
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn newline(out: &mut String, indent: Option<usize>, level: usize) {
    if let Some(n) = indent {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', n * level));
    }
}

fn write_value(v: &Json, out: &mut String, indent: Option<usize>, level: usize) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => out.push_str(n),
        Json::String(s) => write_string(s, out),
        Json::Array(items) if items.is_empty() => out.push_str("[]"),
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_value(item, out, indent, level + 1);
            }
            newline(out, indent, level);
            out.push(']');
        }
        Json::Object(m) if m.is_empty() => out.push_str("{}"),
        Json::Object(m) => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_string(k, out);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(v, out, indent, level + 1);
            }
            newline(out, indent, level);
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_keep_their_text() {
        let v = Json::parse(r#"{"value": 1.50, "big": 12345678901234567890.000}"#).unwrap();
        assert_eq!(
            v.to_compact(),
            r#"{"value":1.50,"big":12345678901234567890.000}"#
        );
    }

    #[test]
    fn escapes_and_surrogates_round_trip() {
        let v = Json::parse(r#""a\"b\\c\né😀/""#).unwrap();
        assert_eq!(v, Json::String("a\"b\\c\né😀/".into()));
        assert_eq!(Json::parse(&v.to_compact()).unwrap(), v);
    }

    #[test]
    fn duplicates_and_order_are_kept() {
        let v = Json::parse(r#"{"b":1,"a":2,"b":3}"#).unwrap();
        let keys: Vec<_> = v
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(keys, ["b", "a", "b"]);
    }

    #[test]
    fn errors_say_where() {
        let e = Json::parse("{\n  \"a\": [1, 2,]\n}").unwrap_err();
        assert_eq!((e.line, e.column), (2, 14), "points at the stray ]");
        assert!(Json::parse("[1] x").is_err());
        assert!(Json::parse("01").is_err());
        assert!(Json::parse(r#""\ud800""#).is_err());
        let deep = "[".repeat(MAX_DEPTH + 2);
        assert!(Json::parse(&deep).is_err());
    }

    #[test]
    fn pretty_printing_is_stable() {
        let v = Json::parse(r#"{"a":[1,{"b":null}],"c":{}}"#).unwrap();
        assert_eq!(
            v.to_pretty(),
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": null\n    }\n  ],\n  \"c\": {}\n}"
        );
        assert!(Json::parse(&v.to_pretty()).unwrap().same_as(&v));
    }
}
