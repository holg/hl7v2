# hl7kit (Python)

`pip install hl7kit` — HL7 v2.x parser (Rust core, PyO3) with byte spans and imaging-order linkage.
No runtime dependencies. Wheels for Linux x86_64/aarch64, macOS, Windows; Python 3.9+ (abi3).

```python
import hl7kit
msg = hl7kit.parse(open("order.hl7", "rb").read())
o = msg.order()
o.accession_number, o.requested_procedure_id, o.scheduled_procedure_step_id
o.study_uid.value, o.study_uid.source   # "IPC-3.1" | "ZDS-1.1" | "OBX-110180"
o.warnings                              # e.g. UID conflict between ZDS and OBX
```

CLI: `hl7kit inspect *.hl7 [--json]` (exit 1 if any linkage warning).
