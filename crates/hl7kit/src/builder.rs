//! Building messages.
//!
//! The builder is structural: you set fields, components and subcomponents
//! as plain text and the builder inserts the delimiters and escapes any
//! delimiter characters that appear inside values. It never lets you write a
//! delimiter by accident, and the output always parses back with
//! [`Message::parse`](crate::Message) to the same values.
//!
//! ```
//! use hl7kit::builder::{Builder, Value};
//! use hl7kit::Message;
//!
//! let mut b = Builder::new();
//! b.segment("MSH")
//!     .set(3, "RIS")
//!     .set(4, "HOSP")
//!     .set(9, Value::components(["ORM", "O01", "ORM_O01"]))
//!     .set(10, "MSG0001")
//!     .set(11, "P")
//!     .set(12, "2.5.1");
//! b.segment("PID")
//!     .set(1, "1")
//!     .set(3, Value::components(["4MR1", "", "", "HOSP", "MR"]))
//!     .set(5, Value::components(["Doe", "Jane & John"]));
//! b.segment("ZDS")
//!     .set(1, Value::components(["1.2.3", "", "Application", "DICOM"]));
//!
//! let text = b.build();
//! assert!(text.starts_with("MSH|^~\\&|RIS|HOSP|||||ORM^O01^ORM_O01|MSG0001|P|2.5.1\r"));
//! let msg = Message::parse(&text).unwrap();
//! assert_eq!(msg.get("PID-3.4"), Some("HOSP"));
//! assert_eq!(msg.get_decoded("PID-5.2").unwrap(), "Jane & John");   // `&` was escaped
//! assert_eq!(msg.get("ZDS-1.1"), Some("1.2.3"));
//! ```

use crate::Encoding;
use std::borrow::Cow;

/// Escape the delimiter characters of `enc` in `text` so it can be placed in
/// a field as a literal. Line breaks become `\.br\`.
///
/// This is the inverse of [`unescape`](crate::unescape) for the delimiter
/// sequences.
pub fn escape<'a>(text: &'a str, enc: Encoding) -> Cow<'a, str> {
    let special = [
        enc.field,
        enc.component,
        enc.repetition,
        enc.escape,
        enc.subcomponent,
        b'\r',
        b'\n',
    ];
    if !text.bytes().any(|b| special.contains(&b)) {
        return Cow::Borrowed(text);
    }
    let esc = enc.escape as char;
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let code = if c.is_ascii() { Some(c as u8) } else { None };
        match code {
            Some(b) if b == enc.escape => push_seq(&mut out, esc, "E"),
            Some(b) if b == enc.field => push_seq(&mut out, esc, "F"),
            Some(b) if b == enc.component => push_seq(&mut out, esc, "S"),
            Some(b) if b == enc.repetition => push_seq(&mut out, esc, "R"),
            Some(b) if b == enc.subcomponent => push_seq(&mut out, esc, "T"),
            Some(b'\r') => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                push_seq(&mut out, esc, ".br");
            }
            Some(b'\n') => push_seq(&mut out, esc, ".br"),
            _ => out.push(c),
        }
    }
    Cow::Owned(out)
}

fn push_seq(out: &mut String, esc: char, seq: &str) {
    out.push(esc);
    out.push_str(seq);
    out.push(esc);
}

/// The value of one field: repetitions of components of subcomponents.
///
/// Build one with [`Value::text`], [`Value::components`] or
/// [`Value::repetitions`], or from a `&str`/`String` (a single text).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Value {
    /// `repetitions[r][c][s]` is subcomponent `s+1` of component `c+1` of
    /// repetition `r+1`.
    pub repetitions: Vec<Vec<Vec<String>>>,
}

impl Value {
    /// A field holding one plain text.
    pub fn text(text: impl Into<String>) -> Value {
        Value {
            repetitions: vec![vec![vec![text.into()]]],
        }
    }

