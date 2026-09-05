# Coding prompt — dicomscope

A browser-only DICOM viewer with HL7v2 linkage. **Pure Rust, no JavaScript, no
TypeScript, no npm.** Leptos CSR + wgpu + dicom-rs, compiled to
`wasm32-unknown-unknown`, served by Trunk.

> **Revision 2026-09-05.** The viewer is the *demo*; the deliverable is the
> `hl7kit` library crate in `crates/hl7kit`, which this page exercises. The
> workspace layout in section 2, the HL7 module in section 5 and several
> technical details were corrected while building it:
>
> - section 4: `dicom_object::from_reader` consumes the `DICM` magic itself,
>   so skip 128 bytes, not 132;
> - section 5: `ZDS-1` is a composite (`uid^^Application^DICOM`), compare
>   component 1; MSH-2 must not be split on its own delimiters; escape
>   sequences are decoded;
> - sections 3, 4, 7: pixel values are `f32` in an `R32Float` texture, not
>   saturated `i16`, so unsigned 16-bit data and fractional slopes survive;
>   upload pads rows to 256 bytes; only frame 0 is decoded;
> - section 1: verified here with Rust 1.96.0 (the draft said 1.98.0); an old
>   `wasm-opt` needs the feature flags `index.html` passes via
>   `data-wasm-opt-params`.

---

## 0. Non-negotiables

1. **No JavaScript authored by hand.** The only `.js` in the repository is what
   `wasm-bindgen` generates. No inline `<script>` beyond what Trunk injects. No
   npm, no package.json, no bundler. Anything the DOM needs goes through
   `web-sys` / `wasm-bindgen`.
2. **No network I/O after the page loads.** No `fetch`, no XHR, no WebSocket, no
   telemetry, no remote fonts, no CDN. Files come from a local `<input
   type="file">` only. This is the product's reason to exist: HL7v2 messages and
   DICOM instances contain patient data, so every server-side viewer is unusable
   in a hospital. Make the guarantee auditable — a reviewer should be able to
   grep the crate for `fetch` and find nothing.
3. **No `unwrap()` / `expect()` on anything derived from user input.** Malformed
   files are the normal case, not the exception. Every failure path ends in a
   message on screen naming what went wrong.

---

## 1. Verified environment

All of the following was compiled and run before this prompt was written. Treat
it as ground truth and do not re-investigate.

Toolchain: Rust 1.96.0, target `wasm32-unknown-unknown`, Trunk 0.21.14. Note
that Rust 1.87+ emits bulk-memory and related wasm features by default; a
`wasm-opt` older than version 119 rejects them unless passed
`--enable-bulk-memory` etc., which `index.html` does through
`data-wasm-opt-params`.

Versions that build together against wasm32 (verified, 3m19s cold):

```toml
leptos                = { version = "0.8.20", features = ["csr"] }
wgpu                  = "30.0.1"
dicom-object          = "0.10.0"
dicom-pixeldata       = { version = "0.10.0", default-features = false, features = ["jpeg", "native", "rle", "deflate"] }
bytemuck              = { version = "1.25.2", features = ["derive"] }
web-sys               = "0.3.105"
wasm-bindgen          = "0.2.128"
wasm-bindgen-futures  = "0.4.78"
js-sys                = "0.3.105"
console_error_panic_hook = "0.1.7"
```

Release profile that produced 980 KB raw / 366 KB gzipped for the DICOM half
alone (expect roughly 2–3 MB gzipped once wgpu and Leptos are in — measure it,
put the number in the README, do not guess):

```toml
[profile.release]
opt-level     = "z"
lto           = true
strip         = true
codegen-units = 1
panic         = "abort"
```

### Pixel decoding — confirmed against real files

Tested with the pydicom corpus (`raw.githubusercontent.com/pydicom/pydicom/main/src/pydicom/data/test_files/`):

| Transfer syntax | UID | Result |
| --- | --- | --- |
| Explicit VR Little Endian | 1.2.840.10008.1.2.1 | decodes, 64×64, 16 bit |
| RLE Lossless | 1.2.840.10008.1.2.5 | decodes, 64×64, 16 bit |
| JPEG Lossless P14 SV1 | 1.2.840.10008.1.2.4.70 | decodes, 100×100, 8 bit |

