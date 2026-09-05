# hl7kit

[![crates.io](https://img.shields.io/crates/v/hl7kit.svg)](https://crates.io/crates/hl7kit)
[![docs.rs](https://docs.rs/hl7kit/badge.svg)](https://docs.rs/hl7kit)
[![CI](https://github.com/holg/hl7v2/actions/workflows/ci.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/hl7kit.svg)](https://github.com/holg/hl7v2#license)

Dependency-free HL7 v2.x parser and builder with byte spans for every node. See the
[workspace README](../../README.md) for an overview and the crate
documentation (`cargo doc -p hl7kit --open`) for the API.

```rust
let msg = hl7kit::Message::parse(text)?;
let patient = msg.get("PID-3.1");
let span = msg.get_span("PID-3.1");   // where it is in `msg.raw()`
```
