//! Error types.

use std::fmt;

/// Why a message could not be parsed.
///
/// The parser is tolerant by design; these are the cases where there is no
/// sensible way to continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The input was empty or whitespace only.
    Empty,
    /// The input is not valid UTF-8. `valid_up_to` is the byte offset of the
    /// first invalid sequence. Use [`crate::Message::parse_lossy`] for
    /// Latin-1 or otherwise mis-declared input.
    InvalidUtf8 {
        /// Offset of the first invalid byte.
        valid_up_to: usize,
    },
    /// The first segment is not `MSH`.
    MissingMsh {
        /// What the first segment was instead (truncated).
        found: String,
    },
    /// The MSH segment is too short to declare the encoding characters.
    MalformedMsh {
        /// Human readable reason.
        reason: &'static str,
    },
    /// A segment line does not start with a three-character segment name.
    MalformedSegment {
        /// One-based line number within the message.
        line: usize,
        /// The offending text (truncated).
        text: String,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "message is empty"),
            ParseError::InvalidUtf8 { valid_up_to } => {
                write!(
                    f,
                    "message is not valid UTF-8 (first bad byte at offset {valid_up_to})"
                )
            }
            ParseError::MissingMsh { found } => {
                write!(f, "message must start with MSH, found {found:?}")
            }
            ParseError::MalformedMsh { reason } => write!(f, "malformed MSH segment: {reason}"),
            ParseError::MalformedSegment { line, text } => {
                write!(f, "line {line}: not a segment: {text:?}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Why a query path such as `PID-3.1` could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// The path was empty.
    Empty,
    /// The segment name is not three alphanumeric characters.
    BadSegment(String),
    /// A numeric index was missing, zero, or not a number.
    BadIndex(String),
    /// Unexpected trailing characters.
    Trailing(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Empty => write!(f, "path is empty"),
            PathError::BadSegment(s) => write!(f, "invalid segment name {s:?}"),
            PathError::BadIndex(s) => write!(f, "invalid index {s:?} (indices are 1-based)"),
            PathError::Trailing(s) => write!(f, "unexpected trailing text {s:?}"),
        }
    }
}

impl std::error::Error for PathError {}