Implicit VR LE and JPEG Baseline are in the same pure-Rust path and are expected
to work; verify and record.

**Trap:** `TransferSyntax::is_fully_supported()` returns `false` for RLE and
`.70` even though decoding succeeds. It measures full codec support including
encoding. Never use it to decide whether to attempt a decode. Attempt, and report
the real error.

### Dead ends — do not attempt

- **JPEG 2000 (`.90`, `.91`).** The `openjp2` crate (pure-Rust c2rust port) is at
  0.6.1 and declares `crate-type = ["cdylib", "staticlib", "rlib"]`. Cargo therefore
  links it as a standalone cdylib, which fails on undefined `free` under
  `wasm32-unknown-unknown` (no libc). Supplying `malloc`/`calloc`/`free`/`realloc`
  shims in the *consuming* crate does not help — the failing link step happens
  inside `openjp2` itself, before your symbols are in scope. A fix requires a fork
  restricting `crate-type` to `rlib` and carrying the shims internally. Out of
  scope here; mention it in the README as a known limitation.
- `openjpeg-sys`, `gdcm` bindings — C, unusable in WASM.
- JPEG-LS (`.80`, `.81`) — no pure-Rust decoder wired into dicom-pixeldata.

---

## 2. Workspace layout

Two crates. The library is the product; the demo is a single flat-module
binary that depends on it. Browser-only dependencies (leptos, wgpu, web-sys)
are declared under `[target.'cfg(target_arch = "wasm32")']` so host `cargo
test` never compiles them.

```
hl7kit/
  Cargo.toml            workspace; release profile
  README.md             library-first overview
  docs/hl7v2.md         this document
  samples/
    README.md           where to get test files
    order.hl7           synthetic ORM^O01 matching pydicom MR_small.dcm
    order-mismatch.hl7  same study UID, different patient
  crates/hl7kit/         the library (no dependencies)
    src/
      lib.rs
      encoding.rs       MSH-1/MSH-2 delimiters
      escape.rs         \F\ \S\ \T\ \R\ \E\ \X..\ \.br\
      message.rs        parser + Segment/Field/Repetition/Component views with spans
      path.rs           "PID-3[2].1" query syntax
      order.rs          the four IHE order identifiers
    tests/parse.rs
  demo/dicomscope/
    Cargo.toml
    Trunk.toml
    index.html          Trunk entry, inline CSS, no external resources
    README.md
    src/
      main.rs           mount, panic hook; host build is a stub
      app.rs            root component, top-level state
      error.rs          AppError
      dicom/
        mod.rs          end-to-end host tests on a synthetic Part 10 file
        load.rs         bytes -> FileDicomObject (preamble detection)
        study.rs        the identifiers the linkage needs
        tags.rs         flat tag list for display
        pixels.rs       decode frame 0 + rescale + photometric flag
        transfer_syntax.rs  what decodes, what does not, and why
      link/mod.rs       resolution strategy + mismatch detection
      render/
        gpu.rs          wgpu init, texture upload, draw
        shader.wgsl
      ui/
        file_drop.rs    <input type=file> + drag/drop, no JS
        controls.rs     window sliders, presets
        link_panel.rs
        hl7_view.rs     raw message with spans highlighted
        tag_tree.rs
  .github/workflows/ci.yml
```

---

## 3. Data model

Keep the domain types free of Leptos and wgpu. They should be testable with
plain `cargo test` on the host target.

```rust
pub struct Study {
    pub patient_id:        Option<String>,   // (0010,0020)
    pub accession_number:  Option<String>,   // (0008,0050)
    pub study_uid:         Option<String>,   // (0020,000D)
    pub transfer_syntax:   String,           // (0002,0010)
    pub rows: u32,
    pub cols: u32,
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,          // after rescale, before windowing
    pub default_window: Option<(f32, f32)>,   // (0028,1050), (0028,1051)
    pub value_range: (f32, f32), // for fallback windowing and slider ranges
    pub inverted: bool,          // MONOCHROME1
    pub frames_skipped: u32,
    // plus bits_stored, photometric, rescale for the overlay
}

// From the hl7kit crate (hl7kit::order):
pub struct Order {
    pub patient_id:   Option<String>,  // PID-3.1
    pub accession:    Option<String>,  // OBR-18
    pub procedure_id: Option<String>,  // OBR-19
    pub study_uid:    Option<String>,  // ZDS-1.1
    pub spans: Vec<(Span, OrderField)>,  // byte ranges for highlighting
}

pub enum OrderField { PatientId, Accession, ProcedureId, StudyUid }
```

