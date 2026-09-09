# dicomscope

A browser-only DICOM viewer with HL7 v2 order linkage. Pure Rust: Leptos CSR,
wgpu, dicom-rs, compiled to `wasm32-unknown-unknown` and served by Trunk. No
hand-written JavaScript, no npm.

This is the demo application for the [`hl7kit`](../../crates/hl7kit) crate. The
viewer is scaffolding; the point is the linkage panel and the highlighted HL7
message underneath it, both driven by the byte spans the crate returns.

## Why a browser-local viewer exists

HL7 v2 messages and DICOM instances contain patient data. A hospital
integration engineer who needs to check why a study did not attach to its order
cannot paste either file into a hosted tool, and installing a desktop viewer on
a locked-down workstation is often impossible. A page that runs entirely in the
browser, reads files from a local `<input type="file">`, and makes no network
requests after load is the only shape that is usable in that environment. The
no-network guarantee has to be auditable, not promised; see below.

## What it shows

1. Load DICOM files, a folder, or a zip, plus an HL7 order (ORM^O01 with a
   `ZDS` segment, or OMI^O23 with `IPC`; `IPC-3` is read first, `ZDS-1`
   second). Every file is header-scanned, grouped into series by Series
   Instance UID, and sorted by Image Position along the slice normal (falling
   back to Instance Number). Multi-frame files contribute one slice per frame.
   DICOMDIR and non-image files are listed as skipped, with reasons.
2. The image renders with GPU windowing. Sliders and presets update a 16-byte
   uniform; the pixels are uploaded once.
3. The linkage panel resolves the study to the order by Study Instance UID
   (`ZDS-1.1` against (0020,000D)) first, Accession Number (`OBR-18` against
   (0008,0050)) second.
4. Independently, Patient ID (0010,0020) is compared with `PID-3.1`. **A linked
   study whose patient identifiers disagree is shown in the danger role.** That
   combination is what integration engineers spend their afternoons hunting.
5. The raw HL7 message is shown with the four identifiers highlighted in place,
   and every DICOM tag is listed with a filter.

Viewer controls: the mouse wheel scrolls through the slices of the current
series (Ctrl/Cmd+wheel zooms at the cursor; on a single image the wheel zooms).
Dragging pans, double-click fits. With the viewer focused (click it), the arrow
keys move the image (Shift for larger steps), PageUp/PageDown step slices and
Home/End jump to the ends, `+`/`-` zoom, `0` fits, `1` shows one image pixel
per device pixel, `i` toggles bilinear interpolation, `r`/`R` rotate by 90
degrees, `h`/`v` flip, `o` resets orientation, space starts and stops cine.
The series buttons (with thumbnails) and the slice slider under the image do
the same. Window and view are kept while scrolling within a series and reset
when a different series is chosen.

Reading tools, the things a clinician reaches for first:

- **Rotate and flip** are part of the view transform. The shader maps every
  fragment canvas → display → source with the same formulas as `view.rs`, so
  measurements drawn on a rotated image land on the right pixels.
- **Length and angle.** Pick the tool (`l`, `a`, or the buttons), drag for a
  length, click three points for an angle. Values are in millimetres from
  Pixel Spacing (0028,0030), computed in physical space so non-square pixels
  do not distort angles. Imager Pixel Spacing (0018,1164) is used as a
  fallback and marked with `*`, since it measures at the detector. Without
  either, lengths are in pixels. Measurements are kept per slice while
  scrolling. Delete removes the last one, Escape returns to Pan.
- **Cine** plays the current series or multi-frame file at Frame Time
  (0018,1063) or Cine Rate (0018,0040), 10 frames per second when neither is
  present, wrapping at the end.
- **Thumbnails** are the middle slice of each series, windowed with the file's
  own window, painted through a 2D canvas (the only 2D canvas in the app).
- **Documents.** Structured Reports and Key Object Selections are rendered as
  an indented tree of concept names and values (text, numbers with units,
  codes, dates, references). Encapsulated PDFs open in the browser's own PDF
  viewer through a blob URL, which stays inside the tab. Both are listed next
  to the series; a study that contains only a report still loads.

Each slice is decoded from the file's bytes when it is shown and uploaded as
one texture; nothing is pre-decoded. Zips are never expanded: the scan reads
each entry as a stream and inflates only as far as the header reader consumes
(a few kilobytes of a half-megabyte CT slice), and an entry is inflated again
when its slice is shown. Memory is therefore the archive as given plus one
frame, and scanning a 250 MB deflated study takes well under a second. Entries
under `__MACOSX/`, `._*` AppleDouble files, `.DS_Store` and `Thumbs.db` are
ignored without a parse attempt. Dropping a folder onto the page is not
supported (it needs a non-standard directory-entry API); use the folder
picker.

