# Coding prompt — FHIR output for dicomscope, IPC support for hl7kit

Extend the existing workspace at `github.com/holg/hl7v2` so that the demo turns
its two-way HL7 v2 ↔ DICOM linkage into a three-way linkage with FHIR R4, and so
that `hl7kit` resolves the Study Instance UID from `IPC-3` as well as `ZDS-1`.

This prompt was written against the repository as of 2026-09-08. Read the
current code first; the structure below is what was found and what the changes
build on.

---

## 0. Non-negotiables (unchanged from the project)

1. **Pure Rust.** No hand-written JavaScript. The only `.js` is what
   `wasm-bindgen` emits. No npm.
2. **No network I/O after page load.** This extension must not add a FHIR
   validator call, a terminology server lookup, a profile package download, or
   a `$validate` round-trip. Everything is generated locally. The README's
   no-network guarantee must remain true and auditable after this change.
3. **No `unwrap()`/`expect()` on user-derived data.** Every absent field
   produces an absent FHIR element, never a panic and never a placeholder that
   looks like data.
4. **No typed FHIR crate.** Three resources do not justify pulling in
   `fhir-sdk`, `fhir-model`, or similar; they are large, some do not target
   wasm32 cleanly, and the bundle size is a published number in the README.
   Build the JSON with `serde_json::json!` / `serde_json::Value` and a thin
   layer of helper functions. Add `serde = { features = ["derive"] }` and
   `serde_json` to `demo/dicomscope`; do not add them to `hl7kit`.

---

## 1. Current state (verified)

Workspace: `crates/hl7kit` (the library, default member) and `demo/dicomscope`
(Leptos CSR + wgpu + dicom-rs 0.10, Trunk). Rust edition 2021,
`rust-version = "1.85"`. Release profile `opt-level = "z"`, LTO, strip,
`panic = "abort"`.

`hl7kit` public surface relevant here:

```rust
pub enum OrderField { PatientId, Accession, ProcedureId, StudyUid }
impl OrderField {
    pub const ALL: [OrderField; 4];
    pub fn path(self) -> &'static str;   // "PID-3.1" | "OBR-18" | "OBR-19" | "ZDS-1.1"
    pub fn label(self) -> &'static str;
}
pub struct Order {
    pub patient_id: Option<String>,
    pub accession: Option<String>,
    pub procedure_id: Option<String>,
    pub study_uid: Option<String>,
    pub spans: Vec<(Span, OrderField)>,
}
impl Order { pub fn extract(msg: &Message) -> Order; pub fn get(&self, OrderField) -> Option<&str>; }
```

`Message` supports `get_span(path)`, `decode(&str)`, `raw()`. Paths are of the
form `SEG-field.component`.

`demo/dicomscope` relevant types:

```rust
// src/dicom/study.rs
pub struct Study {
    pub patient_id: Option<String>,             // (0010,0020)
    pub accession_number: Option<String>,       // (0008,0050)
    pub study_uid: Option<String>,              // (0020,000D)
    pub requested_procedure_id: Option<String>, // (0040,1001)
    pub transfer_syntax: String,
    pub modality: Option<String>,               // (0008,0060)
    pub rows: u32, pub cols: u32,
}
pub fn string(obj: &InMemDicomObject, tag: Tag) -> Option<String>;

// src/link/mod.rs
pub enum LinkPath { StudyUid, Accession, None }
pub struct Pair { pub dicom: Option<String>, /* hl7 side */ }
pub struct Linkage {
    pub path: LinkPath,
    pub patient_match: Option<bool>,
    pub study_uid: Pair, pub accession: Pair, pub patient_id: Pair, pub procedure_id: Pair,
}
pub fn resolve(study: &Study, order: &Order) -> Linkage;
```

Series grouping already exists (`src/dicom/series.rs`): files are grouped by
Series Instance UID and sorted by position. **Use it** — `numberOfSeries`,
`numberOfInstances`, and the `series[].instance[]` arrays come from that
grouping, not from re-scanning.

**Gap found:** `demo/dicomscope/README.md` states that OMI^O23 is supported, but
`OrderField::StudyUid` reads only `ZDS-1.1`. There is no `IPC` anywhere in the
codebase. In OMI^O23 the study UID is carried in `IPC-3`, and `ZDS` is not
present. The README claim is currently false for the study-UID link path. Part A
fixes this before Part B relies on it.

