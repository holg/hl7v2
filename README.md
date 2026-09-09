# hl7kit

[![crates.io](https://img.shields.io/crates/v/hl7kit.svg)](https://crates.io/crates/hl7kit)
[![docs.rs](https://docs.rs/hl7kit/badge.svg)](https://docs.rs/hl7kit)
[![CI](https://github.com/holg/hl7v2/actions/workflows/ci.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/ci.yml)
[![demo](https://github.com/holg/hl7v2/actions/workflows/pages.yml/badge.svg)](https://holg.github.io/hl7v2/)
[![publish](https://github.com/holg/hl7v2/actions/workflows/publish.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/publish.yml)
[![license](https://img.shields.io/crates/l/hl7kit.svg)](#license)
![MSRV 1.85](https://img.shields.io/badge/MSRV-1.85-blue.svg)

A small, dependency-free HL7 v2.x parser for Rust that keeps the **byte span of
every field, repetition, component and subcomponent**, so you can point back
into the original message without searching for the value again.

The crate lives in [`crates/hl7kit`](crates/hl7kit) and on
[crates.io](https://crates.io/crates/hl7kit). The browser DICOM viewer in
[`demo/dicomscope`](demo/dicomscope) is its demo: it uses the spans to
highlight the four order identifiers in place, links them to a DICOM study,
and emits the pair as a FHIR R4 bundle (`Patient`, `ServiceRequest`,
`ImagingStudy` with the MII Bildgebung profile).

**Live demo:** <https://holg.github.io/hl7v2/>, built from `main` by
`.github/workflows/pages.yml`. It runs entirely in your browser: files you
open never leave the tab, and the page makes no network requests after it
loads. Needs WebGPU (Chrome, Edge, Safari 26; Firefox with it enabled).

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
| `crates/hl7kit` | The library. `cargo test -p hl7kit` |
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
supported. `dicomscope order` in the demo uses it to write an order that
matches a real DICOM study, then parses the result and checks the linkage,
which is how the sample messages in `samples/` are produced.

## Releasing

The library is published to crates.io by `.github/workflows/publish.yml`
when a `v*` tag is pushed whose version equals `crates/hl7kit/Cargo.toml`:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

The workflow tests, packages and uploads. Credentials come from crates.io
Trusted Publishing (configure repository `holg/hl7v2`, workflow
`publish.yml` in the crate's settings once it exists) or, for the first
release, a `CARGO_REGISTRY_TOKEN` repository secret. `workflow_dispatch` runs
the same steps without uploading. The demo crate is never published.

## Status

0.1. Parser, query API, order extraction and builder. Not covered yet:
message-structure validation against segment tables, and batch/file headers
(FHS/BHS).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state
otherwise, any contribution intentionally submitted for inclusion in the work
by you shall be dual licensed as above, without any additional terms or
conditions.
