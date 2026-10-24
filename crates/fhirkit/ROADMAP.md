# fhirkit roadmap

fhirkit is the FHIR sibling of hl7kit and mwlkit: general-purpose FHIR R4
for Rust (and Python), not tied to radiology. It uses the same rules as its
siblings: no runtime dependencies in the core crate, no `unsafe`, no panics
on input, and it compiles for `wasm32-unknown-unknown`.

Everything is generated from the official `hl7.fhir.r4.core` 4.0.1 package
by `crates/fhirkit-gen`, and the generated code is committed.

## Phase 1: models and JSON (done: core)

- [x] Generator: 146 resources, 39 datatypes, 658 structs, 186 choice enums,
      cycle-aware boxing, `contentReference` reuse.
- [x] Dependency-free JSON reader and writer. Decimals keep their exact text,
      key order is kept, duplicate keys are reported, errors carry
      line and column.
- [x] Lenient decoding that reports every problem with its path
      (`Observation.component[1].valueQuantity`):
  - primitive formats (per the R4 regexes, hand-written), required elements,
    choice conflicts, unknown elements, empty arrays and strings,
    misaligned `_element` arrays;
  - `from_json` returns the value together with the issues;
    `from_json_strict` fails on any error.
- [x] Cargo features per specification category (`foundation`, `base`,
      `clinical`, `financial`, `specialized`).
- [x] Conformance: all 5,309 official R4 examples read and write back
      byte-for-byte equivalent JSON. The 15 examples that are invalid in the
      package itself are pinned to exactly their known problems.
- [ ] Required bindings: code lists from the package's CodeSystems for
      every `required` binding, checked on read (`Observation.status`).
- [ ] Builders or constructors for ergonomic creation, and
      `Reference::to::<Patient>("123")` helpers.
- [ ] XML reading and writing (optional feature).
- [ ] Measure and cap WebAssembly size per feature group.

## Phase 2: typed search

Generated from the package's 1,400 `SearchParameter`s. Each one has a
name, a type (token, reference, date, string, quantity, uri, number,
composite, special), a list of resource types it applies to, reference
targets and a FHIRPath expression.

- [ ] `Observation::search().code(token).date(ge("2024"))`: only the
      parameters that exist for the resource, with typed values and the
      modifiers each type allows (`:exact`, `:contains`, `:missing`, `:not`,
      `:above`, `:below`, `:in`, `:of-type`, …).
- [ ] Chaining (`subject:Patient.identifier=…`), checked against the
      reference targets; reverse chaining (`_has`); `_include` and
      `_revinclude` with their `:iterate` forms. All of these are valid by
      construction.
- [ ] Result parameters (`_sort`, `_count`, `_elements`, `_summary`,
      `_total`, `_contained`).
- [ ] Rendering to URL form, and parsing a URL back into the typed form, so
      incoming queries can be checked and explained.
- [ ] Composite parameters (`component-code-value-quantity`).
- [ ] Tests: every SearchParameter's expression and targets round-trip, plus
      the examples from the FHIR search page.

## Phase 3: FHIRPath

- [ ] Parser for the FHIRPath 2.0 grammar (normative in R4), with
      positioned errors.
- [ ] Evaluator over the typed models, through a generated reflection
      layer (child elements by name, type names, choice resolution for
      `ofType`/`is`/`as`). It keeps decimal precision and gives
      date/time partial-precision comparison semantics.
- [ ] Function library, in this order: the SQL-on-FHIR shareable subset
      first (`where`, `exists`, `empty`, `first`, `ofType`, `extension`,
      `join`, indexers, arithmetic, comparison), then the rest of the
      specification.
- [ ] Custom function registry that accepts type-specifier arguments
      (`getReferenceKey(Patient)`), and environment variables (`%resource`,
      `%context`, `%rowIndex`, typed `%constant`s).
- [ ] Run the generated invariants (`obs-6`, `ele-1`, …) as part of
      validation.
- [ ] Gate: HL7's official FHIRPath test suite (R4 `tests-fhir-r4.xml`).
- [ ] Reference to check against: helios-fhirpath (MIT).

## Phase 4: mappings

