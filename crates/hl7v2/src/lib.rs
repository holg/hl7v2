//! A small, dependency-free HL7 v2.x parser.
//!
//! The parser splits a message into segments, fields, repetitions, components and
//! subcomponents and keeps the **byte span** of every node, so a caller can point
//! back into the original text (for highlighting, diffing or error reporting)
//! without searching for the value again.
//!
//! Design constraints:
//!
//! * No dependencies, no `unsafe`, no panics on malformed input. Malformed
//!   messages are the normal case in integration work; every failure is a value.
//! * Tolerant of what real interfaces produce: `\r`, `\r\n` and bare `\n`
//!   segment terminators, MLLP framing bytes, a UTF-8 BOM, a malformed MSH-2.
//! * MSH numbering is handled correctly: MSH-1 is the field separator and
//!   MSH-2 the encoding characters, so `MSH-9` is the message type in this
//!   crate exactly as it is in the standard.
//! * Encoding characters come from MSH-2, never hardcoded. Escape sequences
//!   (`\F\`, `\S\`, `\T\`, `\R\`, `\E\`, `\Xdd..\`, `\.br\`) decode on demand.
//! * Host-target only code: no `web-sys`, no I/O. It compiles for
//!   `wasm32-unknown-unknown` unchanged.
//!
//! ```
//! use hl7v2::Message;
//!
//! let text = "MSH|^~\\&|RIS|HOSP|PACS|HOSP|20260905120000||ORM^O01|42|P|2.5.1\r\
//!             PID|1||4MR1^^^HOSP^MR||Doe^Jane||19700101|F\r\
//!             OBR|1|A1001||MR1^MRI HEAD|||20260905|||||||||||A1001|RP-77\r\
//!             ZDS|1.3.6.1.4.1.5962.1.2.4.20040826185059.5457^^Application^DICOM\r";
//!
//! let msg = Message::parse(text).unwrap();
//! assert_eq!(msg.get("MSH-9.1"), Some("ORM"));
//! assert_eq!(msg.get("PID-3.1"), Some("4MR1"));
//! assert_eq!(msg.get("OBR-18"), Some("A1001"));
//! assert_eq!(msg.get("ZDS-1.1"), Some("1.3.6.1.4.1.5962.1.2.4.20040826185059.5457"));
//!
//! // Every value knows where it came from.
//! let span = msg.get_span("PID-3.1").unwrap();
//! assert_eq!(&text[span.range()], "4MR1");
//! ```
//!
//! The [`order`] module builds on the parser to extract the four identifiers an
//! imaging order carries (patient ID, accession number, requested procedure ID
//! and study instance UID) as defined by the IHE Radiology Technical Framework.
//!
//! The [`builder`] module writes messages: values are set structurally and
//! delimiter characters inside them are escaped, so what you build parses
//! back to what you set.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod builder;
mod encoding;
mod error;
mod escape;
mod message;
pub mod order;
mod path;

pub use encoding::Encoding;
pub use error::{ParseError, PathError};
pub use escape::unescape;
pub use message::{
    Component, Field, Message, MessageType, Repetition, Segment, Span, Subcomponent, Warning,
};
pub use path::Path;