`f32` rather than `i16`: MR and CR images store unsigned 16-bit values above
32767, and rescale slopes are frequently fractional. Saturating to `i16` would
silently clip them; `f32` costs 4 bytes per pixel and nothing else.

---

## 4. DICOM module

### load.rs

```rust
pub fn load(bytes: &[u8]) -> Result<FileDicomObject<InMemDicomObject>, LoadError>
```

Skip the 128-byte preamble **but not the `DICM` magic**: dicom-rs's reader
consumes the magic itself, so position it at offset 128 and pass
`ReadPreamble::Never`. Reject inputs without the magic with an error that says
how long the file was. Some files in the wild lack the preamble — if the magic
is at offset 0, accept that too.

### tags.rs

Produce a flat display list, not a nested tree — sequences expand one level with
an indent depth field. Each row: tag as `(gggg,eeee)`, keyword when known, VR,
and a truncated value string (cap at ~120 chars, mark truncation). Binary VRs
(`OB`, `OW`, `UN`) show length only, never a hex dump.

### pixels.rs

```rust
pub fn decode_first_frame(obj: &FileDicomObject<InMemDicomObject>) -> Result<Frame, PixelError>
```

Order of operations, and this order matters:

1. `obj.decode_pixel_data()`.
2. Apply Rescale Slope (0028,1053) and Rescale Intercept (0028,1052). Default to
   1.0 and 0.0 when absent. Ask dicom-pixeldata for raw stored values
   (`ModalityLutOption::None`, `VoiLutOption::Identity`) and apply the rescale
   in a pure, tested function. Store as `f32`.
3. Read Photometric Interpretation (0028,0004). `MONOCHROME1` means low values
   are bright — set `inverted`, do not pre-invert the buffer. Inversion belongs
   in the shader.
4. Read Window Center (0028,1050) and Width (0028,1051). These may be
   multi-valued — take the first. When absent, compute min/max and use
   `center = (min+max)/2`, `width = max-min`.
5. Colour images (`RGB`, `YBR_FULL_422`) are out of scope for v1: detect and
   report "colour images not supported yet" rather than rendering garbage.

Frame 0 only, via `decode_pixel_data_frame(0)` so the other frames are never
decoded. If `NumberOfFrames > 1`, note in the overlay how many frames were
skipped.

---

## 5. HL7 module

This is the `hl7kit` crate. The demo does not parse HL7 itself; it calls
`hl7kit::Message::parse` and `hl7kit::order::Order::extract`. The requirements
below are what the crate implements and tests.

### Parser

Segments are separated by `\r` (carriage return, 0x0D). Tolerate `\r\n` and bare
`\n` because real-world files have been through Windows editors. Fields split on
`|`, components on `^`, repetitions on `~`. The encoding characters are declared
in MSH-2 — read them rather than hardcoding, but fall back to the defaults
`^~\&` when MSH-2 is malformed.

Field numbering trap: in MSH, MSH-1 *is* the field separator, so MSH-2 is the
first element after the split. In every other segment, index 0 of the split is
the segment name and field *n* is at index *n*. Get this right or every value
will be off by one in MSH only.

MSH-2 holds the delimiters themselves (`^~\&`) and must be treated as an
opaque field, never split on the component separator it contains.