Note on dev builds: `trunk serve` ships the `dev` profile. The workspace sets
`opt-level = 3` for dependencies in that profile so miniz_oxide and dicom-rs
run at full speed while our own code stays debuggable; without it the same
scan was measured 6x slower.

The accession fallback exists because `ZDS` is a vendor-defined Z-segment from
the IHE Radiology Technical Framework, not part of HL7 v2 proper, so many sites
never populate it; and when the study UID is absent, archives such as dcm4che
derive one from the requested procedure ID or the accession number, or generate
a random one. The identifier that looks canonical may have been invented
downstream.

## FHIR output

Once a study and an order are loaded, the app generates a FHIR R4 (4.0.1)
`Bundle` of type `collection` with three resources: `Patient`,
`ServiceRequest` and `ImagingStudy`. The FHIR panel shows each as
pretty-printed JSON with copy and download; both use local browser APIs, no
request leaves the page. Nothing is validated online: no `$validate`, no
terminology server, no profile package download. The mapping is built as
plain JSON (`serde_json`), not with a typed FHIR crate, to keep the bundle
size a published number.

The `ImagingStudy` declares the MII (Medizininformatik-Initiative) Modul
Bildgebung profile
`https://www.medizininformatik-initiative.de/fhir/ext/modul-bildgebung/StructureDefinition/mii-pr-bildgebung-bildgebungsstudie`,
version 2025.0.0-ballot, the version read when writing the mapping. *The
ImagingStudy declares the MII Bildgebung profile and populates its core
elements; it is not validated against the profile, and the modality-specific
extensions are not emitted.* Element choices follow the HL7 "Version 2 to
FHIR" implementation guide (PID → Patient, ORC/OBR → ServiceRequest) and the
R4 ImagingStudy definition; every element a mapping maintainer would question
has a one-line comment in `src/fhir/` saying where it comes from.

Identifier rules, in three lines:

- Study Instance UIDs use `system: urn:dicom:uid` with `urn:oid:` values;
  the study UID appears on both `ImagingStudy` and `ServiceRequest`, the row
  generic v2-to-FHIR mappers leave out.
- Accession, MRN and order numbers get a `v2-0203` type (`ACSN`, `MR`,
  `PLAC`, `FILL`); a `system` only when the message carries a URI or ISO OID
  in the assigning authority (PID-3.4, EI.2–4), else `assigner.display`. No
  system is fabricated when the message carries no assigning authority.
- Requested Procedure ID has no `v2-0203` type; it is carried untyped and the
  link panel says so.

Timezone rule, in one line: a time of day is emitted only when an offset is
known, (0008,0201) for DICOM or the trailing `±ZZZZ` for HL7; otherwise the
date alone, because a naive `dateTime` is invalid FHIR.

`ImagingStudy.basedOn` is set only when the study actually linked to the
order; an unlinked study asserts no relationship. The link panel's third
column shows where every identifier lands in the bundle, together with the
HL7 path that supplied it (`IPC-3.1` or `ZDS-1.1` for the study UID) and any
`ConflictingStudyUid` warning from several `IPC` segments.

Every bundle entry carries a `fullUrl` of the form `urn:uuid:…`, and the
references between entries use the same URNs. FHIR requires a `fullUrl` on
every entry of a bundle that is not a transaction or batch, and the HL7
validator rejects a collection without one, contrary to the design prompt's
assumption. The UUIDs are RFC 9562 version 8 values hashed from the Study
Instance UID and the resource type, so the same study yields the same bundle
on every run, in the browser and on the command line, with no random source
and no dependency.

### Validation

The host binary emits the same bundle the browser builds:

```sh
cargo run --release -p dicomscope -- fhir samples/dicom/MR_small.dcm samples/order.hl7 -o bundle.json
java -jar validator_cli.jar bundle.json -version 4.0.1 \
  -ig de.medizininformatikinitiative.kerndatensatz.bildgebung#2025.0.0-ballot
```

Run on 2026-09-09 with validator 6.10.4 over four bundles (ORM^O01 and
OMI^O23 against `MR_small.dcm`, the patient-mismatch order, and the 970-slice
anonymised CT with its generated order): **0 errors** in each, with the MII
profile applied to the `ImagingStudy`. The remaining warnings, and why they
stay:

