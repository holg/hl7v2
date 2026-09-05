//! The parsed message and its node views.

use crate::escape::unescape;
use crate::{Encoding, ParseError, Path};
use std::borrow::Cow;
use std::ops::Range;

/// A byte range into [`Message::raw`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    /// Inclusive start offset.
    pub start: usize,
    /// Exclusive end offset.
    pub end: usize,
}

impl Span {
    /// Create a span.
    pub const fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }
    /// Length in bytes.
    pub const fn len(&self) -> usize {
        self.end - self.start
    }
    /// Whether the span is empty.
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }
    /// The span as a range, for slicing.
    pub const fn range(&self) -> Range<usize> {
        self.start..self.end
    }
    /// Slice `text` with this span. Returns an empty string when the span is
    /// out of bounds or not on a character boundary rather than panicking.
    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.range()).unwrap_or("")
    }
}

impl From<Span> for Range<usize> {
    fn from(s: Span) -> Range<usize> {
        s.range()
    }
}

/// Something the parser tolerated but a reviewer may want to know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// MSH-2 was malformed; the standard encoding characters were used instead.
    EncodingFallback {
        /// The MSH-2 text that was found.
        found: String,
    },
    /// MLLP framing bytes (`0x0B`, `0x1C`) were present and ignored.
    MllpFraming,
    /// A UTF-8 byte order mark preceded MSH and was ignored.
    ByteOrderMark,
    /// Segment terminators were not `\r` (the standard) but `\n` or `\r\n`.
    NonStandardTerminator,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::EncodingFallback { found } => write!(
                f,
                "MSH-2 {found:?} is not a valid set of encoding characters; using ^~\\&"
            ),
            Warning::MllpFraming => write!(f, "MLLP framing bytes present and ignored"),
            Warning::ByteOrderMark => write!(f, "UTF-8 byte order mark ignored"),
            Warning::NonStandardTerminator => {
                write!(f, "segments terminated by LF or CRLF instead of CR")
            }
        }
    }
}

#[derive(Debug, Clone)]
struct SegmentNode {
    name: Span,
    span: Span,
    /// Index 0 is the segment name; index `n` is field `n` (HL7 numbering).
    fields: Vec<FieldNode>,
}

#[derive(Debug, Clone)]
struct FieldNode {
    span: Span,
    repetitions: Vec<RepetitionNode>,
}

#[derive(Debug, Clone)]
struct RepetitionNode {
    span: Span,
    components: Vec<ComponentNode>,
}

#[derive(Debug, Clone)]
struct ComponentNode {
    span: Span,
    subcomponents: Vec<Span>,
}

/// A parsed HL7 v2 message.
///
/// The message owns a copy of the input text; all views borrow from it.
#[derive(Debug, Clone)]
pub struct Message {
    raw: String,
    encoding: Encoding,
    segments: Vec<SegmentNode>,
    warnings: Vec<Warning>,
}

/// MSH-9, the message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageType<'m> {
    /// MSH-9.1, e.g. `ORM`.
    pub code: &'m str,
    /// MSH-9.2, e.g. `O01`.
    pub trigger: &'m str,
    /// MSH-9.3, e.g. `ORM_O01`; empty when absent.
    pub structure: &'m str,
}

impl Message {
    /// Parse a message from text.
    pub fn parse(text: impl Into<String>) -> Result<Message, ParseError> {
        parse(text.into())
    }

