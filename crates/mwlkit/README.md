# mwlkit

[![crates.io](https://img.shields.io/crates/v/mwlkit.svg)](https://crates.io/crates/mwlkit)
[![docs.rs](https://docs.rs/mwlkit/badge.svg)](https://docs.rs/mwlkit)
[![CI](https://github.com/holg/hl7v2/actions/workflows/ci.yml/badge.svg)](https://github.com/holg/hl7v2/actions/workflows/ci.yml)

Turn an HL7 v2 imaging order into a DICOM Modality Worklist item, with
every mapping choice sourced and every invented value reported.

Built on [`hl7kit`](https://crates.io/crates/hl7kit) for the message and
[`dicom-object`](https://crates.io/crates/dicom-object) for the data set.
Compiles for `wasm32-unknown-unknown`; the
[dicomscope demo](https://holg.github.io/hl7v2/) builds the item in the
browser and lets you download it.

## What a worklist item is

Before an examination the modality asks the Modality Worklist SCP, with a
C-FIND, what is scheduled for it. Each answer is one data set in the
Modality Worklist Information Model (DICOM PS3.4 Annex K, Table K.6-1):
patient identification, the imaging service request (order numbers,
accession number), the requested procedure (procedure ID, Study Instance
UID) and one or more Scheduled Procedure Steps (modality, station, start
time). The modality copies Patient ID, Accession Number, Study Instance
UID, Requested Procedure ID and SPS ID verbatim into every image it
produces. That data set is the hinge of RIS/PACS linkage: what goes wrong
here is what the archive and the reporting system fight over later.

The mapping follows IHE Radiology Technical Framework Volume 2, transaction
RAD-4 "Procedure Scheduled", with dcm4che's HL7-to-MWL behaviour noted
where it differs. Every line in `src/map.rs` says which document it comes
from, or says `unsourced` and why.

```rust
use hl7kit::{order::Order, Message};
use mwlkit::{to_bytes, worklist_item, Input, Options};

let msg = Message::parse(text)?;
let order = Order::extract(&msg);
let item = worklist_item(&Input { message: &msg, order: &order }, &Options::default())?;
for w in &item.warnings {
    eprintln!("{w}");
}
std::fs::write("order.mwl.dcm", to_bytes(&item)?)?;
```

## The mapping

Types are those of Table K.6-1. Sources: IHE RAD-4 unless marked.

| DICOM attribute | Type | From |
|---|---|---|
| (0008,0005) Specific Character Set | 1C | MSH-18; `ASCII` omits it, unknown or absent becomes `ISO_IR 192` with a warning |
| (0010,0020) Patient ID | 1 | PID-3.1; refused when absent |
| (0010,0021) Issuer of Patient ID | 3 | PID-3.4 |
| (0010,0010) Patient's Name | 2 | PID-5, XPN reordered to PN (see below) |
| (0010,0030) Patient's Birth Date | 2 | PID-7, date part |
| (0010,0040) Patient's Sex | 2 | PID-8; M/F/O pass, U is empty, anything else empty with a warning |
| (0008,0050) Accession Number | 2 | OBR-18, else IPC-1.1 (`hl7kit::order`) |
| (0040,2016) Placer Order Number | 3 | ORC-2.1 |
| (0040,2017) Filler Order Number | 3 | ORC-3.1 |
| (0008,0090) Referring Physician's Name | 2 | PV1-8, else ORC-12 (dcm4che) |
| (0032,1032) Requesting Physician | 3 | OBR-16, else ORC-12 |
| (0040,1001) Requested Procedure ID | 1 | OBR-19, else IPC-2.1; else the placer order number, reported |
| (0020,000D) Study Instance UID | 1 | IPC-3.1, ZDS-1.1 or OBX DCM 110180; else the UID policy |
| (0032,1060) Requested Procedure Description | 1C | OBR-4.2 |
| (0032,1064) Requested Procedure Code Sequence | 1C | OBR-4.1/4.2/4.3, only when 4.3 names a coding scheme |
| (0040,1003) Requested Procedure Priority | 3 | OBR-27.6, else TQ1-9: S→STAT, A→HIGH, R→ROUTINE; T→HIGH and P→MEDIUM are unsourced |
| (0008,1110) Referenced Study Sequence | 2 | present, empty |
| (0040,0100) SPS Sequence | 1 | one item per IPC segment; without IPC, one item from ORC/OBR/TQ1 |
| (0008,0060) Modality | 1 | IPC-5, else OBR-24; HL7 table 0074 codes with one DICOM meaning are translated (MRI→MR, NMS→NM, RUS→US, XRC→XA, OTH→OT) and reported, anything else refused |
| (0040,0001) Scheduled Station AE Title | 1 | IPC-9, else `Options::default_station_ae`, reported; refused without either |
| (0040,0002/0003) SPS Start Date and Time | 1 | OBR-27.4, else TQ1-7; refused without either |
| (0040,0006) Scheduled Performing Physician's Name | 2 | OBR-34 |
| (0040,0007) SPS Description | 1C | OBR-4.2 |
| (0040,0008) Scheduled Protocol Code Sequence | 1C | IPC-6, only with a coding scheme |
| (0040,0009) SPS ID | 1 | IPC-4.1, else OBR-20; else `<procedure id>-<n>`, reported (unsourced) |
| (0040,0010) Scheduled Station Name | 2 | IPC-7 |
| (0040,0011) SPS Location | 2 | IPC-8 |
| (0040,0020) SPS Status | 3 | `SCHEDULED` when ORC-1 is `NW` (unsourced) |
| (0038,0010) Admission ID | 2 | PV1-19.1 |
| (0038,0300) Current Patient Location | 2 | PV1-3 |

Not mapped in this version: Institution Name (MSH-4 is a facility code,
not a name), Patient's Weight, Medical Alerts and Allergies, NTE comments.

### Person names

HL7 XPN is `family^given^middle^suffix^prefix`; DICOM PN is
`family^given^middle^prefix^suffix`. The two outer components swap. XCN
(ORC-12, OBR-16, PV1-8) has the person ID in front and is handled too.
When a prefix or suffix is present the reorder is reported as
`Warning::NameComponentsReordered`, because a reviewer comparing the two
sides will otherwise see a difference that is not an error.

## Study Instance UID policy

The only value this crate ever invents. When IPC-3, ZDS-1 and the DCM
110180 OBX are all absent, `Options::uid_policy` decides:

* `Dcm4cheStyle` (default): a deterministic name-based UID from the
  Requested Procedure ID, else from the Accession Number, else an error.
  This mirrors dcm4che, which derives a name-based UID from those
  identifiers so the same order always yields the same UID. It is
  dcm4che-style, not dcm4che-identical: dcm4che uses UUID version 5, this
  crate a version 8 hash, both under the `2.25.` UUID-derived root of
  PS3.5 B.2, which needs no registered root and fits the UI limit.
* `Random(u128)`: the caller's random bits as a `2.25.` UID. The crate has
  no random source and does not pretend to.
* `Refuse`: fail with `MwlError::NoStudyUid`.

A generated UID is always reported as `Warning::StudyUidGenerated` and
`WorklistItem::study_uid.origin` says `Generated`. The consequence is
visible in the demo's chain view: the modality copies this UID into the
images, and the RIS never learns it.

## Length policy

DICOM VR limits: SH 16, LO 64, PN 64, AE 16, CS 16, UI 64 characters.
Real RIS accession numbers are often longer than 16. `Options::length_policy`:

* `Refuse` (default): `MwlError::TooLong` naming the attribute. A cut
  accession number is a different key from the one the RIS holds.
* `TruncateAndWarn`: cut and report `Warning::Truncated`, to see what a
  lenient interface would have produced.

AE and UI are never truncated under either policy: a cut AE title is a
wrong destination and a cut UID is a different study.

## Writing to a file

A worklist item is a C-FIND response, not a stored object, so the standard
defines no file format for it. `to_bytes` writes a Part 10 file anyway,
because that is what file-based worklist SCPs (dcm4che `wlmscpfs`,
Orthanc's worklist plugin) read: Media Storage SOP Class
`1.2.840.10008.5.1.4.31` (Modality Worklist Information Model - FIND), a
deterministic `2.25.` instance UID, Explicit VR Little Endian.

## Non-goals

No MLLP, no C-FIND, no network code of any kind. No guessing: a modality
is never inferred from procedure text, a coding scheme is never invented,
a Type 1 attribute is refused rather than filled with a placeholder.

## License

MIT OR Apache-2.0, like the rest of the workspace.
