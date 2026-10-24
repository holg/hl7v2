# fhirkit-rs

FHIR R4 for Python, in Rust. It covers all 146 resources and every
datatype, generated from the official specification. JSON goes in and
out without loss: decimals stay `Decimal` (`72.50` stays `72.50`), and
extensions on primitives survive. Validation reports **every** problem
with the element's path, not only the first.

```sh
pip install fhirkit-rs
```

```python
import fhirkit

r = fhirkit.parse(open("patient.json", "rb").read())   # str, bytes or dict
r.resource_type, r.id, r.issues                         # ('Patient', 'example', [])

for issue in fhirkit.validate({"resourceType": "Observation", "valueString": "x", "valueBoolean": True}):
    print(issue)
# error at Observation.value[x]: only one type may be given, found valueString, valueBoolean
# error at Observation.status: required element missing
# error at Observation.code: required element missing

fhirkit.parse(data, strict=True)   # raises fhirkit.ValidationError(.issues) on any error
fhirkit.parse_many(items)          # many at once, on all cores, GIL released
r.to_dict()                        # plain Python values, decimals as decimal.Decimal
r.to_json(pretty=True)             # canonical FHIR JSON
```

```sh
fhirkit validate *.json            # exit 1 if any resource has an error
fhirkit format patient.json        # canonical, pretty-printed
```

- **R4 (4.0.1).** fhir.resources dropped plain R4 in its 7.0 release.
- **Zero runtime dependencies.** One abi3 wheel per platform covers
  Python 3.9 and later.
- **Conformance.** All 5,309 examples in the official R4 examples package
  read and write back unchanged.
- **Fast.** On an M2 Max, the 5,309 R4 examples (188 MB) are read and
  validated in 0.56 s with `parse_many`, against 14.0 s for fhir.resources
  6.5 (25×). Written back without changing a single value, where
  fhir.resources alters 197 decimals. Details: [BENCH.md](https://github.com/holg/hl7v2/blob/main/crates/fhirkit/BENCH.md).

The PyPI name is `fhirkit-rs` because `fhirkit` on PyPI is an unrelated
project; the import name is `fhirkit`. The Rust crate is
[fhirkit](https://docs.rs/fhirkit). Typed search, FHIRPath and
SQL-on-FHIR are coming; see the
[roadmap](https://github.com/holg/hl7v2/blob/main/crates/fhirkit/ROADMAP.md).