    /// Parse a message from bytes that must be valid UTF-8.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Message, ParseError> {
        match std::str::from_utf8(bytes) {
            Ok(text) => parse(text.to_string()),
            Err(e) => Err(ParseError::InvalidUtf8 {
                valid_up_to: e.valid_up_to(),
            }),
        }
    }

    /// Parse a message from bytes, replacing invalid UTF-8 sequences. Spans
    /// refer to the converted text available through [`Message::raw`], not to
    /// the original bytes.
    pub fn parse_lossy(bytes: &[u8]) -> Result<Message, ParseError> {
        parse(String::from_utf8_lossy(bytes).into_owned())
    }

    /// The message text that all spans index into.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// The encoding characters in effect.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Tolerated irregularities in the input.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Number of segments.
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// All segments in order.
    pub fn segments(&self) -> impl Iterator<Item = Segment<'_>> + '_ {
        self.segments
            .iter()
            .enumerate()
            .map(move |(index, node)| Segment {
                msg: self,
                node,
                index,
            })
    }

    /// The segment at position `index` (zero-based).
    pub fn segment_at(&self, index: usize) -> Option<Segment<'_>> {
        self.segments.get(index).map(|node| Segment {
            msg: self,
            node,
            index,
        })
    }

    /// The first segment with the given name (case-insensitive).
    pub fn segment(&self, name: &str) -> Option<Segment<'_>> {
        self.segments_named(name).next()
    }

    /// All segments with the given name (case-insensitive).
    pub fn segments_named(&self, name: &str) -> impl Iterator<Item = Segment<'_>> + '_ {
        let name = name.to_ascii_uppercase();
        self.segments()
            .filter(move |s| s.name().eq_ignore_ascii_case(&name))
    }

    /// Look up a value by path, e.g. `PID-3.1`. See [`Path`] for the syntax.
    ///
    /// Returns `None` when the path does not parse or the addressed element is
    /// absent. Use [`Path::parse`] first to distinguish the two.
    pub fn get(&self, path: &str) -> Option<&str> {
        self.get_span(path).map(|s| s.slice(&self.raw))
    }

    /// Like [`Message::get`] but decodes escape sequences.
    pub fn get_decoded(&self, path: &str) -> Option<Cow<'_, str>> {
        self.get(path).map(|v| unescape(v, self.encoding))
    }

    /// The span of the element addressed by `path`.
    pub fn get_span(&self, path: &str) -> Option<Span> {
        Path::parse(path).ok().and_then(|p| self.resolve(&p))
    }

    /// Resolve a parsed path to a span.
    pub fn resolve(&self, path: &Path) -> Option<Span> {
        let seg = self
            .segments_named(&path.segment)
            .nth(path.occurrence.checked_sub(1)?)?;
        let Some(field_no) = path.field else {
            return Some(seg.span());
        };
        let field = seg.field(field_no)?;
        let rep_no = path.repetition;
        let Some(comp_no) = path.component else {
            return match rep_no {
                None => Some(field.span()),
                Some(r) => field.repetition(r).map(|r| r.span()),
            };
        };
        let rep = field.repetition(rep_no.unwrap_or(1))?;
        let comp = rep.component(comp_no)?;
        match path.subcomponent {
            None => Some(comp.span()),
            Some(s) => comp.subcomponent(s).map(|s| s.span()),
        }
    }

    /// Decode escape sequences in `text` with this message's encoding.
    pub fn decode<'a>(&self, text: &'a str) -> Cow<'a, str> {
        unescape(text, self.encoding)
    }

    /// MSH-9, the message type.
    pub fn message_type(&self) -> Option<MessageType<'_>> {
        let field = self.segment("MSH")?.field(9)?;
        Some(MessageType {
            code: field.component(1).map(|c| c.value()).unwrap_or(""),
            trigger: field.component(2).map(|c| c.value()).unwrap_or(""),
            structure: field.component(3).map(|c| c.value()).unwrap_or(""),
        })
    }

    /// MSH-10, the message control ID.
    pub fn control_id(&self) -> Option<&str> {
        self.get("MSH-10").filter(|v| !v.is_empty())
    }

    /// MSH-12.1, the HL7 version.
    pub fn version(&self) -> Option<&str> {
        self.get("MSH-12.1").filter(|v| !v.is_empty())
    }
}

/// A segment view.
#[derive(Clone, Copy)]
pub struct Segment<'m> {
    msg: &'m Message,
    node: &'m SegmentNode,
    index: usize,
}

