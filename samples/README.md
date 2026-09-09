# Samples

## HL7 orders

- `order.hl7` — synthetic ORM^O01 whose PID-3.1 (`4MR1`) and ZDS-1.1
  (`1.3.6.1.4.1.5962.1.2.4.20040826185059.5457`) are the Patient ID and Study
  Instance UID of pydicom's `MR_small.dcm`. Loading both should link by study
  UID with matching patients.
- `order-mismatch.hl7` — same study UID, different patient (`9ZZ9`). Loading it
  with `MR_small.dcm` shows the headline case: linked study, patient mismatch.
- `order-omi.hl7` — the same order as an OMI^O23 (HL7 2.5.1 imaging order):
  no `ZDS`, the study UID is in `IPC-3`, accession in `IPC-1` and requested
  procedure ID in `IPC-2`. Links to `MR_small.dcm` by study UID; the link
  panel shows `IPC-3.1` as the source.
- `order-oru.hl7` — an ORU^R01 image-availability notification for the same
  study, as a PACS sends it back to the RIS: no `ZDS`, no `IPC`, the study
  UID sits in an `OBX` whose `OBX-3` is `110180^Study Instance UID^DCM`.
  The other `OBX` segments (series and instance counts) are ignored. Links
  by study UID from the `OBX`, accession from `OBR-18`.

Both use the standard `\r` segment terminator. The library's tests use a
copy of `order.hl7` at `crates/hl7kit/tests/fixtures/order.hl7`, so the
published crate carries its own fixture; a demo test fails if the two files
ever differ.

- `anonymized-demo.hl7` — generated from `anonymized-demo.zip` (an anonymised
  five-series CT study, 970 files, not committed) with the `hl7kit` builder:

  ```sh
  cargo run -p dicomscope -- order samples/anonymized-demo.zip --control-id MSG0003 -o samples/anonymized-demo.hl7
  ```

  The tool reads one instance header out of the archive (streaming, so the
  516 MB zip is not loaded), builds the ORM^O01, parses it back, extracts the
  order fields and resolves the linkage before writing. The anonymiser
  blanked Accession Number and Requested Procedure ID, so the message carries
  them blank and the study links by Study Instance UID alone; that is the
  realistic shape of an anonymised export.
- `anonymized-demo-mismatch.hl7` — the same with `--patient-id 9ZZ9`: linked by
  study UID, patient identifiers disagree. Load it with the zip to see the
  headline warning on a real study.

The archive itself came as a stored (uncompressed) zip with macOS resource
forks inside. `anonymized-demo-clean.zip` is the same study rewritten by

```sh
cargo run --release -p dicomscope -- pack "anonymized-demo 2.zip" -o anonymized-demo-clean.zip
```

which deflates it to 247 MB and drops the 976 `__MACOSX` entries. Neither zip
is committed. To make a small demo study, add `--series 1,3 --every 4`.

If the identifiers in your copy of `MR_small.dcm` differ, the tag list in the
demo shows the real values; edit the sample to match.

## DICOM instances

No DICOM files are committed; `samples/dicom/` is git-ignored. Run
`samples/fetch-dicom.sh` to download the pydicom corpus files the demo was
verified against, then check them all with the host build:

```sh
samples/fetch-dicom.sh
cargo run -p dicomscope -- samples/dicom/*.dcm
```

Results on 2026-09-05 (dicom-rs 0.10): 11 files decode (Explicit and Implicit
VR LE, Explicit VR BE, Deflated, RLE, JPEG Lossless .70, multi-frame with frame
0 shown), and every other file fails with a message naming why (JPEG 2000,
JPEG-LS, 12-bit JPEG Extended, colour, Bits Allocated 32, truncated file).

Sources: <https://github.com/pydicom/pydicom/tree/main/src/pydicom/data/test_files>
and <https://github.com/pydicom/pydicom-data>.

These files contain no real patient data. Do not commit real studies to this
repository, even anonymised ones.