Escape sequences (`\F\`, `\S\`, `\T\`, `\R\`, `\E\`, `\Xdd..\`, `\.br\`)
are decoded on demand, never in place, so spans always index the raw text.
Unknown sequences are kept verbatim.

Return byte spans alongside values so the raw-message view can highlight in place
without re-searching the text.

### Order fields (`hl7kit::order`)

| Field | Location | Notes |
| --- | --- | --- |
| Patient identifier | `PID-3.1` | may repeat; take the first |
| Accession number | `OBR-18` | Placer Field 1 per IHE RAD-TF |
| Requested procedure ID | `OBR-19` | |
| Study Instance UID | `ZDS-1.1` | `ZDS-1` is `uid^^Application^DICOM`; only component 1 is the UID. Vendor Z-segment, frequently absent |

Some sites put the study UID in `OBR-3` instead. Do not chase that in v1, but
leave a comment where it would go.

---

## 6. Linkage — the actual point

This module is the reason the project exists. Everything else is scaffolding
around it.

```rust
pub enum LinkPath {
    StudyUid,        // ZDS-1 == (0020,000D), exact
    Accession,       // OBR-18 == (0008,0050), fallback
    None,
}

pub struct Linkage {
    pub path: LinkPath,
    pub patient_match: Option<bool>,  // None when either side is absent
}

pub fn resolve(study: &Study, order: &Order) -> Linkage
```

Resolution order: study UID first, accession second, otherwise none. Compare
after trimming whitespace; DICOM string values are frequently space-padded to
even length.

Independently of the link, compare `PID-3` against Patient ID (0010,0020).
**A successful link with disagreeing patient identifiers is the headline case.**
Render it prominently, in the danger role, not as a footnote. That combination —
the study matches but the patient does not — is what integration engineers spend
their afternoons hunting, and no browser tool surfaces it today.

The UI must state, in one sentence, why the fallback exists: `ZDS` is a
vendor-defined Z-segment from the IHE Radiology Technical Framework, not part of
HL7v2 proper, so many sites never populate it — and when the study UID is absent,
archives such as dcm4che derive one from the requested procedure ID or the
accession number, or generate a random one. The identifier that looks canonical
may have been invented downstream.

---

## 7. Rendering

### The integration problem

wgpu's device and queue are not `Send`/`Sync`-friendly to move through Leptos
signals, and adapter acquisition is async. Do not fight this:

- Hold the renderer in `Rc<RefCell<Option<Renderer>>>`, created once in an
  `Effect` after the canvas node ref is mounted.
- Initialise inside `wasm_bindgen_futures::spawn_local`.
- Get the surface with
  `instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas))` from the
  `HtmlCanvasElement` behind a `NodeRef`.
- Keep pixel data *out* of signals. A signal holds `Option<Study>` and window
  parameters; the `Vec<i16>` goes straight to the GPU and is dropped from Rust
  memory or kept in the renderer, never cloned into reactive state.

If adapter request fails, set an error signal naming the requirement (WebGPU) and
render a message in place of the canvas. A blank black rectangle is not an error
report.

### Texture format

`R32Float`, read with `textureLoad` (no sampler). Nothing is filtered, so the
`float32-filterable` feature is unnecessary and must not be requested; at 1:1
display you want the actual sample values. Do not request any optional feature.
If magnified zoom is added later, interpolate manually in the shader over four
texels rather than changing format.

Rows are padded to `COPY_BYTES_PER_ROW_ALIGNMENT` (256 bytes) on upload. A
64-pixel-wide `f32` image is exactly 256 bytes per row; anything else needs the
padding.

Upload once per loaded file. Window changes update a uniform buffer only.

### shader.wgsl

Sketch — the windowing must live here, not on the CPU:

```wgsl
struct Win {
    center:   f32,
    width:    f32,
    invert:   u32,
    _pad:     u32,
};