impl<'m> Segment<'m> {
    /// Segment name, e.g. `PID`.
    pub fn name(&self) -> &'m str {
        self.node.name.slice(&self.msg.raw)
    }
    /// Zero-based position within the message.
    pub fn index(&self) -> usize {
        self.index
    }
    /// Span of the whole segment line, without its terminator.
    pub fn span(&self) -> Span {
        self.node.span
    }
    /// The segment text.
    pub fn text(&self) -> &'m str {
        self.node.span.slice(&self.msg.raw)
    }
    /// Highest field number present.
    pub fn field_count(&self) -> usize {
        self.node.fields.len().saturating_sub(1)
    }
    /// Field `n` in HL7 numbering (one-based). For `MSH`, field 1 is the field
    /// separator and field 2 the encoding characters, as the standard says.
    pub fn field(&self, n: usize) -> Option<Field<'m>> {
        if n == 0 {
            return None;
        }
        self.node.fields.get(n).map(|node| Field {
            msg: self.msg,
            node,
            number: n,
        })
    }
    /// All fields, numbered from 1.
    pub fn fields(&self) -> impl Iterator<Item = Field<'m>> + 'm {
        let msg = self.msg;
        self.node
            .fields
            .iter()
            .enumerate()
            .skip(1)
            .map(move |(number, node)| Field { msg, node, number })
    }
}

impl std::fmt::Debug for Segment<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Segment")
            .field("name", &self.name())
            .field("index", &self.index)
            .field("span", &self.node.span)
            .finish()
    }
}

/// A field view.
#[derive(Clone, Copy)]
pub struct Field<'m> {
    msg: &'m Message,
    node: &'m FieldNode,
    number: usize,
}

impl<'m> Field<'m> {
    /// One-based field number.
    pub fn number(&self) -> usize {
        self.number
    }
    /// Span of the whole field including all repetitions.
    pub fn span(&self) -> Span {
        self.node.span
    }
    /// Raw field text including separators of nested levels.
    pub fn value(&self) -> &'m str {
        self.node.span.slice(&self.msg.raw)
    }
    /// Field text with escape sequences decoded.
    pub fn decoded(&self) -> Cow<'m, str> {
        unescape(self.value(), self.msg.encoding)
    }
    /// True when the field has no content.
    pub fn is_empty(&self) -> bool {
        self.node.span.is_empty()
    }
    /// True when the field is the HL7 null value `""`.
    pub fn is_null(&self) -> bool {
        self.value() == "\"\""
    }
    /// Number of repetitions (at least 1).
    pub fn repetition_count(&self) -> usize {
        self.node.repetitions.len()
    }
    /// Repetition `n` (one-based).
    pub fn repetition(&self, n: usize) -> Option<Repetition<'m>> {
        let node = self.node.repetitions.get(n.checked_sub(1)?)?;
        Some(Repetition {
            msg: self.msg,
            node,
            index: n,
        })
    }
    /// All repetitions.
    pub fn repetitions(&self) -> impl Iterator<Item = Repetition<'m>> + 'm {
        let msg = self.msg;
        self.node
            .repetitions
            .iter()
            .enumerate()
            .map(move |(i, node)| Repetition {
                msg,
                node,
                index: i + 1,
            })
    }
    /// Component `n` of the first repetition.
    pub fn component(&self, n: usize) -> Option<Component<'m>> {
        self.repetition(1)?.component(n)
    }
}

impl std::fmt::Debug for Field<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Field")
            .field("number", &self.number)
            .field("value", &self.value())
            .field("span", &self.node.span)
            .finish()
    }
}

/// A repetition view.
#[derive(Clone, Copy)]
pub struct Repetition<'m> {
    msg: &'m Message,
    node: &'m RepetitionNode,
    index: usize,
}

impl<'m> Repetition<'m> {
    /// One-based repetition index.
    pub fn index(&self) -> usize {
        self.index
    }
    /// Span of this repetition.
    pub fn span(&self) -> Span {
        self.node.span
    }
    /// Raw text of this repetition.
    pub fn value(&self) -> &'m str {
        self.node.span.slice(&self.msg.raw)
    }
    /// Text with escape sequences decoded.
    pub fn decoded(&self) -> Cow<'m, str> {
        unescape(self.value(), self.msg.encoding)
    }
    /// Number of components (at least 1).
    pub fn component_count(&self) -> usize {
        self.node.components.len()
    }
    /// Component `n` (one-based).
    pub fn component(&self, n: usize) -> Option<Component<'m>> {
        let node = self.node.components.get(n.checked_sub(1)?)?;
        Some(Component {
            msg: self.msg,
            node,
            index: n,
        })
    }
    /// All components.
    pub fn components(&self) -> impl Iterator<Item = Component<'m>> + 'm {
        let msg = self.msg;
        self.node
            .components
            .iter()
            .enumerate()
            .map(move |(i, node)| Component {
                msg,
                node,
                index: i + 1,
            })
    }
}