---

## Part A — `hl7kit`: resolve the study UID from `IPC-3` and `ZDS-1`

### Background

`IPC` (Imaging Procedure Control) is a standard HL7 v2 segment introduced with
the imaging order messages (OMI^O23, HL7 2.5.1+). `IPC-1` is the accession
identifier, `IPC-2` the requested procedure ID, `IPC-3` the Study Instance UID,
`IPC-4` the scheduled procedure step ID. `ZDS` is a vendor Z-segment from the
IHE Radiology Technical Framework used with ORM^O01 on older interfaces; `ZDS-1`
carries the study UID as `uid^^Application^DICOM`, first component only.

When an order has several procedure steps, several `IPC` segments follow one
`ORC/OBR` pair, and `IPC-3` must be identical across them. dcm4che documents
that when neither is present, the archive derives a UID from OBR-19 or the
accession number, or generates a random one — which is the reason the demo's
fallback path exists and must stay visible.

### API change (breaking; the crate is 0.x — bump the minor version)

Replace the single-path lookup for `StudyUid` with an ordered list of candidate
paths, and record which one matched.

```rust
impl OrderField {
    /// Candidate paths in precedence order. First present, non-empty value wins.
    pub fn paths(self) -> &'static [&'static str] {
        match self {
            OrderField::PatientId   => &["PID-3.1"],
            OrderField::Accession   => &["OBR-18"],
            OrderField::ProcedureId => &["OBR-19"],
            OrderField::StudyUid    => &["IPC-3.1", "ZDS-1.1"],
        }
    }
    // Keep `path()` as a deprecated shim returning `paths()[0]`, or remove it;
    // either is fine, but do not leave it returning "ZDS-1.1" for StudyUid.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StudyUidSource { Ipc3, Zds1 }

pub struct Order {
    // existing fields …
    /// Which segment supplied `study_uid`, when present.
    pub study_uid_source: Option<StudyUidSource>,
}
```

`Order::extract` iterates `paths()` per field and stops at the first non-empty
decoded value. For `StudyUid`, set `study_uid_source` accordingly and push the
span of the path that matched. Keep the existing comment about OBR-3 as a
possible gated fallback; do not implement it.

Multi-IPC rule: if the message contains more than one `IPC` segment and their
`IPC-3.1` values differ, do not silently take the first. Add a
`Warning::ConflictingStudyUid { first: Span, other: Span }` variant to the
existing `Warning` enum in `message.rs` (or the equivalent mechanism the crate
already uses for non-fatal findings), take the first, and surface the warning.
The demo shows it in the link panel.

### Tests (host target, `crates/hl7kit/tests/`)

- OMI^O23 with one IPC: `study_uid` from IPC-3.1, source `Ipc3`, span points at
  the IPC segment.
- ORM^O01 with ZDS: source `Zds1`, unchanged behaviour.
- Message with both (real interfaces do produce this): IPC wins.
- Two IPC segments with identical IPC-3.1: no warning.
- Two IPC segments with different IPC-3.1: warning emitted, first value taken.
- `IPC-3` present but empty, `ZDS-1` present: falls through to ZDS, source `Zds1`.
- `IPC-3.1` is `uid` only; `IPC-3` with further components (`uid^^App^DICOM`
  shape does not apply to IPC, but a stray component must not leak into the
  value).

Add one OMI^O23 sample to `samples/` that links to an existing DICOM sample,
alongside the existing ORM^O01.

---

## Part B — `dicomscope`: generate a FHIR R4 Bundle from the loaded pair

### Target

After both a study and an order are loaded, the app produces one FHIR R4
(4.0.1) `Bundle` of `type: "collection"` containing three resources: `Patient`,
`ServiceRequest`, `ImagingStudy`. The `ImagingStudy` declares the MII
Bildgebung profile. The link panel gains a third column showing, for each
identifier, the FHIR element it lands in. A new panel shows the JSON with a
copy button.

Version target is R4 because the MII Kerndatensatz modules are R4. Do not
target R5.

### Why this matters (for the README and for your own orientation)

The MII (Medizininformatik-Initiative) Modul Bildgebung defines an
`ImagingStudy` profile that maps DICOM metadata at study, series, and instance
level into FHIR for the Datenintegrationszentren of the German university
hospitals. The canonical URL is

