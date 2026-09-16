# hl7kit · mwlkit · dicomscope

[![CI](https://github.com/holg/hl7v2/actions/workflows/ci.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/ci.yml)
[![demo](https://github.com/holg/hl7v2/actions/workflows/pages.yml/badge.svg)](https://holg.github.io/hl7v2/)
[![fhir validation](https://github.com/holg/hl7v2/actions/workflows/fhir.yml/badge.svg)](https://holg.github.io/hl7v2/fhir/)
[![publish](https://github.com/holg/hl7v2/actions/workflows/publish.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/publish.yml)
[![python wheels](https://github.com/holg/hl7v2/actions/workflows/python.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/python.yml)
[![crates.io hl7kit](https://img.shields.io/crates/v/hl7kit.svg?label=crates.io%20hl7kit)](https://crates.io/crates/hl7kit)
[![crates.io mwlkit](https://img.shields.io/crates/v/mwlkit.svg?label=crates.io%20mwlkit)](https://crates.io/crates/mwlkit)
[![docs.rs hl7kit](https://docs.rs/hl7kit/badge.svg)](https://docs.rs/hl7kit)
[![docs.rs mwlkit](https://docs.rs/mwlkit/badge.svg)](https://docs.rs/mwlkit)
[![PyPI hl7kit](https://img.shields.io/pypi/v/hl7kit?label=pypi%20hl7kit)](https://pypi.org/project/hl7kit/)
[![PyPI mwlkit](https://img.shields.io/pypi/v/mwlkit?label=pypi%20mwlkit)](https://pypi.org/project/mwlkit/)
[![Python versions](https://img.shields.io/pypi/pyversions/hl7kit)](https://pypi.org/project/hl7kit/)
[![downloads](https://img.shields.io/pypi/dm/hl7kit)](https://pypistats.org/packages/hl7kit)
[![license](https://img.shields.io/crates/l/hl7kit.svg)](#license)
![MSRV 1.85](https://img.shields.io/badge/MSRV-1.85-blue.svg)

Rust for the seam between the radiology information system and the
archive: the HL7 v2 order that schedules an examination, the DICOM
worklist entry the modality reads, the images that come back, and the
FHIR resources that describe the whole thing afterwards. Three pieces,
each usable on its own:

| | What | Where |
|---|---|---|
| **hl7kit** | Dependency-free HL7 v2.x parser and builder with the byte span of every field, repetition, component and subcomponent, plus extraction of the four IHE imaging-order identifiers with their source and conflicts | [crates.io](https://crates.io/crates/hl7kit) · [PyPI](https://pypi.org/project/hl7kit/) · [`crates/hl7kit`](crates/hl7kit) |
| **mwlkit** | An hl7kit order turned into a DICOM Modality Worklist item per IHE RAD-4, every mapping choice sourced, every invented value reported, written as a Part 10 file | [crates.io](https://crates.io/crates/mwlkit) · [PyPI](https://pypi.org/project/mwlkit/) · [`crates/mwlkit`](crates/mwlkit) |
| **dicomscope** | A DICOM viewer that loads a study next to its order, shows the linkage order → worklist → image → FHIR, and runs in the browser, on the desktop and on an iPad from one code base | [live demo](https://holg.github.io/hl7v2/) · [`demo/`](demo) |

Everything is Rust, `#![forbid(unsafe_code)]` except the Objective-C border
on iOS, with no panics on input data and no network I/O anywhere. The
libraries compile unchanged for `wasm32-unknown-unknown`.

## hl7kit

A parser built for integration work, where most messages are slightly
wrong: `\r`, `\r\n` and bare `\n` terminators, MLLP framing, a BOM and a
malformed MSH-2 are all accepted and each is reported through
`msg.warnings()`. MSH-1 and MSH-2 are numbered as the standard numbers
them. Escape sequences decode on demand and never silently.

```rust
use hl7kit::{Message, order::Order};

let msg = Message::parse(text)?;
assert_eq!(msg.get("PID-3.1"), Some("4MR1"));
assert_eq!(msg.get("OBX[3]-5"), Some("three"));       // third OBX, field 5
let span = msg.get_span("ZDS-1.1")?;                  // byte range in msg.raw()

let order = Order::extract(&msg);
order.study_uid;          // IPC-3.1, else ZDS-1.1, else OBX DCM 110180
order.study_uid_source;   // which one it was
order.warnings;           // ConflictingStudyUid when they disagree
```

Query paths are `SEG[occurrence]-field[repetition].component.subcomponent`,
one-based. The builder writes messages back with correct escaping. Full
description in the [crate README](crates/hl7kit/README.md) and on
[docs.rs](https://docs.rs/hl7kit).

## mwlkit

The modality copies Patient ID, Accession Number, Study Instance UID,
Requested Procedure ID and SPS ID from the worklist entry into every image,
so the entry is the hinge of RIS/PACS linkage. mwlkit builds it from an
order and refuses to guess: a Type 1 attribute the order lacks is an error,
the only generated value is the Study Instance UID under an explicit policy
(dcm4che-style name-based, caller-supplied random bits, or refuse), and
values over the VR length are refused unless truncation is asked for,
never for AE titles or UIDs.

```rust
use hl7kit::{Message, order::Order};
use mwlkit::{worklist_item, to_bytes, Input, Options};

let msg = Message::parse(text)?;
let order = Order::extract(&msg);
let item = worklist_item(&Input { message: &msg, order: &order }, &Options::default())?;
for w in &item.warnings { eprintln!("{w}"); }   // what was defaulted, translated, generated
std::fs::write("order.mwl.dcm", to_bytes(&item)?)?;
```

The mapping table with its IHE and dcm4che sources, the UID and length
policies and the non-goals are in the [crate README](crates/mwlkit/README.md).

## dicomscope

One core, three shells. `demo/dicomscope-core` holds everything that is
not a user interface: DICOM loading from folders and zips, series scanning
and slice ordering, pixel decoding for the common transfer syntaxes,
windowing in a WGSL shader, measurements, the order linkage, the worklist
item, the FHIR R4 output, and the annotation tools. It is tested on the
host, and the shells only draw it.

- **Browser** ([live demo](https://holg.github.io/hl7v2/), `demo/dicomscope`):
  Leptos and WebGPU, no JavaScript written by hand, nothing leaves the tab.
  Drop a study and an order; see the series, the HL7 message with the four
  identifiers highlighted from their spans, the linkage verdict (including
  the headline case: linked, but the patient identifiers disagree), the
  worklist item with its notes and the chain order → worklist → image, and
  the FHIR bundle, which CI validates with the HL7 validator against the
  MII Bildgebung profile on every commit.
- **Desktop** (`demo/dicomscope-desktop`): winit, wgpu on Vulkan, Metal,
  DX12 or OpenGL, egui panels. No WebGPU and no webview, so it runs on
  Linux LTS machines. Same panels, plus native file dialogs, drag and drop,
  cine, SR and PDF documents, and a speed-first build profile.
- **iPad** (`demo/dicomscope-desktop/ios`): the same crate as a static
  library in a signed app, Metal underneath, touch gestures (two fingers
  scroll slices and set the level, pinch zooms), the document picker and
  "Open in dicomscope". Verified on an iPad Pro 10.5" on iOS 17.
- **For dentists**: a nerve tool that traces the mandibular canal as a
  yellow tube in millimetres, a local-contrast enhancement in the shader
  that brings the canal's cortical lines out, and a first semi-automatic
  canal finder. All of it is an annotation aid the dentist edits, not a
  detection device.

```sh
cd demo/dicomscope && trunk serve                                             # browser, http://localhost:8080
cargo run --profile native -p dicomscope-desktop -- study.zip order.hl7        # desktop
cargo run -q -p dicomscope -- fhir study.zip order.hl7 -o bundle.json          # host CLI: check, order, pack, fhir
```

Details, supported transfer syntaxes, the no-network guarantee and the
iPad build recipe are in the [demo README](demo/dicomscope/README.md).

## Python

Both libraries ship as wheels with no runtime dependencies, for Python 3.9
and newer on Linux (glibc and musl), macOS and Windows:

```sh
pip install hl7kit            # parser, spans, order linkage; `hl7kit inspect order.hl7`
pip install "mwlkit[pydicom]" # HL7 order -> Modality Worklist item, bytes for pydicom
```

```python
import hl7kit, mwlkit
msg = hl7kit.parse(open("order.hl7", "rb").read())
o = msg.order()
print(o.accession_number, o.study_uid.value, o.study_uid.source, o.warnings)
item = mwlkit.worklist_item(msg, default_station_ae="OPG1_AE")
ds = mwlkit.to_pydicom(item)    # or item.to_bytes() for a file
```

The binding crates are `crates/hl7kit-py` and `crates/mwlkit-py`; the
wheels are built and published by `.github/workflows/python.yml`.

## Repository layout

| Path | What |
| --- | --- |
| `crates/hl7kit` | The parser library. `cargo test -p hl7kit` |
| `crates/mwlkit` | Order to worklist item, on top of hl7kit and dicom-rs |
| `crates/hl7kit-py`, `crates/mwlkit-py` | PyO3 bindings; standalone packages built by maturin, not workspace members |
| `demo/dicomscope-core` | The viewer's domain code and renderer, host-tested |
| `demo/dicomscope` | Browser app and host CLI |
| `demo/dicomscope-desktop` | Native app, with the iPad project under `ios/` |
| `samples/` | HL7 fixtures used by the tests and the demo; DICOM samples are fetched, not committed |

```sh
cargo test --workspace                                       # libraries and the viewer's domain code
cargo clippy --workspace --all-targets -- -D warnings        # what CI runs
cargo clippy -p dicomscope --target wasm32-unknown-unknown   # the browser build
```

Toolchain: Rust 1.96 (MSRV 1.85 for the libraries), `wasm32-unknown-unknown`,
Trunk 0.21; Xcode for the iPad.

## Releasing

Every artefact leaves from its own tag, all through Trusted Publishing:
`hl7kit-vX.Y.Z` and `mwlkit-vX.Y.Z` to crates.io (`publish.yml`),
`hl7kit-py-vX.Y.Z` and `mwlkit-py-vX.Y.Z` to PyPI with attestations and a
GitHub release (`python.yml`). The browser demo deploys from every push to
`main`. See [RELEASING.md](RELEASING.md).

## Status

hl7kit 0.2: parser, query API, order extraction with source and conflict
reporting, builder. mwlkit 0.1: the RAD-4 mapping, UID and length
policies, Part 10 output. dicomscope: browser and desktop feature-complete
for viewing and linkage; the iPad build is functional and still
desktop-shaped in places. Not covered: HL7 message-structure validation
against segment tables, batch headers, MLLP and C-FIND (by design, see the
crate READMEs).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state
otherwise, any contribution intentionally submitted for inclusion in the work
by you shall be dual licensed as above, without any additional terms or
conditions.