impl std::fmt::Debug for Repetition<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repetition")
            .field("index", &self.index)
            .field("value", &self.value())
            .finish()
    }
}

/// A component view.
#[derive(Clone, Copy)]
pub struct Component<'m> {
    msg: &'m Message,
    node: &'m ComponentNode,
    index: usize,
}

impl<'m> Component<'m> {
    /// One-based component index.
    pub fn index(&self) -> usize {
        self.index
    }
    /// Span of this component.
    pub fn span(&self) -> Span {
        self.node.span
    }
    /// Raw text of this component.
    pub fn value(&self) -> &'m str {
        self.node.span.slice(&self.msg.raw)
    }
    /// Text with escape sequences decoded.
    pub fn decoded(&self) -> Cow<'m, str> {
        unescape(self.value(), self.msg.encoding)
    }
    /// Number of subcomponents (at least 1).
    pub fn subcomponent_count(&self) -> usize {
        self.node.subcomponents.len()
    }
    /// Subcomponent `n` (one-based).
    pub fn subcomponent(&self, n: usize) -> Option<Subcomponent<'m>> {
        let span = *self.node.subcomponents.get(n.checked_sub(1)?)?;
        Some(Subcomponent {
            msg: self.msg,
            span,
            index: n,
        })
    }
    /// All subcomponents.
    pub fn subcomponents(&self) -> impl Iterator<Item = Subcomponent<'m>> + 'm {
        let msg = self.msg;
        self.node
            .subcomponents
            .iter()
            .enumerate()
            .map(move |(i, &span)| Subcomponent {
                msg,
                span,
                index: i + 1,
            })
    }
}

impl std::fmt::Debug for Component<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Component")
            .field("index", &self.index)
            .field("value", &self.value())
            .finish()
    }
}

/// A subcomponent view.
#[derive(Clone, Copy)]
pub struct Subcomponent<'m> {
    msg: &'m Message,
    span: Span,
    index: usize,
}

impl<'m> Subcomponent<'m> {
    /// One-based subcomponent index.
    pub fn index(&self) -> usize {
        self.index
    }
    /// Span of this subcomponent.
    pub fn span(&self) -> Span {
        self.span
    }
    /// Raw text.
    pub fn value(&self) -> &'m str {
        self.span.slice(&self.msg.raw)
    }
    /// Text with escape sequences decoded.
    pub fn decoded(&self) -> Cow<'m, str> {
        unescape(self.value(), self.msg.encoding)
    }
}

impl std::fmt::Debug for Subcomponent<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subcomponent")
            .field("index", &self.index)
            .field("value", &self.value())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

const MLLP_START: u8 = 0x0B;
const MLLP_END: u8 = 0x1C;
const BOM: &[u8] = b"\xEF\xBB\xBF";