- `dom-6`, "a resource should have narrative": best practice, not a
  requirement; no narrative is generated.
- `Coding has no system` on `ServiceRequest.code`: OBR-4.3 was `L` (a local
  table name, not a URI), so the code is kept without a system rather than
  given a fabricated one.
- `ValueSet … dicom.nema.org … not found` on modality and `sopClass`: the
  validator cannot fetch DICOM's value sets; the codes are the DICOM ones.

The validator is a development tool and is not part of the demo; the page
still makes no network request.

## Supported transfer syntaxes

Pixel decoding is dicom-rs (`dicom-pixeldata` with the `native`, `jpeg`, `rle`
and `deflate` features). The list below is the same table the app uses to
explain failures (`src/dicom/transfer_syntax.rs`).

| Transfer syntax | UID | Status |
| --- | --- | --- |
| Explicit VR Little Endian | 1.2.840.10008.1.2.1 | **verified** (MR_small, CT_small, emri_small 10 frames) |
| Implicit VR Little Endian | 1.2.840.10008.1.2 | **verified** (MR_small_implicit) |
| Explicit VR Big Endian | 1.2.840.10008.1.2.2 | **verified** (MR_small_bigendian, MR_small_expb) |
| Deflated Explicit VR LE | 1.2.840.10008.1.2.1.99 | **verified** (image_dfl, 512x512 8-bit) |
| RLE Lossless | 1.2.840.10008.1.2.5 | **verified** (MR_small_RLE, emri_small_RLE) |
| JPEG Lossless, Process 14 SV1 | 1.2.840.10008.1.2.4.70 | **verified** (JPEG-LL, 256x1024 16-bit) |
| JPEG Baseline, 8-bit | 1.2.840.10008.1.2.4.50 | **verified** (SC_rgb_jpeg_dcmtk, colour) |
| JPEG Extended, 12-bit | 1.2.840.10008.1.2.4.51 | **fails**: the pure-Rust JPEG decoder rejects 12-bit sample precision (JPEG-lossy, JPGExtended) |
| JPEG Lossless, Process 14 | 1.2.840.10008.1.2.4.57 | expected |

Verified means the file was loaded through the same code path the browser uses.
The host build of the demo is a checker for exactly this:

```sh
samples/fetch-dicom.sh                       # downloads the pydicom corpus files (not committed)
cargo run -p dicomscope -- samples/dicom/*.dcm   # one line per file
cargo run -p dicomscope -- samples/dicom         # a folder (or a zip): series summary
DICOMSCOPE_TAGS=1 cargo run -p dicomscope -- samples/dicom/CT_small.dcm   # every tag
```

The host binary can also write the HL7 order for a study:

```sh
cargo run -p dicomscope -- order study.zip -o order.hl7
cargo run -p dicomscope -- order study.zip --patient-id 9ZZ9 -o mismatch.hl7   # the headline case
```

And it can rewrite a study as a clean archive:

```sh
cargo run --release -p dicomscope -- pack exported-folder -o study.zip
cargo run --release -p dicomscope -- pack study.zip -o small.zip --series 1,5 --every 3
```

`pack` scans the input like the browser does, then writes a deflated zip
holding only the image instances that passed the scan, named
`series-<number>/<file>` in slice order. OS metadata, DICOMDIR and stray files
are left out. `--series` keeps the listed series numbers and `--every K`
keeps every K-th slice, which is how a 500 MB export becomes a demo-sized
study without touching any pixel. Pixel data is copied unchanged.

The `order` subcommand reads one instance header (streamed out of a zip, so archive size does not
matter), builds an ORM^O01 with the `hl7kit` builder from Patient ID, name,
sex, birth date, study date and time, description, modality, Accession
Number, Requested Procedure ID and Study Instance UID, then parses the message
back, extracts the order fields and resolves the linkage. It refuses to write
a message that does not link. Blank attributes stay blank; use `--accession`
and `--procedure-id` to fill what an anonymiser removed.

Also found in the corpus: pixel data with Bits Allocated 32 (RT dose, `liver`)
is rejected by dicom-pixeldata (`must be 1, 8 or 16`), and a data set with no
Part 10 header is read as a raw little-endian data set when its first element
header looks like one.

Not supported, with reasons:

