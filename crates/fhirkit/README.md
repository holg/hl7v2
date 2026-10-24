# fhirkit

FHIR R4 for Rust. It has typed models for all 146 resources and every
datatype, generated from the official specification package. JSON is
read and written without losing anything: decimal precision, extensions
on primitives and key order all survive. While reading, fhirkit reports
every problem with the element's path, instead of stopping at the first
error.

```rust
use fhirkit::r4::{Observation, ObservationValue};

let obs: Observation = fhirkit::from_json_strict(r#"{
  "resourceType": "Observation",
  "status": "final",
  "code": {"text": "Body weight"},
  "valueQuantity": {"value": 72.50, "unit": "kg"}
}"#).unwrap();

if let Some(ObservationValue::Quantity(q)) = &obs.value {
    assert_eq!(q.value.as_ref().unwrap().value.as_ref().unwrap().as_str(), "72.50");
}

// Lenient reading: what could be read, plus every problem and where it is.
let (_, issues) = fhirkit::from_json::<Observation>(
    r#"{"resourceType": "Observation", "valueString": "x", "valueBoolean": true}"#,
).unwrap();
for issue in &issues {
    println!("{issue}");
}
// error at Observation.value[x]: only one type may be given, found valueString, valueBoolean
// error at Observation.status: required element missing
// error at Observation.code: required element missing
```

- **No dependencies.** It uses its own small JSON reader, because FHIR
  decimals must round-trip exactly. No `unsafe`, and it builds for
  WebAssembly.
- **Conformance.** All 5,309 examples of the official R4 examples package
  read and write back unchanged.
- **Parallel.** `from_json_many` reads many documents across cores: the
  5,309 R4 examples (188 MB) in 0.49 s on 6 threads of an M2 Max, 2.1 s
  on one; about 29× faster than fhir.resources. Full numbers, method and a
  correctness comparison are in [BENCH.md](BENCH.md).
- **Feature groups.** `foundation`, `base`, `clinical`, `financial` and
  `specialized`, as the specification files resources; all are on by
  default. Datatypes are always included.

For Python, the same crate ships as [`fhirkit-rs`](https://pypi.org/project/fhirkit-rs/)
(import name `fhirkit`). Planned: typed search queries, FHIRPath, v2 → FHIR
mappings and SQL-on-FHIR. See [ROADMAP.md](ROADMAP.md).

## Regenerating

```sh
curl -L -o r4core.tgz https://packages.fhir.org/hl7.fhir.r4.core/4.0.1
mkdir -p tmp/fhir/core && tar -xzf r4core.tgz -C tmp/fhir/core
cargo run -p fhirkit-gen -- tmp/fhir/core/package crates/fhirkit/src/r4
cargo fmt -p fhirkit
```

The conformance test reads the examples package from
`tmp/fhir/examples/package`, or from the path in `FHIRKIT_R4_EXAMPLES`
(download it from `https://packages.fhir.org/hl7.fhir.r4.examples/4.0.1`).