fn parse(raw: String) -> Result<Message, ParseError> {
    let bytes = raw.as_bytes();
    let mut warnings = Vec::new();

    let mut lines = split_lines(bytes, &mut warnings);
    // Drop empty and whitespace-only lines, strip MLLP framing.
    lines.retain_mut(|line| {
        let mut s = line.start;
        let mut e = line.end;
        loop {
            if s < e && (bytes[s] == MLLP_START || bytes[s].is_ascii_whitespace()) {
                if bytes[s] == MLLP_START {
                    push_unique(&mut warnings, Warning::MllpFraming);
                }
                s += 1;
            } else if bytes[s..e].starts_with(BOM) {
                push_unique(&mut warnings, Warning::ByteOrderMark);
                s += BOM.len();
            } else {
                break;
            }
        }
        while e > s && (bytes[e - 1] == MLLP_END || bytes[e - 1].is_ascii_whitespace()) {
            if bytes[e - 1] == MLLP_END {
                push_unique(&mut warnings, Warning::MllpFraming);
            }
            e -= 1;
        }
        *line = Span::new(s, e);
        s < e
    });

    let Some(first) = lines.first().copied() else {
        return Err(ParseError::Empty);
    };
    let first_text = first.slice(&raw);
    if !first_text.starts_with("MSH") {
        return Err(ParseError::MissingMsh {
            found: truncate(first_text, 16),
        });
    }
    if first.len() < 4 {
        return Err(ParseError::MalformedMsh {
            reason: "no field separator after MSH",
        });
    }
    let fs = bytes[first.start + 3];
    if !fs.is_ascii() || fs.is_ascii_alphanumeric() || fs.is_ascii_whitespace() {
        return Err(ParseError::MalformedMsh {
            reason: "byte after MSH is not a usable field separator",
        });
    }
    let enc_start = first.start + 4;
    let enc_end = bytes[enc_start..first.end]
        .iter()
        .position(|&b| b == fs)
        .map(|p| enc_start + p)
        .unwrap_or(first.end);
    let enc_bytes = &bytes[enc_start..enc_end];
    let encoding = match Encoding::from_msh2(fs, enc_bytes) {
        Some(e) => e,
        None => {
            warnings.push(Warning::EncodingFallback {
                found: String::from_utf8_lossy(enc_bytes).into_owned(),
            });
            Encoding::with_field(fs)
        }
    };

    let mut segments = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let name_len = bytes[line.start..line.end]
            .iter()
            .position(|&b| b == fs)
            .unwrap_or(line.len());
        let name = Span::new(line.start, line.start + name_len);
        let name_bytes = &bytes[name.range()];
        if name_bytes.len() != 3 || !name_bytes.iter().all(u8::is_ascii_alphanumeric) {
            return Err(ParseError::MalformedSegment {
                line: i + 1,
                text: truncate(line.slice(&raw), 40),
            });
        }
        let mut fields = vec![opaque_field(name)];
        if i == 0 {
            // MSH: field 1 is the separator, field 2 the encoding characters,
            // and neither is split on any delimiter.
            fields.push(opaque_field(Span::new(line.start + 3, line.start + 4)));
            fields.push(opaque_field(Span::new(enc_start, enc_end)));
            if enc_end < line.end {
                for span in split_on(bytes, Span::new(enc_end + 1, line.end), fs) {
                    fields.push(parse_field(bytes, span, encoding));
                }
            }
        } else if name.end < line.end {
            for span in split_on(bytes, Span::new(name.end + 1, line.end), fs) {
                fields.push(parse_field(bytes, span, encoding));
            }
        }
        segments.push(SegmentNode {
            name,
            span: *line,
            fields,
        });
    }

    Ok(Message {
        raw,
        encoding,
        segments,
        warnings,
    })
}

fn split_lines(bytes: &[u8], warnings: &mut Vec<Warning>) -> Vec<Span> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                lines.push(Span::new(start, i));
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                    push_unique(warnings, Warning::NonStandardTerminator);
                }
                start = i + 1;
            }
            b'\n' => {
                lines.push(Span::new(start, i));
                push_unique(warnings, Warning::NonStandardTerminator);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        lines.push(Span::new(start, bytes.len()));
    }
    lines
}

fn split_on(bytes: &[u8], span: Span, sep: u8) -> Vec<Span> {
    let mut out = Vec::new();
    let mut start = span.start;
    for (i, &b) in bytes[span.range()].iter().enumerate() {
        if b == sep {
            out.push(Span::new(start, span.start + i));
            start = span.start + i + 1;
        }
    }
    out.push(Span::new(start, span.end));
    out
}

fn opaque_field(span: Span) -> FieldNode {
    FieldNode {
        span,
        repetitions: vec![RepetitionNode {
            span,
            components: vec![ComponentNode {
                span,
                subcomponents: vec![span],
            }],
        }],
    }
}

fn parse_field(bytes: &[u8], span: Span, enc: Encoding) -> FieldNode {
    let repetitions = split_on(bytes, span, enc.repetition)
        .into_iter()
        .map(|rep| RepetitionNode {
            span: rep,
            components: split_on(bytes, rep, enc.component)
                .into_iter()
                .map(|comp| ComponentNode {
                    span: comp,
                    subcomponents: split_on(bytes, comp, enc.subcomponent),
                })
                .collect(),
        })
        .collect();
    FieldNode { span, repetitions }
}

fn push_unique(warnings: &mut Vec<Warning>, w: Warning) {
    if !warnings.contains(&w) {
        warnings.push(w);
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}
