# hl7kit

Dependency-free HL7 v2.x parser with byte spans for every node. See the
[workspace README](../../README.md) for an overview and the crate
documentation (`cargo doc -p hl7kit --open`) for the API.

```rust
let msg = hl7kit::Message::parse(text)?;
let patient = msg.get("PID-3.1");
let span = msg.get_span("PID-3.1");   // where it is in `msg.raw()`
```