```
https://www.medizininformatik-initiative.de/fhir/ext/modul-bildgebung/StructureDefinition/mii-pr-bildgebung-bildgebungsstudie
```

(current published version 2025.0.0-ballot; put the version you actually
looked at in the README). HL7 International publishes the "HL7 Version 2 to
FHIR" implementation guide with segment-level mapping tables (PID → Patient,
ORC/OBR → ServiceRequest). Use both as the source of truth for element choices;
do not invent mappings where those documents have one, and say so in the code
comments where they do not.

### New module layout

```
demo/dicomscope/src/fhir/
  mod.rs          pub fn bundle(study, series, order, linkage) -> serde_json::Value
  patient.rs      Patient from DICOM patient module + PID
  service_request.rs   ServiceRequest from ORC/OBR/IPC or ZDS
  imaging_study.rs     ImagingStudy from Study + series grouping
  datetime.rs     DICOM DA/TM/DT → FHIR date / dateTime, with the timezone rule
  ids.rs          identifier constructors and the system constants
demo/dicomscope/src/ui/fhir_panel.rs
```

Keep `fhir/` free of Leptos and `web-sys` so it is testable on the host target
like `link/` already is.

### Resource ids and references

`Bundle.type = "collection"`. Do not add a `uuid` dependency for `fullUrl`. Use
stable ids and relative references:

```
Patient.id         = "patient-1"        → "Patient/patient-1"
ServiceRequest.id  = "servicerequest-1" → "ServiceRequest/servicerequest-1"
ImagingStudy.id    = "imagingstudy-1"
```

`entry[].fullUrl` may be omitted for a collection bundle; if you include it, use
`"urn:uuid:"` only if you generate real UUIDs, otherwise omit it. Relative
references inside a collection bundle are acceptable and keep the output
readable.

### Identifier systems — exact strings

These are fixed by FHIR and the v2-to-FHIR guide. Put them in `ids.rs` as
`const`s with a doc comment citing where each comes from.

| Thing | `identifier.system` | `identifier.value` | `identifier.type.coding` |
| --- | --- | --- | --- |
| Study Instance UID | `urn:dicom:uid` | `urn:oid:<uid>` | — |
| Accession Number | site-specific; see below | the accession string | `http://terminology.hl7.org/CodeSystem/v2-0203` code `ACSN` |
| Patient identifier (PID-3 / (0010,0020)) | from PID-3.4 when present; see below | the identifier | `v2-0203` code `MR` |
| Placer order number ORC-2 | site-specific | ORC-2.1 | `v2-0203` code `PLAC` |
| Filler order number ORC-3 | site-specific | ORC-3.1 | `v2-0203` code `FILL` |
| SOP Class UID (instance.sopClass) | `urn:ietf:rfc:3986` | `urn:oid:<sop class uid>` | — (it is a `Coding`, `system`/`code`) |

"Site-specific" means: there is no universal system URI for a hospital's
accession or MRN namespace. Rules:

- If `PID-3.4` (assigning authority) is present, use its first component as a
  bare string in `system` **only** if it is a valid URI; otherwise place it in
  `identifier.assigner.display` and leave `system` absent. Do not fabricate a
  URI.
- Otherwise leave `system` absent and set `identifier.type` only. The link
  panel notes "no assigning authority in message" for that row.
- Never emit `"system": ""` or a placeholder like `"urn:example"`.

### Patient

Source precedence: DICOM patient module first (the imaged patient is the
subject of the study), PID second, per element. Record which was used in a
`meta.source`-free way — do not abuse `meta.source`; instead the link panel
already compares `patient_id` and shows mismatches, which is where source
disagreement belongs.

| FHIR element | DICOM | HL7 |
| --- | --- | --- |
| `identifier[MR]` | (0010,0020) | PID-3.1, assigner from PID-3.4 |
| `name[0].family` / `given[]` | (0010,0010) PN, `Family^Given^Middle` | PID-5.1 / PID-5.2 |
| `birthDate` | (0010,0030) DA | PID-7 (DTM, date part) |
| `gender` | (0010,0040) | PID-8 |

Gender mapping: `M → male`, `F → female`, `O → other`, `U`/absent → omit the
element entirely (do not emit `unknown` unless the source explicitly says `U`;
absent is absent).

