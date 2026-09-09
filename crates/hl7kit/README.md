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

// Imaging-order identifiers, ORM^O01 (ZDS) and OMI^O23 (IPC) alike.
let order = hl7kit::order::Order::extract(&msg);
order.study_uid;                       // IPC-3.1 first, ZDS-1.1 second
order.study_uid_source;                // Some(StudyUidSource::Ipc3 | Zds1)
order.source_path(hl7kit::order::OrderField::Accession); // "OBR-18" or "IPC-1.1"
order.warnings;                        // ConflictingStudyUid when IPC segments disagree
```

`OrderField::paths()` lists the candidate paths per field in precedence
order; `path()` is deprecated since 0.2.0.
