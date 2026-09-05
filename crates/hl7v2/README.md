# hl7v2

Dependency-free HL7 v2.x parser with byte spans for every node. See the
[workspace README](../../README.md) for an overview and the crate
documentation (`cargo doc -p hl7v2 --open`) for the API.

```rust
let msg = hl7v2::Message::parse(text)?;
let patient = msg.get("PID-3.1");
let span = msg.get_span("PID-3.1");   // where it is in `msg.raw()`
```