PN parsing: split on `^`; component 1 family, 2 given, 3 middle (append to
`given`), 4 prefix, 5 suffix. DICOM PN may have `=` separated alphabetic /
ideographic / phonetic groups — take the first group only, and note this in a
comment.

### ServiceRequest

| FHIR element | Source |
| --- | --- |
| `status` | `"active"` for ORM^O01 NW / OMI^O23; map ORC-1 if present: `NW`→`active`, `CA`→`revoked`, `DC`→`revoked`, `XO`→`active`, else `active` with a comment |
| `intent` | `"order"` |
| `identifier[PLAC]` | ORC-2 |
| `identifier[FILL]` | ORC-3 |
| `identifier[ACSN]` | OBR-18 (or IPC-1 for OMI when OBR-18 absent) |
| `identifier[]` with `system: urn:dicom:uid` | study UID from `Order.study_uid` (IPC-3 or ZDS-1). **This is the row VOLCANO-style HL7→FHIR mappers commonly miss**; it is the reason this feature exists. |
| `code.coding` | OBR-4: `system` from OBR-4.3 if it is a URI-shaped string, else omit `system` and keep `code`/`display`; also set `code.text` from OBR-4.2 |
| `subject` | `Patient/patient-1` |
| `authoredOn` | ORC-9 → dateTime via `datetime.rs` |
| `requester.display` | ORC-12 / OBR-16 formatted `Family, Given` when present |

Requested Procedure ID (OBR-19 / (0040,1001)): there is no standard
`v2-0203` type code for it. Emit it as an additional `identifier` with no
`type` and no `system`, and mark the row in the link panel as "carried, not
standardised". Do not invent a type code.

### ImagingStudy

```json
"meta": { "profile": ["<MII canonical URL>"] }
```

| FHIR element | Source |
| --- | --- |
| `identifier[]` | `urn:dicom:uid` / `urn:oid:` + (0020,000D); plus `ACSN` from (0008,0050) |
| `status` | `"available"` |
| `subject` | `Patient/patient-1` |
| `basedOn[]` | `ServiceRequest/servicerequest-1` — **only when `Linkage.path != None`**. If the study did not link to the order, do not assert `basedOn`; that would encode a false relationship. |
| `started` | (0008,0020) + (0008,0030) via the timezone rule |
| `modality[]` | distinct (0008,0060) across series, `system: http://dicom.nema.org/resources/ontology/DCM` |
| `numberOfSeries` / `numberOfInstances` | from the existing series grouping |
| `description` | (0008,1030) |
| `series[]` | one per grouped series |
| `series[].uid` | Series Instance UID |
| `series[].number` | (0020,0011) |
| `series[].modality` | (0008,0060) as above |
| `series[].description` | (0008,103E) |
| `series[].numberOfInstances` | count |
| `series[].bodySite` | omit in this iteration |
| `series[].instance[]` | one per instance |
| `instance.uid` | SOP Instance UID (0008,0018) |
| `instance.sopClass` | `Coding { system: urn:ietf:rfc:3986, code: urn:oid:<(0008,0016)> }` |
| `instance.number` | (0020,0013) |

MII-specific extensions (modality-specific series extensions, Bildgebungsgrund,
etc.) are **out of scope**. The output declares the profile and populates the
core elements the profile constrains; it does not claim conformance. The README
says exactly that.

### `datetime.rs` — the timezone rule

FHIR `dateTime`: if hours and minutes are present, a timezone **must** be
present. DICOM DA/TM carry no zone. Therefore:

- If (0008,0201) Timezone Offset From UTC is present and well-formed
  (`±HHMM`), emit full `YYYY-MM-DDThh:mm:ss±hh:mm`.
- Otherwise emit **date only** `YYYY-MM-DD`, and drop the time. Losing the time
  is correct; emitting a naive time is invalid FHIR.
- Fractional seconds: keep up to what FHIR allows only if you also have a zone;
  otherwise irrelevant.
- HL7 DTM (`YYYYMMDDHHMMSS[.S+][±ZZZZ]`): same rule, zone from the trailing
  offset if present.
- Malformed input → `None`, element omitted. Never emit a partial string.

Unit-test every branch.

### Link panel — third column

Extend `Linkage` presentation (not the `resolve` logic) so each row shows:

