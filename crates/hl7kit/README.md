# hl7kit

[![crates.io](https://img.shields.io/crates/v/hl7kit.svg)](https://crates.io/crates/hl7kit)
[![docs.rs](https://docs.rs/hl7kit/badge.svg)](https://docs.rs/hl7kit)
[![CI](https://github.com/holg/hl7v2/actions/workflows/ci.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/hl7kit?label=pypi)](https://pypi.org/project/hl7kit/)
[![license](https://img.shields.io/crates/l/hl7kit.svg)](https://github.com/holg/hl7v2#license)

A small, dependency-free HL7 v2.x parser and builder for Rust that keeps the
**byte span of every field, repetition, component and subcomponent**, so you
can point back into the original message without searching for the value
again. Also available as a Python wheel (`pip install hl7kit`).

The crate is one third of the [hl7v2 workspace](https://github.com/holg/hl7v2):
[mwlkit](https://crates.io/crates/mwlkit) turns its orders into DICOM
Modality Worklist items, and [dicomscope](https://holg.github.io/hl7v2/) is
the viewer that shows the linkage from order to image to FHIR.

## Why another HL7 parser

Integration work is mostly reading messages that are slightly wrong. The
parser is built for that:

- **Spans everywhere.** `msg.get_span("PID-3.1")` gives you the byte range of
  the value in the raw text. Highlighting, diffing and error reporting need
  this; re-searching for a value that may appear twice is a bug waiting to
  happen.
- **Tolerant, but honest.** `\r`, `\r\n` and bare `\n` terminators, MLLP framing
  bytes, a UTF-8 BOM, and a malformed MSH-2 are all accepted, and each is
  reported through `msg.warnings()` so a reviewer can see what was tolerated.
- **MSH numbering done right.** MSH-1 is the field separator and MSH-2 the
  encoding characters, exactly as the standard numbers them. MSH-2 is never
  split on its own component separator.
- **Encoding characters from MSH-2.** Custom delimiters work. Escape sequences
  (`\F\`, `\S\`, `\T\`, `\R\`, `\E\`, `\Xdd..\`, `\.br\`) decode on demand and
  never silently; unknown sequences are kept verbatim.
- **No dependencies, no `unsafe`, no panics on input.** Every failure is a value
  with a message that names the line or segment. Compiles unchanged for
  `wasm32-unknown-unknown`.

## Usage

```rust
use hl7kit::Message;
use hl7kit::order::Order;

let msg = Message::parse(text)?;
assert_eq!(msg.get("MSH-9.1"), Some("ORM"));
assert_eq!(msg.get("PID-3.1"), Some("4MR1"));
assert_eq!(msg.get("PID-3[2].4"), Some("OTHER"));   // second repetition, fourth component
assert_eq!(msg.get("OBX[3]-5"), Some("three"));    // third OBX segment

// Byte span of any value, for highlighting in the raw text.
let span = msg.get_span("ZDS-1.1")?;
let uid = &msg.raw()[span.range()];

// Walk the tree.
for seg in msg.segments_named("OBX") {
    let value = seg.field(5)?.decoded();  // escapes decoded
}
```

Query path syntax: `SEG[occurrence]-field[repetition].component.subcomponent`,
all indices one-based. `PID.3.1` is accepted as an alias for `PID-3.1`.

## Imaging orders

`hl7kit::order` extracts the four identifiers that link an order to its
images (IHE Radiology RAD-4), from ORM^O01 with a ZDS segment, OMI^O23
with IPC segments, and ORU^R01 with a DCM 110180 OBX alike:

```rust
let order = Order::extract(&msg);
order.patient_id;         // PID-3.1
order.accession;          // OBR-18, else IPC-1.1
order.procedure_id;       // OBR-19, else IPC-2.1
order.study_uid;          // IPC-3.1, else ZDS-1.1, else the OBX
order.study_uid_source;   // Some(StudyUidSource::Ipc3 | Zds1 | Obx)
order.source_path(hl7kit::order::OrderField::Accession); // "OBR-18" or "IPC-1.1"
order.spans;              // where each present field sits in the text
order.warnings;           // ConflictingStudyUid when the sources disagree
```

`OrderField::paths()` lists the candidate paths per field in precedence
order. Study UIDs are compared across sources and a disagreement is a
warning, never a silent choice: the accession number is what a RIS works
by, and archives are known to derive or regenerate the UID downstream.

## Building messages

```rust
use hl7kit::builder::{Builder, Value};

let mut b = Builder::new();
b.segment("MSH").set(3, "RIS").set(9, Value::components(["ORM", "O01"])).set(10, "1");
b.segment("PID").set(3, Value::components(["4MR1", "", "", "HOSP", "MR"]));
b.segment("OBR").set(4, Value::components(["", "CT head | with contrast"]));  // `|` is escaped
b.segment("ZDS").set(1, Value::components(["1.2.3", "", "Application", "DICOM"]));
let text = b.build();   // CR-terminated; parses back to the same values
```

The builder is structural: delimiter characters inside values are escaped,
MSH-1 and MSH-2 are written from the encoding, and custom encodings are
supported.

## Status

0.2: parser, query API, order extraction with source and conflict
reporting, builder. Not covered: message-structure validation against
segment tables, and batch/file headers (FHS/BHS). No MLLP: transport is a
different component.

## License

MIT OR Apache-2.0.