@group(0) @binding(0) var img: texture_2d<f32>;
@group(0) @binding(1) var<uniform> win: Win;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(img));
    let p    = vec2<i32>(clamp(uv * dims, vec2<f32>(0.0), dims - 1.0));
    let v    = textureLoad(img, p, 0).r;

    // PS3.3 C.11.2.1.2.1 linear VOI LUT: y = (x - (c - 0.5)) / (w - 1) + 0.5
    let w  = max(win.width, 1.0);
    var g  = clamp((v - (win.center - 0.5)) / max(w - 1.0, 1e-3) + 0.5, 0.0, 1.0);
    if (win.invert == 1u) { g = 1.0 - g; }

    return vec4<f32>(g, g, g, 1.0);
}
```

Full-screen triangle, no vertex buffer. Redraw on uniform change only.

---

## 8. UI

Single column on narrow viewports, two columns above ~900px. Order top to bottom:

1. **Load bar.** Two file inputs, DICOM and HL7, each also a drop target. Use
   `web-sys` `DragEvent` and `DataTransfer` — no JS shims. Read bytes via
   `File::array_buffer()` and `JsFuture`.
2. **Canvas** with a small overlay showing modality, dimensions, bit depth, and
   the live `C … / W …` readout.
3. **Window controls.** Two sliders plus three preset buttons: soft tissue
   40/400, lung −600/1500, bone 500/2000. Sliders must feel continuous — this is
   the visible payoff of doing the windowing on the GPU, so do not debounce them
   into sluggishness.
4. **Linkage panel.** Which path resolved, the compared values side by side, and
   the patient-mismatch warning when it applies.
5. **HL7 raw view** with the four extracted fields highlighted in place.
6. **DICOM tag list**, scrollable, with a filter box.

Before both files are loaded, show what is missing, not an empty frame.

---

## 9. Errors

One `AppError` enum with `Display`, surfaced in a banner. Distinguish at minimum:
not a DICOM file, unsupported transfer syntax (name the UID and the reason),
colour image, decode failure with the underlying dicom-rs error, malformed HL7
(name the segment), no WebGPU adapter.

The unsupported-transfer-syntax message should be specific and useful: name the
UID, say it is not supported in the browser build, and for JPEG 2000 say why.

---

## 10. Build and run

`Trunk.toml`, `index.html` with `<link data-trunk rel="rust" data-wasm-opt="z" />`.
`trunk serve` for development, `trunk build --release` for the artifact. No other
toolchain. The README states the exact commands and the measured gzipped size.

Include a GitHub Actions workflow that runs `cargo fmt --check`, `cargo clippy
--target wasm32-unknown-unknown -- -D warnings`, host-target `cargo test` for the
domain modules, and `trunk build --release`.

---

## 11. Tests

Domain logic must be testable without a browser. Feature-gate or simply avoid
`web-sys` in `dicom/`, `hl7/`, and `link/` so `cargo test` runs on the host.

- HL7 parser (in the crate): MSH off-by-one, `\r\n` and `\n` line endings,
  absent ZDS, ZDS-1 component extraction, custom encoding characters, malformed
  MSH-2 fallback, empty and null trailing fields, repetitions and
  subcomponents, escape decoding, MLLP framing and BOM, spans indexing the raw
  text, and a no-panic sweep over degenerate inputs.
- DICOM (host, on a synthetic Part 10 file built in memory): preamble and
  no-preamble load, study identifiers with padding trimmed, tag list with binary
  VRs shown as lengths, colour rejection, unsupported transfer syntax naming
  its reason.
- Linkage: all three paths, plus the linked-but-patient-mismatch case, plus
  whitespace-padded DICOM values.
- Rescale: slope/intercept applied, defaults when absent, MONOCHROME1 flag set
  rather than buffer pre-inverted.

Ship the sample HL7 message in `samples/` and use it as a fixture.

---

## 12. Non-goals for v1

Named explicitly so scope does not drift:

multi-frame and series scrolling · MPR and 3D reconstruction · DICOMDIR ·
C-FIND/C-STORE/DICOMweb · JPEG 2000 and JPEG-LS · HL7v2 conformance validation
against segment tables · FHIR · measurement and annotation tools · persistence,
accounts, settings · colour images · WebGL2 fallback

Series scrolling is the obvious v2, and the groundwork exists: partially-bound
binding arrays over per-slice textures. Not now.

---

## 13. README requirements

The README carries as much weight as the code, because it is what a reviewer
reads first. It must contain:

- One paragraph on why a browser-local viewer exists at all — the compliance
  argument, stated plainly.
- The supported transfer syntax table, with the three verified entries marked as
  verified.
- The unsupported list with reasons, including the `openjp2` crate-type problem
  in one honest sentence.
- The measured release bundle size, gzipped.
- The no-network guarantee and how to verify it.
- A note that the linkage fallback exists because ZDS is a vendor Z-segment.

Boring, accurate, no marketing. The audience is people who have debugged an HL7
interface at two in the morning.