| Identifier | HL7 path | DICOM tag | FHIR element |
| --- | --- | --- | --- |
| Study Instance UID | `IPC-3.1` or `ZDS-1.1` (whichever matched — read `study_uid_source`) | (0020,000D) | `ImagingStudy.identifier[urn:dicom:uid]` and `ServiceRequest.identifier[urn:dicom:uid]` |
| Accession Number | `OBR-18` | (0008,0050) | `ImagingStudy.identifier[ACSN]`, `ServiceRequest.identifier[ACSN]` |
| Patient ID | `PID-3.1` | (0010,0020) | `Patient.identifier[MR]` |
| Requested Procedure ID | `OBR-19` | (0040,1001) | `ServiceRequest.identifier` (untyped) — "carried, not standardised" |

Show the `ConflictingStudyUid` warning from Part A here in the warning role.
Show "basedOn omitted — study did not link" in the ImagingStudy row when
`LinkPath::None`.

### FHIR panel (`ui/fhir_panel.rs`)

- Three tabs or a segmented control: Patient / ServiceRequest / ImagingStudy,
  plus "Bundle".
- Pretty-printed JSON (`serde_json::to_string_pretty`), monospace, scrollable.
- A copy-to-clipboard button using `web_sys::Clipboard` via
  `navigator().clipboard()` and `wasm_bindgen_futures::spawn_local`. This is
  a local browser API, not network. If the clipboard API is unavailable
  (insecure context), fall back to selecting the text and say so.
- A "Download bundle.json" link built from a Blob URL (`web_sys::Blob`,
  `Url::create_object_url_with_blob`), revoked after click. Also local.
- Regenerate on any change of study, order, or linkage; it is cheap.
- Before both files are loaded: a one-line note, not an empty JSON.

### Tests (host target, `demo/dicomscope/src/fhir/` unit tests + a fixture
test)

- Fixture: build a `Study`, a two-series grouping with three instances, an
  `Order` from the ORM sample, and one from the OMI sample. Assert on
  `serde_json::Value` paths, not on string equality of the whole document.
- `basedOn` present iff linked.
- Identifier systems exactly as specified; no empty `system`.
- Gender mapping incl. absent and `U`.
- `started` date-only when (0008,0201) absent; full dateTime when present.
- PN with ideographic group → first group only.
- `study_uid_source` drives the HL7-path column label.

### README additions (`demo/dicomscope/README.md`)

- New section "FHIR output": what is generated, R4, the MII profile URL and
  the version looked at, and the sentence: *"The ImagingStudy declares the MII
  Bildgebung profile and populates its core elements; it is not validated
  against the profile, and the modality-specific extensions are not emitted."*
- The identifier-system rules in three lines, including "no system is
  fabricated when the message carries no assigning authority".
- The timezone rule in one line.
- Correct the OMI^O23 claim: it is now true, and say that IPC-3 takes
  precedence over ZDS-1.
- Updated measured gzipped bundle size after adding `serde_json`. Measure,
  do not estimate.

### `hl7kit` README / CHANGELOG

- Document `paths()`, `StudyUidSource`, and the multi-IPC warning.
- Note the breaking change from `path()` to `paths()`.

---

## Part C — non-goals for this change

Named so they are not accidentally pulled in:

profile validation · terminology lookups · MII modality extensions ·
`DiagnosticReport` / `Observation` / `Encounter` · FHIR R5 · pushing to a FHIR
server · reading an existing `ImagingStudy` as a third input (a good later
step: three-way consistency check, but not now) · OBR-3 fallback for the study
UID (stays gated and unimplemented) · any change to rendering, series
handling, SR, or measurement code.

---

## Order of work

1. Part A in `hl7kit`, with tests green on the host. Bump version.
2. `datetime.rs` and `ids.rs` with tests.
3. `patient.rs`, `service_request.rs`, `imaging_study.rs`, `bundle()`; fixture test.
4. Link panel third column and warnings.
5. FHIR panel UI.
6. READMEs, CHANGELOG, measured bundle size.
7. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` on both
   host and `wasm32-unknown-unknown`, host `cargo test`, `trunk build --release`.

## Style

Same as the rest of the repository: boring, readable, no macros beyond
`json!`, comments where a mapping choice comes from a specific document and
where it does not. This output will be read by people who maintain HL7→FHIR
mappings for a living; every element they would question should have a
one-line comment saying where it came from.