    /// One repetition with the given components, each a plain text.
    pub fn components<I, S>(components: I) -> Value
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Value {
            repetitions: vec![components.into_iter().map(|c| vec![c.into()]).collect()],
        }
    }

    /// Several repetitions, each given as its components.
    pub fn repetitions<I, R, S>(reps: I) -> Value
    where
        I: IntoIterator<Item = R>,
        R: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Value {
            repetitions: reps
                .into_iter()
                .map(|r| r.into_iter().map(|c| vec![c.into()]).collect())
                .collect(),
        }
    }

    /// Set subcomponent `s` of component `c` of repetition `r` (all
    /// one-based), growing the value as needed.
    pub fn set(&mut self, r: usize, c: usize, s: usize, text: impl Into<String>) -> &mut Value {
        let (r, c, s) = (r.max(1) - 1, c.max(1) - 1, s.max(1) - 1);
        if self.repetitions.len() <= r {
            self.repetitions.resize(r + 1, Vec::new());
        }
        let rep = &mut self.repetitions[r];
        if rep.len() <= c {
            rep.resize(c + 1, Vec::new());
        }
        let comp = &mut rep[c];
        if comp.len() <= s {
            comp.resize(s + 1, String::new());
        }
        comp[s] = text.into();
        self
    }

    fn render(&self, enc: Encoding) -> String {
        let sub = (enc.subcomponent as char).to_string();
        let comp = (enc.component as char).to_string();
        let rep = (enc.repetition as char).to_string();
        self.repetitions
            .iter()
            .map(|r| {
                r.iter()
                    .map(|c| {
                        c.iter()
                            .map(|s| escape(s, enc))
                            .collect::<Vec<_>>()
                            .join(&sub)
                    })
                    .collect::<Vec<_>>()
                    .join(&comp)
            })
            .collect::<Vec<_>>()
            .join(&rep)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::text(s)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::text(s)
    }
}

impl From<&String> for Value {
    fn from(s: &String) -> Value {
        Value::text(s.clone())
    }
}

/// One segment under construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentBuilder {
    name: String,
    /// `fields[n - 1]` is field `n`; `None` renders empty.
    fields: Vec<Option<Value>>,
}

impl SegmentBuilder {
    fn new(name: &str) -> SegmentBuilder {
        SegmentBuilder {
            name: name.to_ascii_uppercase(),
            fields: Vec::new(),
        }
    }

    /// Segment name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Set field `n` (one-based). For `MSH`, fields 1 and 2 are the
    /// delimiters and cannot be set; they are written from the encoding.
    pub fn set(&mut self, n: usize, value: impl Into<Value>) -> &mut SegmentBuilder {
        if n == 0 || (self.name == "MSH" && n <= 2) {
            return self;
        }
        if self.fields.len() < n {
            self.fields.resize(n, None);
        }
        self.fields[n - 1] = Some(value.into());
        self
    }

    /// Set component `c` of field `n` (first repetition), keeping the other
    /// components.
    pub fn set_component(
        &mut self,
        n: usize,
        c: usize,
        text: impl Into<String>,
    ) -> &mut SegmentBuilder {
        self.set_subcomponent(n, c, 1, text)
    }

    /// Set subcomponent `s` of component `c` of field `n` (first repetition).
    pub fn set_subcomponent(
        &mut self,
        n: usize,
        c: usize,
        s: usize,
        text: impl Into<String>,
    ) -> &mut SegmentBuilder {
        if n == 0 || (self.name == "MSH" && n <= 2) {
            return self;
        }
        if self.fields.len() < n {
            self.fields.resize(n, None);
        }
        self.fields[n - 1]
            .get_or_insert_with(Value::default)
            .set(1, c, s, text);
        self
    }

    /// Append a repetition to field `n`.
    pub fn add_repetition<I, S>(&mut self, n: usize, components: I) -> &mut SegmentBuilder
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if n == 0 || (self.name == "MSH" && n <= 2) {
            return self;
        }
        if self.fields.len() < n {
            self.fields.resize(n, None);
        }
        let value = self.fields[n - 1].get_or_insert_with(Value::default);
        if value.repetitions.len() == 1 && value.repetitions[0].is_empty() {
            value.repetitions.clear();
        }
        value
            .repetitions
            .push(components.into_iter().map(|c| vec![c.into()]).collect());
        self
    }

    fn render(&self, enc: Encoding) -> String {
        let fs = enc.field as char;
        let mut out = self.name.clone();
        let mut fields: Vec<String> = self
            .fields
            .iter()
            .map(|f| f.as_ref().map(|v| v.render(enc)).unwrap_or_default())
            .collect();
        if self.name == "MSH" {
            // Fields 1 and 2 are the delimiters themselves.
            let mut msh = vec![enc.msh2()];
            msh.extend(fields.drain(..).skip(2));
            fields = msh;
        }
        for f in fields {
            out.push(fs);
            out.push_str(&f);
        }
        out
    }
}

/// Builds a message segment by segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Builder {
    encoding: Encoding,
    segments: Vec<SegmentBuilder>,
}

impl Default for Builder {
    fn default() -> Self {
        Builder::new()
    }
}

impl Builder {
    /// A builder using the standard delimiters `|^~\&`.
    pub fn new() -> Builder {
        Builder::with_encoding(Encoding::STANDARD)
    }

