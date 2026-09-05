# hl7v2

A small, dependency-free HL7 v2.x parser for Rust that keeps the **byte span of
every field, repetition, component and subcomponent**, so you can point back
into the original message without searching for the value again.

The crate lives in [`crates/hl7v2`](crates/hl7v2). The browser DICOM viewer in
[`demo/dicomscope`](demo/dicomscope) is its demo: it uses the spans to
highlight the four order identifiers in place and links them to a DICOM study.

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
use hl7v2::Message;
use hl7v2::order::Order;

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

// The four IHE imaging-order identifiers, with spans.
let order = Order::extract(&msg);
order.study_uid;   // ZDS-1.1, first component only
order.accession;   // OBR-18
```

Query path syntax: `SEG[occurrence]-field[repetition].component.subcomponent`,
all indices one-based. `PID.3.1` is accepted as an alias for `PID-3.1`.

## Workspace

| Path | What |
| --- | --- |
| `crates/hl7v2` | The library. `cargo test -p hl7v2` |
| `demo/dicomscope` | Browser demo: Leptos + wgpu + dicom-rs, no JavaScript. See its [README](demo/dicomscope/README.md) |
| `samples/` | Fixture HL7 messages used by the tests and the demo |
| `docs/hl7v2.md` | The design document the demo was built from |

```sh
cargo test --workspace                                     # host tests for library and demo domain modules
cargo clippy -p dicomscope --target wasm32-unknown-unknown # the browser build
cd demo/dicomscope && trunk serve                          # run the demo locally
```

Toolchain: Rust 1.96, `wasm32-unknown-unknown` target, Trunk 0.21. Minimum
supported Rust version for the library is 1.85.

## Building messages

```rust
use hl7v2::builder::{Builder, Value};

let mut b = Builder::new();
b.segment("MSH").set(3, "RIS").set(9, Value::components(["ORM", "O01"])).set(10, "1");
b.segment("PID").set(3, Value::components(["4MR1", "", "", "HOSP", "MR"]));
b.segment("OBR").set(4, Value::components(["", "CT head | with contrast"]));  // `|` is escaped
b.segment("ZDS").set(1, Value::components(["1.2.3", "", "Application", "DICOM"]));
let text = b.build();   // CR-terminated; parses back to the same values
```

The builder is structural: delimiter characters inside values are escaped,
MSH-1 and MSH-2 are written from the encoding, and custom encodings are
supported. `dicomscope order` in the demo uses it to write an order that
matches a real DICOM study, then parses the result and checks the linkage,
which is how the sample messages in `samples/` are produced.

## Status

0.1. Parser, query API, order extraction and builder. Not covered yet:
message-structure validation against segment tables, and batch/file headers
(FHS/BHS).
