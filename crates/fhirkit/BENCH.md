# fhirkit benchmarks

What reading, validating and writing FHIR R4 JSON costs with fhirkit (Rust),
fhirkit-rs (its Python package) and fhir.resources, the common Python
library. The benchmark checks correctness and fidelity as well as speed.

Measured 2026-09-28 on one machine; see [Conditions](#conditions).

## Data

The official **hl7.fhir.r4.examples 4.0.1** package: 5,309 resources in 187.6
MB of JSON, ranging from single Patients to Bundles of several MB. 15 of the
examples are invalid R4 in the package itself (listed in
`tests/r4_examples.rs`). Files are read into memory before timing starts,
so disk speed is not measured.

## Results

### Read and validate everything

| | Median | vs fhir.resources 6.5 |
|---|---:|---:|
| fhir.resources 8.3 (Python, R4B models) | 14.91 s | 0.9× |
| fhir.resources 6.5 (Python, R4) | 14.00 s | 1× |
| fhirkit-rs `parse`, one document at a time | 2.53 s | 5.5× |
| fhirkit (Rust), 1 thread | 2.11 s | 6.6× |
| **fhirkit-rs `parse_many`, 6 threads** | **0.56 s** | **25×** |
| **fhirkit (Rust) `from_json_many`, 6 threads** | **0.49 s** | **29×** |

fhirkit's single-thread time splits into JSON parsing (283 ms) and decoding
into typed resources with validation (1,795 ms).

### Write everything back as JSON

| | Median |
|---|---:|
| fhir.resources 8.3 | 16.85 s |
| fhir.resources 6.5 | 2.58 s |
| fhirkit-rs `to_json` | 0.58 s |
| fhirkit (Rust), 1 thread | 0.59 s |

### Threads

`from_json_many` (Rust, median of 7 rounds) and `parse_many` (Python,
median of 5 rounds). The thread counts ran interleaved within each round,
so background load hits every count about equally.

| Threads | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Rust (ms) | 2110 | 1121 | 788 | 626 | 525 | **490** | 492 | 504 | 518 | 525 | 538 | 559 |
| Python (s) | 2.32 | 1.27 | 0.91 | 0.73 | 0.61 | **0.56** | 0.56 | 0.57 | 0.58 | 0.59 | 0.60 | 0.62 |

- **Scaling is near-linear up to 4 threads** (3.4×).
- **It levels off from 6 to 8**, then slows slightly: 12 threads are about 14%
  slower than 6 in Rust and 11% in Python.
- **Why:** the M2 Max has 8 performance and 4 efficiency cores, and decoding
  allocates a great deal (every string and element is a heap allocation), so
  extra threads mostly compete for the allocator and memory rather than add
  work. Fewer allocations should lift this plateau (see below).
- **The Python path is only about 70 ms behind Rust.** The GIL is released
  while the Rust threads work; the difference is copying input in and
  creating the Python objects.
- **Recommendation:** on this machine pass `threads=6` to `threads=8`. The
  default (`0`, one per core) is within about 15% of the best.

### Correctness

| | fhirkit | fhir.resources 6.5 | fhir.resources 8.3 |
|---|---:|---:|---:|
| Examples rejected | 15 | 625 | 76 |
| rejected by it alone | 0 | 610 | 61 |

The 15 are the examples invalid in the package, and all three libraries
reject them. Of the 610 that only fhir.resources 6.5 rejects:

| Errors | Why fhir.resources rejects | Correct R4 |
|---:|---|---|
| 5,807 | element ids such as `Observation.value[x]:valueQuantity` fail the resource-id pattern or its 64-character limit | an *element's* id is a plain string in R4; the id rules apply to resource ids |
| 1,035 | `_valueString`, an extension on a primitive choice value: "extra fields not permitted" | valid FHIR JSON |
| 13 | MedicinalProduct\*, EffectEvidenceSynthesis, RiskEvidenceSynthesis, SubstanceSpecification: model missing | these are R4 resources |
| 1 | `Evidence.variableDefinition` required | that element only exists from R4B on |
| 1 | a value that is a lone no-break space | debatable; HL7's validator accepts it, as fhirkit does |

The 61 extra rejections of fhir.resources 8.3 are mostly the same
`_valueString` gap; some may come from reading R4 data with R4B models.

### Fidelity: does writing change the data?

| | fhirkit | fhir.resources 6.5 | fhir.resources 8.3 |
|---|---:|---:|---:|
| Examples changed on write | **0** of 5,294 | 1,796 of 4,684 | 1,821 of 5,233 |
| Decimal values altered | **0** | 197 | 197 |

What fhir.resources changes:
- **decimal precision:** `105.00` becomes `105.0` in Claims, and `0.40`
  becomes `0.4` in lab reference ranges;
- **time zones:** `Z` becomes `+00:00` in 6.5, and the reverse in 8.3;
- **instant precision:** `.715Z` becomes `.715000Z`;
- **base64 content** is rewrapped.

fhirkit keeps decimals as written text (in Python as `decimal.Decimal`),
and writes primitives exactly as it read them.

Decimals are compared as written text, value by value in document order,
because `0.40` equals `0.4` as a number.

## Conditions

| | |
|---|---|
| Machine | Apple M2 Max, 8 performance + 4 efficiency cores, 96 GB, macOS 26.6.2 |
| Rust | 1.96.0, `native` profile (opt-level 3, LTO); the workspace's `release` profile optimises for size and takes about 50% longer here (3.1 s instead of 2.1 s single-threaded) |
| Python | 3.12.11; fhirkit-rs 0.1.0 wheel (maturin, `--release`); fhir.resources 6.5.0 with pydantic 1.10.26, fhir.resources 8.3.0 with pydantic 2.13.5 |
| Load | not an idle machine: load average 4-7 during the runs, from an Android emulator, the window server and other apps; medians over interleaved rounds keep the comparisons fair, and absolute times on an idle machine should be a little lower |

## Reproduce

```sh
crates/fhirkit/fetch-fixtures.sh                     # R4 core + examples into tmp/fhir/

# Rust: single-thread breakdown, then the thread sweep
cargo run --profile native -p fhirkit --example validate_dir -- tmp/fhir/examples/package --threads 1
cargo run --profile native -p fhirkit --example validate_dir -- tmp/fhir/examples/package --sweep 7

# Python: in an environment with fhirkit-rs and one fhir.resources version
pip install "fhir.resources==6.5.0" "pydantic<2"     # R4; or "fhir.resources>=8" for R4B
python crates/fhirkit-py/bench/compare.py tmp/fhir/examples/package --rounds 3
```

## Next steps suggested by these numbers

- **Fewer allocations** while decoding: reuse strings and store short ones
  compactly. This targets the plateau from 6 threads on.
- **An optional faster allocator** (e.g. mimalloc) behind a feature, so the
  core crate stays dependency-free. Measure it with `--sweep` first.
- **GPUs** do not suit this step: the parsing is branchy and its output is
  a tree of allocations. They fit SQL-on-FHIR analytics over columnar data;
  see ROADMAP.md.