    /// A builder using custom delimiters.
    pub fn with_encoding(encoding: Encoding) -> Builder {
        Builder {
            encoding,
            segments: Vec::new(),
        }
    }

    /// The encoding in use.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Append a segment and return it for filling in. The first segment
    /// should be `MSH`; [`Builder::build`] inserts one if it is missing.
    pub fn segment(&mut self, name: &str) -> &mut SegmentBuilder {
        self.segments.push(SegmentBuilder::new(name));
        let last = self.segments.len() - 1;
        &mut self.segments[last]
    }

    /// The segments so far.
    pub fn segments(&self) -> &[SegmentBuilder] {
        &self.segments
    }

    /// Mutable access to segment `index` (zero-based), e.g. to fill MSH later.
    pub fn segment_at(&mut self, index: usize) -> Option<&mut SegmentBuilder> {
        self.segments.get_mut(index)
    }

    /// Render the message with `\r` segment terminators, as the standard
    /// requires. Every segment, including the last, ends with `\r`.
    pub fn build(&self) -> String {
        let mut out = String::new();
        if self.segments.first().map(|s| s.name.as_str()) != Some("MSH") {
            out.push_str(&SegmentBuilder::new("MSH").render(self.encoding));
            out.push('\r');
        }
        for seg in &self.segments {
            out.push_str(&seg.render(self.encoding));
            out.push('\r');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{unescape, Message};

    #[test]
    fn escape_roundtrips_through_unescape() {
        let enc = Encoding::STANDARD;
        let text = "a|b^c~d\\e&f\r\ng\nh";
        let escaped = escape(text, enc);
        assert_eq!(escaped, "a\\F\\b\\S\\c\\R\\d\\E\\e\\T\\f\\.br\\g\\.br\\h");
        assert_eq!(unescape(&escaped, enc), "a|b^c~d\\e&f\ng\nh");
        assert!(matches!(escape("plain", enc), Cow::Borrowed(_)));
    }

    #[test]
    fn msh_is_written_from_the_encoding() {
        let mut b = Builder::new();
        b.segment("MSH")
            .set(1, "ignored")
            .set(2, "ignored")
            .set(3, "APP");
        assert_eq!(b.build(), "MSH|^~\\&|APP\r");
        let mut c = Builder::with_encoding(Encoding::from_msh2(b'#', b"!*?%").unwrap());
        c.segment("MSH").set(9, Value::components(["ADT", "A01"]));
        c.segment("PID").set(3, "A#B");
        assert_eq!(c.build(), "MSH#!*?%#######ADT!A01\rPID###A?F?B\r");
        let msg = Message::parse(c.build()).unwrap();
        assert_eq!(msg.get_decoded("PID-3").unwrap(), "A#B");
    }

    #[test]
    fn missing_msh_is_inserted() {
        let mut b = Builder::new();
        b.segment("PID").set(1, "1");
        assert_eq!(b.build(), "MSH|^~\\&\rPID|1\r");
    }

    #[test]
    fn structure_roundtrips_through_parser() {
        let mut b = Builder::new();
        b.segment("MSH")
            .set(9, Value::components(["ORU", "R01"]))
            .set(10, "1");
        let pid = b.segment("PID");
        pid.set(
            3,
            Value::repetitions([["A", "", "", "H1"], ["B", "", "", "H2"]]),
        );
        pid.set_subcomponent(3, 4, 2, "sub");
        pid.set_component(5, 2, "First");
        pid.add_repetition(3, ["C"]);
        b.segment("OBX").set(5, "value with | and ^ inside");
        let text = b.build();
        assert_eq!(
            text,
            "MSH|^~\\&|||||||ORU^R01|1\rPID|||A^^^H1&sub~B^^^H2~C||^First\rOBX|||||value with \\F\\ and \\S\\ inside\r"
        );
        let msg = Message::parse(&text).unwrap();
        assert_eq!(msg.get("PID-3[2].4"), Some("H2"));
        assert_eq!(msg.get("PID-3[3].1"), Some("C"));
        assert_eq!(msg.get("PID-3.4.2"), Some("sub"));
        assert_eq!(msg.get("PID-5.2"), Some("First"));
        assert_eq!(
            msg.get_decoded("OBX-5").unwrap(),
            "value with | and ^ inside"
        );
        assert!(msg.warnings().is_empty());
    }

    #[test]
    fn value_set_grows() {
        let mut v = Value::default();
        v.set(2, 3, 1, "x");
        assert_eq!(v.render(Encoding::STANDARD), "~^^x");
        assert_eq!(Value::from("t").render(Encoding::STANDARD), "t");
    }
}