- **JPEG 2000 and HTJ2K** (`.4.90`, `.4.91`, `.4.201`–`.4.203`): the only
  pure-Rust decoder, the `openjp2` crate (0.6.1), declares
  `crate-type = ["cdylib", "staticlib", "rlib"]`, so Cargo links it as a
  standalone cdylib and the link fails on an undefined `free` under
  `wasm32-unknown-unknown`; shims in the consuming crate do not help because
  the failing step happens inside `openjp2`. A fork restricting `crate-type` to
  `rlib` would fix it.
- **JPEG-LS** (`.4.80`, `.4.81`): no pure-Rust decoder is wired into
  dicom-pixeldata.
- **JPEG XL** (`.4.110`–`.4.112`): the decoder exists but is not enabled in this
  build.
- **MPEG / H.264** video: out of scope.
- **Colour** is rendered as stored: RGB, YBR_FULL and YBR_FULL_422 (converted
  to RGB), and PALETTE COLOR (non-segmented lookup tables, 8- or 16-bit
  entries). Planar configuration 1 and segmented palettes are reported, not
  rendered. Windowing does not apply to colour images.
- Multi-frame files are scrolled frame by frame like a series; only the shown
  frame is decoded.

`TransferSyntax::is_fully_supported()` in dicom-rs returns `false` for RLE and
`.70` even though decoding works, because it also covers encoding. The app
never consults it; it attempts the decode and reports the real error.

## Release bundle size

Measured with `trunk build --release` (opt-level `z`, LTO, `wasm-opt -Oz`):

| File | Raw | Gzipped |
| --- | --- | --- |
| `wasm-bindgen loader` | 72 KB | 12 KB |
| `wasm` | 1520 KB | 640 KB |

Total over the wire: **652 KB gzipped** (Rust 1.96, wgpu 30.0.1, leptos 0.8.20,
dicom-rs 0.10.0, zip 7.2, serde_json 1, 2026-09-09). The first cut of the
viewer alone was 476 KB; series scanning, zip, colour, measurements,
documents and the FHIR output added the rest.

## The no-network guarantee, and how to verify it

Nothing is fetched after the page loads. Files enter through
`File.arrayBuffer()` from a local file input or drop event and never leave the
tab.

- `grep -rn 'fetch\|XMLHttpRequest\|WebSocket' src/` finds nothing.
- Zip archives are unpacked in memory by a pure-Rust crate; no browser or
  server involvement.
- The `web-sys` feature list in `Cargo.toml` enables only file, drag, mouse,
  wheel, keyboard and canvas types plus `Window` (for the device pixel ratio).
  No `Request`, `Response`, `Fetch`, `WebSocket`.
- `index.html` has no external stylesheet, font, or script; the only `<script>`
  is the loader Trunk injects for the wasm-bindgen output.
- Open the browser's network panel, load the page, then load files: no requests
  after the initial document, JS, and wasm.

CI repeats the grep on every push.

## Rendering notes

- Frame values are stored as `f32` after Rescale Slope/Intercept and uploaded
  to an `R32Float` texture read with `textureLoad`. No optional WebGPU feature
  is requested; `float32-filterable` is unnecessary because nothing is
  filtered.
- `MONOCHROME1` sets an `invert` flag in the uniform; the buffer is never
  pre-inverted.
- Windowing uses the linear VOI LUT function from PS3.3 C.11.2.1.2.1.
- The view transform (scale and offset) is applied in the fragment shader from
  the framebuffer position. When magnified with interpolation on, the shader
  does bilinear filtering by hand over four `textureLoad`s, since `R32Float`
  is not filterable without an optional feature.
- The canvas backing store follows its CSS box times the device pixel ratio
  and is resized with the window.
- Texture rows are padded to 256 bytes on upload.
- If no WebGPU adapter is available the canvas is replaced by a message naming
  the requirement. A blank black rectangle is not an error report.

## Build and run

```sh
cd demo/dicomscope
trunk serve            # http://127.0.0.1:8080
trunk build --release  # writes dist/
```

Host-side tests for the DICOM and linkage modules run without a browser:

```sh
cargo test -p dicomscope
```

Toolchain: Rust 1.96, `wasm32-unknown-unknown`, Trunk 0.21.14. If your
`wasm-opt` is older than version 119 it needs the feature flags that
`index.html` passes via `data-wasm-opt-params`; newer versions ignore them.

## Non-goals for v1

MPR and 3D, DICOMDIR navigation (the index is skipped, the files are scanned),
C-FIND/C-STORE/DICOMweb,
JPEG 2000 and JPEG-LS, HL7 v2 conformance validation against segment tables,
FHIR, measurement and annotation, persistence and accounts, WebGL2 fallback.
