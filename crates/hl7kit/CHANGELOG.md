# Changelog

## 0.2.0 (unreleased)

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
- `Warning::ConflictingStudyUid`: several `IPC` segments with different
  `IPC-3.1` values. The first is used; the warning is recorded on `Order`.
- Fixture `tests/fixtures/order-omi.hl7`, an OMI^O23 for the same study as
  the ORM^O01 fixture.

## 0.1.0

Initial release: span-preserving parser, path queries, escape handling,
tolerance warnings, IHE order extraction, structural builder.
