# Changelog

## 0.2.0 (2026-09-09)

### Breaking

- `OrderField::path()` is deprecated in favour of `OrderField::paths()`,
  which returns the candidate query paths in precedence order. The study
  instance UID is now read from `IPC-3.1` first and `ZDS-1.1` second, so
  `path()` for `StudyUid` returns `"IPC-3.1"`, not `"ZDS-1.1"`.
- `Order` gained `study_uid_source`, `warnings` and `sources`. Code that
  constructs `Order` by hand must use `..Order::default()`.

### Added

- OMI^O23 support: the `IPC` segment (HL7 2.5.1 imaging order messages) is
  read for the study instance UID (`IPC-3.1`), and as a fallback for the
  accession number (`IPC-1.1`) and requested procedure ID (`IPC-2.1`) when
  `OBR-18` and `OBR-19` are blank.
- `StudyUidSource` says which segment supplied the UID.
- `Order::source_path()` gives the path that supplied any field.
- ORU^R01 image-availability support: the study UID is read from an `OBX`
  whose `OBX-3` is DCM `110180` (Study Instance UID) or names it, after
  `IPC-3.1` and `ZDS-1.1`. `StudyUidSource::Obx` reports it.
- `Warning::ConflictingStudyUid`: the study UID sources disagree (several
  `IPC` segments, or `IPC`/`ZDS` against an `OBX`). The first in precedence
  is used; the warning is recorded on `Order`.
- Fixtures `tests/fixtures/order-omi.hl7` (OMI^O23) and
  `tests/fixtures/order-oru.hl7` (ORU^R01) for the same study as the
  ORM^O01 fixture.

## 0.1.0

Initial release: span-preserving parser, path queries, escape handling,
tolerance warnings, IHE order extraction, structural builder.
