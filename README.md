# hl7kit · mwlkit · fhirkit · dicomscope

[![CI](https://github.com/holg/hl7v2/actions/workflows/ci.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/ci.yml)
[![demo](https://github.com/holg/hl7v2/actions/workflows/pages.yml/badge.svg)](https://holg.github.io/hl7v2/)
[![fhir validation](https://github.com/holg/hl7v2/actions/workflows/fhir.yml/badge.svg)](https://holg.github.io/hl7v2/fhir/)
[![publish](https://github.com/holg/hl7v2/actions/workflows/publish.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/publish.yml)
[![python wheels](https://github.com/holg/hl7v2/actions/workflows/python.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/python.yml)
[![crates.io hl7kit](https://img.shields.io/crates/v/hl7kit.svg?label=crates.io%20hl7kit)](https://crates.io/crates/hl7kit)
[![crates.io mwlkit](https://img.shields.io/crates/v/mwlkit.svg?label=crates.io%20mwlkit)](https://crates.io/crates/mwlkit)
[![crates.io fhirkit](https://img.shields.io/crates/v/fhirkit.svg?label=crates.io%20fhirkit)](https://crates.io/crates/fhirkit)
[![docs.rs hl7kit](https://docs.rs/hl7kit/badge.svg)](https://docs.rs/hl7kit)
[![docs.rs mwlkit](https://docs.rs/mwlkit/badge.svg)](https://docs.rs/mwlkit)
[![docs.rs fhirkit](https://docs.rs/fhirkit/badge.svg)](https://docs.rs/fhirkit)
[![PyPI hl7kit](https://img.shields.io/pypi/v/hl7kit?label=pypi%20hl7kit)](https://pypi.org/project/hl7kit/)
[![PyPI mwlkit](https://img.shields.io/pypi/v/mwlkit?label=pypi%20mwlkit)](https://pypi.org/project/mwlkit/)
[![PyPI fhirkit-rs](https://img.shields.io/pypi/v/fhirkit-rs?label=pypi%20fhirkit-rs)](https://pypi.org/project/fhirkit-rs/)
[![license](https://img.shields.io/crates/l/hl7kit.svg)](#license)
![MSRV 1.85](https://img.shields.io/badge/MSRV-1.85-blue.svg)

Rust for the seam between the radiology information system (RIS), the
archive (PACS/VNA), modalities, and FHIR: the HL7 v2 order that schedules an
examination, the DICOM Modality Worklist entry the modality reads, the images
that return, and the FHIR resources that describe the entire workflow.

Everything is pure Rust with `#![forbid(unsafe_code)]` in library crates
(except the thin Objective-C bridge on iOS), zero runtime network dependencies,
no panics on untrusted input data, and full `wasm32-unknown-unknown` support.

---

## Overview

| Component | What it does | Links & Packages |
|---|---|---|
| **hl7kit** | Dependency-free HL7 v2.x parser and builder with exact byte spans for every element, plus IHE imaging-order identifier extraction (Accession, Study Instance UID, Requested Procedure ID, SPS ID) with source tracking and conflict detection. | [crates.io](https://crates.io/crates/hl7kit) · [PyPI](https://pypi.org/project/hl7kit/) · [`crates/hl7kit`](crates/hl7kit) |
| **mwlkit** | Turns an HL7 order into a DICOM Modality Worklist item per IHE RAD-4, tracking mapping decisions, reporting generated values, and writing Part 10 files. | [crates.io](https://crates.io/crates/mwlkit) · [PyPI](https://pypi.org/project/mwlkit/) · [`crates/mwlkit`](crates/mwlkit) |
| **fhirkit** | Zero-dependency, high-performance FHIR R4 models for all 146 resources and datatypes. Lossless JSON reader/writer preserving exact decimal precision and primitive extensions; parallel parsing across cores. | [crates.io](https://crates.io/crates/fhirkit) · [PyPI (fhirkit-rs)](https://pypi.org/project/fhirkit-rs/) · [`crates/fhirkit`](crates/fhirkit) |
| **dicomscope** | In-browser DICOM viewer and CLI tool. Links studies to orders, highlights raw HL7 spans, builds worklist entries and MII-profiled FHIR R4 bundles. Audited 100% client-side with zero network calls after load. | [Live Demo](https://holg.github.io/hl7v2/) · [`demo/dicomscope`](demo/dicomscope) |
| **dicomscope-desktop** | Native desktop (Linux, macOS, Windows) and iPad DICOM viewer. Native file dialogs, cine playback, SR and encapsulated PDF documents, Metal/Vulkan/DX12/OpenGL backends. | [`demo/dicomscope-desktop`](demo/dicomscope-desktop) |
| **dicomscope-tui** | Terminal viewer and scriptable CLI tool. Inline GPU-rendered slices via Kitty, iTerm2, Sixel, or Unicode half-blocks, plus standalone `--window` mode via `ratatui-wgpu`. | [`demo/dicomscope-tui`](demo/dicomscope-tui) |

---

## Tech Stack

- **Language**: Rust (Edition 2021, MSRV 1.85), Python (3.9+ for PyO3 bindings), WGSL (GPU compute & fragment shaders).
- **Package Managers**: Cargo (Rust workspace), pip / maturin (Python packages).
- **Frameworks & Core Libraries**:
  - **Web**: [Leptos](https://leptos.dev) (CSR), [wgpu](https://wgpu.rs) (WebGPU), wasm-bindgen, [Trunk](https://trunkrs.dev).
  - **Desktop / Mobile**: [winit](https://github.com/rust-windowing/winit), [egui](https://github.com/emilk/egui), wgpu (Metal, Vulkan, DX12, OpenGL), xcodegen (iOS).
  - **Terminal / CLI**: [Ratatui](https://ratatui.rs), `ratatui-wgpu`, `crossterm`.
  - **DICOM**: `dicom-rs` (`dicom-pixeldata`, `dicom-object`, `dicom-dictionary-std`).
  - **Python Bindings**: [PyO3](https://pyo3.rs), [maturin](https://github.com/PyO3/maturin).

---

## Requirements & Prerequisites

- **Rust Toolchain**: Rust 1.85 or newer (`rustfmt` and `clippy` components).
- **Wasm Target** (for web demo and wasm checks):
  ```sh
  rustup target add wasm32-unknown-unknown
  ```
- **Trunk** (for running and building the browser demo):
  ```sh
  cargo install trunk --version 0.21.14
  ```
- **Python** (optional, for building and testing Python wheels):
  - Python 3.9+ with `maturin` and `pytest`.
- **Xcode & xcodegen** (optional, for iOS iPad build):
  - Xcode 15+, `xcodegen`, and `aarch64-apple-ios` target (`rustup target add aarch64-apple-ios`).

---

## Quick Start & Setup

### 1. Build and Run the Applications

#### Browser Demo (Leptos + WebGPU)
```sh
cd demo/dicomscope
trunk serve            # Open http://127.0.0.1:8080
trunk build --release  # Build static bundle in dist/
```

#### Native Desktop App (egui + wgpu)
```sh
# Run on desktop (Metal on macOS, Vulkan/DX12 on Linux/Windows, GL fallback)
cargo run --profile native -p dicomscope-desktop -- samples/anonymized-demo-clean.zip
```

#### Terminal Viewer & CLI (Ratatui + wgpu)
```sh
# Terminal viewer (auto-detects Kitty, iTerm2, Sixel, or half-blocks)
cargo run -p dicomscope-tui --release -- samples/anonymized-demo.zip samples/anonymized-demo.hl7

# Standalone native window mode (full resolution without terminal graphics)
cargo run -p dicomscope-tui --release -- --window samples/anonymized-demo.zip samples/anonymized-demo.hl7
```

#### iPad App
```sh
cd demo/dicomscope-desktop/ios
xcodegen generate
xcodebuild -project dicomscope.xcodeproj -scheme dicomscope -configuration Release \
  -destination "id=<device UDID>" -allowProvisioningUpdates -derivedDataPath build build
```

---

## CLI & Entry Points

### `dicomscope-tui` CLI Subcommands
The `dicomscope-tui` binary serves as both an interactive viewer and a scriptable CLI:

```sh
# Check study against order (exit code 0 = linked, 3 = mismatch / conflict)
cargo run -q -p dicomscope-tui -- link study.zip order.hl7 --json

# Inspect study metadata, series, skipped files
cargo run -q -p dicomscope-tui -- info study.zip --json

# Dump DICOM tags with filtering
cargo run -q -p dicomscope-tui -- tags study.zip --series 1 --slice 10 --grep patient

# GPU offscreen render of a slice to PNG
cargo run -q -p dicomscope-tui -- render study.zip --series 1 --slice 10 --window 40/400 -o slice.png

# Generate FHIR R4 Bundle from study + order
cargo run -q -p dicomscope-tui -- fhir study.zip order.hl7 -o bundle.json

# Build DICOM Modality Worklist item from order
cargo run -q -p dicomscope-tui -- worklist order.hl7 -o order.mwl.dcm

# List or extract embedded Structured Reports and PDFs
cargo run -q -p dicomscope-tui -- docs study.zip --show 1 -o report.txt
```

### `dicomscope` Host CLI
```sh
# Header check across DICOM files or folders
cargo run -p dicomscope -- samples/dicom/*.dcm

# Synthesize an HL7 ORM^O01 order matching a study
cargo run -p dicomscope -- order study.zip -o order.hl7

# Pack a folder or large export into a compact deflated zip
cargo run --release -p dicomscope -- pack exported-folder -o study.zip --series 1,5 --every 3

# Generate FHIR R4 Bundle
cargo run -q -p dicomscope -- fhir study.zip order.hl7 -o bundle.json
```

---

## Library Usage

### Rust

#### `hl7kit`
```rust
use hl7kit::{Message, order::Order};

let msg = Message::parse(text)?;
assert_eq!(msg.get("PID-3.1"), Some("4MR1"));
assert_eq!(msg.get("OBX[3]-5"), Some("three"));
let span = msg.get_span("ZDS-1.1")?; // Exact byte range in msg.raw()

let order = Order::extract(&msg);
println!("Study UID: {:?} (Source: {:?})", order.study_uid, order.study_uid_source);
```

#### `mwlkit`
```rust
use hl7kit::{Message, order::Order};
use mwlkit::{worklist_item, to_bytes, Input, Options};

let msg = Message::parse(text)?;
let order = Order::extract(&msg);
let item = worklist_item(&Input { message: &msg, order: &order }, &Options::default())?;
for w in &item.warnings { eprintln!("{w}"); }
std::fs::write("order.mwl.dcm", to_bytes(&item)?)?;
```

#### `fhirkit`
```rust
use fhirkit::r4::{Observation, ObservationValue};

// Strict parsing
let obs: Observation = fhirkit::from_json_strict(r#"{
  "resourceType": "Observation",
  "status": "final",
  "code": {"text": "Body weight"},
  "valueQuantity": {"value": 72.50, "unit": "kg"}
}"#)?;

// Lenient parsing: collects issues with exact JSON element paths
let (obs, issues) = fhirkit::from_json::<Observation>(json_text)?;
for issue in &issues {
    println!("{issue}");
}
```

### Python Wheels

Install via pip:
```sh
pip install hl7kit            # HL7 v2 parser, spans, and order extraction
pip install "mwlkit[pydicom]" # HL7 order -> DICOM Modality Worklist item
pip install fhirkit-rs        # FHIR R4 models & JSON engine (import as `fhirkit`)
```

```python
import hl7kit, mwlkit, fhirkit

# HL7 & MWL
msg = hl7kit.parse(open("order.hl7", "rb").read())
order = msg.order()
print(order.accession_number, order.study_uid.value, order.study_uid.source)

item = mwlkit.worklist_item(msg, default_station_ae="OPG1_AE")
dataset = mwlkit.to_pydicom(item)

# FHIR R4: parse (str, bytes or dict), every problem with its path, lossless JSON back
patient = fhirkit.parse('{"resourceType": "Patient", "name": [{"family": "Everyman", "given": ["Adam"]}], "gender": "male"}')
print(patient.resource_type, patient.issues)   # Patient []
for issue in fhirkit.validate({"resourceType": "Observation", "valueString": "x", "valueBoolean": True}):
    print(issue)                                # error at Observation.value[x]: only one type may be given ...
json_str = patient.to_json(pretty=True)
```

---

## Scripts

| Script | Description |
|---|---|
| `scripts/test-ci-locally.sh` | Local CI runner matching GitHub Actions: runs `cargo fmt`, clippy (host + wasm32), fetches fixtures, tests workspace, verifies no network APIs in demo, and builds/tests Python wheels if environment is set. |
| `scripts/test-ci-locally.sh --full` | Same as `scripts/test-ci-locally.sh`, plus compiles the release WebAssembly bundle with Trunk. |
| `crates/fhirkit/fetch-fixtures.sh` | Fetches the official FHIR R4 core package (`hl7.fhir.r4.core`), R4 examples package (`hl7.fhir.r4.examples`), and SQL-on-FHIR test suite into `tmp/fhir/`. |
| `samples/fetch-dicom.sh` | Downloads test DICOM files from pydicom test corpus into `samples/dicom/` (synthetic, no real patient data). |

---

## Environment Variables

| Variable | Scope / Tool | Purpose |
|---|---|---|
| `WGPU_BACKEND` | `dicomscope-tui`, `dicomscope-desktop` | Restricts wgpu backend selection (comma-separated: `metal`, `vulkan`, `dx12`, `gl`). |
| `DICOMSCOPE_TAGS` | `dicomscope` CLI | When set (e.g. `DICOMSCOPE_TAGS=1`), forces tag dump to print all DICOM tags. |
| `FHIRKIT_R4_EXAMPLES` | `crates/fhirkit` tests | Custom path to the uncompressed FHIR R4 examples directory (defaults to `tmp/fhir/examples/package`). |
| `LOCAL_CI_PYENV` | `scripts/test-ci-locally.sh` | Path to a virtualenv containing `maturin` and `pytest` for local wheel verification (can also be configured in `.local-ci.env`). |
| `PYO3_PYTHON` | Python binding builds | Path to the Python executable used by PyO3. |
| `PYTHONHOME`, `DYLD_LIBRARY_PATH`, `LD_LIBRARY_PATH` | PyO3 binding tests | Configures Python standard library and shared library locations for embedded interpreter tests. |
| `TERM`, `TERM_PROGRAM`, `TMUX` | `dicomscope-tui` | Used for terminal capability and graphics protocol autodetection. |

---

## Testing & Quality Assurance

```sh
# Run all workspace unit and integration tests
cargo test --workspace

# Host clippy (must pass with 0 warnings)
cargo clippy --workspace --all-targets -- -D warnings

# WebAssembly clippy checks
cargo clippy -p dicomscope --target wasm32-unknown-unknown -- -D warnings
cargo clippy -p fhirkit --target wasm32-unknown-unknown -- -D warnings

# FHIR R4 conformance tests (5,309 official round-trip examples)
crates/fhirkit/fetch-fixtures.sh
cargo test -p fhirkit --test r4_examples

# Python wheels build & tests (with maturin & pytest installed)
scripts/test-ci-locally.sh
```

### No-Network Guarantee Audit
CI and `scripts/test-ci-locally.sh` enforce that no network APIs exist in the client-side viewer crates:
```sh
grep -rnE '(^|[^A-Za-z0-9_])(fetch\(|XMLHttpRequest|WebSocket|EventSource)' \
  demo/dicomscope/src demo/dicomscope-core/src demo/dicomscope-desktop/src crates/hl7kit/src crates/mwlkit/src crates/fhirkit/src
```

---

## Project Structure

```text
hl7v2/
├── Cargo.toml                  # Workspace root configuration & build profiles
├── scripts/test-ci-locally.sh  # Local CI runner script
├── RELEASING.md                # Publishing and release instructions
├── crates/
│   ├── hl7kit/                 # Dependency-free HL7 v2 parser, builder & order extractor
│   ├── hl7kit-py/              # PyO3 bindings for hl7kit (PyPI: hl7kit)
│   ├── mwlkit/                 # HL7 order to DICOM Modality Worklist (IHE RAD-4)
│   ├── mwlkit-py/              # PyO3 bindings for mwlkit (PyPI: mwlkit)
│   ├── fhirkit/                # Zero-dependency FHIR R4 typed models & lossless JSON engine
│   ├── fhirkit-py/             # PyO3 bindings for fhirkit (PyPI: fhirkit-rs)
│   └── fhirkit-gen/            # FHIR R4 StructureDefinition code generator for fhirkit
├── demo/
│   ├── dicomscope-core/        # Shared core: DICOM scanner, WGSL renderer, linkage logic, FHIR
│   ├── dicomscope/             # Browser-based viewer (Leptos + WebGPU) and CLI tool
│   ├── dicomscope-desktop/     # Native desktop app (winit + egui + wgpu) & iOS iPad app (ios/)
│   └── dicomscope-tui/         # Terminal viewer (Ratatui + wgpu) and scriptable CLI
└── samples/                    # HL7 test messages, test scripts & DICOM corpus fetcher
```

---

## Releasing

Artefacts are released independently via GitHub Actions using Trusted Publishing (OIDC):
- `hl7kit-vX.Y.Z`, `mwlkit-vX.Y.Z` and `fhirkit-vX.Y.Z` tags publish Rust crates to [crates.io](https://crates.io).
- `hl7kit-py-vX.Y.Z`, `mwlkit-py-vX.Y.Z`, and `fhirkit-py-vX.Y.Z` tags build and publish multi-platform wheels to [PyPI](https://pypi.org) with provenance attestations and GitHub releases.
- The web demo is continuously deployed to GitHub Pages from pushes to `main`.

See [RELEASING.md](RELEASING.md) for step-by-step instructions.

---

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you shall be dual licensed as above, without any additional terms or conditions.
