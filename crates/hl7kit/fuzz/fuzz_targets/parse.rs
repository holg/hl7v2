//! Any byte string must parse to a value or an error, and every accessor on
//! the result must return without panicking: the crate promises no panics on
//! malformed input.
//!
//!     cargo +nightly fuzz run parse
#![no_main]

use hl7kit::order::Order;
use hl7kit::Message;
use libfuzzer_sys::fuzz_target;

const PATHS: &[&str] = &[
    "MSH-9.1", "MSH-9", "MSH-1", "MSH-2", "MSH-18", "PID-3.1", "PID-3[2].1", "PID-5.1.1",
    "OBR-18", "OBR-19", "IPC-3.1", "IPC[2]-3.1", "ZDS-1.1", "OBX-5", "OBX[3]-5", "ZZZ-99.9.9",
];

fuzz_target!(|data: &[u8]| {
    for parsed in [Message::parse_bytes(data), Message::parse_lossy(data)] {
        let Ok(msg) = parsed else { continue };
        let _ = (msg.message_type(), msg.control_id(), msg.version());
        for seg in msg.segments() {
            let _ = (seg.name(), seg.text(), seg.span().slice(msg.raw()));
            for field in seg.fields() {
                let _ = field.decoded();
                for rep in field.repetitions() {
                    for comp in rep.components() {
                        let _ = comp.decoded();
                        for sub in comp.subcomponents() {
                            let _ = sub.decoded();
                        }
                    }
                }
            }
        }
        for path in PATHS {
            let _ = (msg.get(path), msg.get_decoded(path), msg.get_span(path));
        }
        let _ = Order::extract(&msg);
    }
});