- [ ] hl7kit gets a general v2 → FHIR layer:
  - PID → Patient
  - PV1 → Encounter
  - ORC/OBR → ServiceRequest
  - OBX → Observation
  - AL1 → AllergyIntolerance
  - DG1 → Condition

  Follow the HL7 v2-to-FHIR ConceptMaps where they exist.
- [ ] The radiology mapping (DICOM → ImagingStudy, MII profiles) moves from
      `dicomscope-core` onto fhirkit types. dicomscope's bundle and its CI
      validation must come out unchanged.

## Phase 5: Python

- [ ] `fhirkit-py` (PyO3 in its own crate, as with hl7kit-py): typed resource
      classes or dict views, `parse` and `validate` with issue lists, the search
      builder, FHIRPath `evaluate`.
- [ ] Positioning: fast, R4 support (which fhir.resources dropped in v7),
      path-precise issues, v2 mapping. Benchmark against fhir.resources.

## SQL-on-FHIR (planned; starts after FHIRPath)

Target specification: view semantics as in the 3.0.0 ballot (Sept 2026,
FHIR 6.0.0 based). A 2.0.0 view (published 2024-10) gives the same rows,
and fhirkit will read both JSON shapes: `resourceType` as a URL or a
token, `identifier` as an object or an array.

Milestones:

1. **ViewDefinition model and static checks.** Unique and SQL-safe column
   names; `unionAll` branches with the same columns in the same order;
   at most one of `forEach`, `forEachOrNull` and `repeat` per select; every
   FHIRPath expression parses.
2. **FHIRPath prerequisites.** These come from phase 3:
   - `getResourceKey()`, `getReferenceKey(Type)` with a pluggable key
     strategy (default `Type/id`);
   - `%rowIndex` and typed constants of all 19 types;
   - `ofType` on choice elements;
   - `lowBoundary`/`highBoundary`;
   - errors instead of silent results for a non-boolean `where` and for a
     multi-valued column without `collection`.
3. **Runner.**
   - The specification's processing algorithm: Cartesian product of
     selects, `forEach` as an inner join, `forEachOrNull` as a left join,
     recursive `repeat`, `unionAll`, column order, `where` filters.
   - Contained resources are not emitted as rows.
   - Rows carry typed FHIR values.
   - Gate: the official suite from `FHIR/sql-on-fhir.js` `tests/*.json`
     (MIT; 144 tests, 133 shareable and 11 experimental), run by a Rust
     test that writes `test_report.json`. The suite changes often, so it
     is not vendored: `fetch-fixtures.sh` fetches it fresh from `main` in
     every CI run and every `local-ci.sh` run, into
     `tmp/fhir/sql-on-fhir/`, and records the commit. Target 133/133, then 144/144,
     then registration on sql-on-fhir.org's implementations page.
4. **Output.**
   - JSON, NDJSON and CSV without dependencies.
   - Behind an `arrow` feature: Arrow `RecordBatch`, with the schema taken
     from `column.type`, the specification's default SQL type map and
     `ansi/type` tags; Parquet from that.
   - `CREATE TABLE` generation.
5. **Scale.** Streaming NDJSON and Bulk Data export folders, fan-out per
   resource type, parallel with bounded memory.
6. **Python.**
   - `run_view(view, resources | path)` returning dict rows or a
     `pyarrow.Table` (zero-copy via `pyo3-arrow`, so pandas, polars and
     DuckDB work directly);
   - `validate_view`, `to_parquet`.
7. **Later, optional.**
   - SQLQuery/SQLView libraries executed through DuckDB or SQLite over the
     Arrow tables.
   - A FHIRPath-to-SQL backend (JSONB, DuckDB).
   - `$sql-run` / `$sql-export` endpoints in a separate server crate.

Prior art:
- helios-sof, with `pysof` on PyPI (MIT, Rust, 144/144, Parquet);
- octofhir-sof (MIT/Apache, in-memory and FHIRPath→SQL, 144/144);
- the reference implementation sof-js (MIT).

What fhirkit adds is R4 typed models underneath, the v2 → FHIR pipeline
in front, and Arrow-native Python.

Open questions:
- R4 only, or R5/R6 later? The ViewDefinition resource itself is R6.
- Is an HTTP operations layer in scope at all?
