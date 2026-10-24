"""fhirkit-rs against fhir.resources on the official R4 examples.

    python bench/compare.py <examples-package-dir> [--rounds N]

Needs fhirkit-rs and fhir.resources in the same environment. fhir.resources
below 7 has the R4 models; 7 and later only R4B (read with R4B models). The
examples are read into memory first; every timing is the median of
`--rounds` runs, and the thread counts of parse_many run interleaved so
background load spreads evenly. Results feed crates/fhirkit/BENCH.md.
"""
import argparse, json, pathlib, platform, statistics, sys, time
from decimal import Decimal

import fhirkit
import fhir.resources as fr

ap = argparse.ArgumentParser()
ap.add_argument("examples")
ap.add_argument("--rounds", type=int, default=3)
a = ap.parse_args()

R4 = int(fr.__version__.split(".")[0]) < 7
if R4:
    from fhir.resources import get_fhir_model_class
    fr_parse = lambda rt, raw: get_fhir_model_class(rt).parse_raw(raw)
    fr_dump = lambda o: o.json()
else:
    from fhir.resources.R4B import get_fhir_model_class
    fr_parse = lambda rt, raw: get_fhir_model_class(rt).model_validate_json(raw)
    fr_dump = lambda o: o.model_dump_json()

docs = []
for p in sorted(pathlib.Path(a.examples).glob("*.json")):
    if p.name.startswith(".") or p.name == "package.json":
        continue
    raw = p.read_bytes()
    rt = json.loads(raw).get("resourceType")
    if rt:
        docs.append((p.name, rt, raw))
raws = [d[2] for d in docs]


def timed(fn, rounds=a.rounds):
    runs = []
    for _ in range(rounds):
        t = time.perf_counter()
        out = fn()
        runs.append(time.perf_counter() - t)
    return statistics.median(runs), out


def fr_read_all():
    ok, bad = {}, {}
    for name, rt, raw in docs:
        try:
            ok[name] = fr_parse(rt, raw)
        except Exception as e:
            bad[name] = type(e).__name__
    return ok, bad


import pydantic
print(f"# {platform.machine()} {platform.system()} {platform.release()}, Python {platform.python_version()}")
print(f"# fhirkit-rs {fhirkit.__version__}, fhir.resources {fr.__version__} ({'R4' if R4 else 'R4B models'}), pydantic {pydantic.VERSION}")
print(f"# {len(docs)} examples, {sum(len(r) for r in raws) / 1e6:.1f} MB, median of {a.rounds}")

t_fk, fk = timed(lambda: [fhirkit.parse(r) for r in raws])
t_fk_out, fk_out = timed(lambda: [r.to_json() for r in fk])
t_fr, (fr_ok, fr_bad) = timed(fr_read_all)
t_fr_out, fr_out = timed(lambda: {n: fr_dump(o) for n, o in fr_ok.items()})
print(f"read+validate, one by one   fhirkit {t_fk:.2f} s   fhir.resources {t_fr:.2f} s")
print(f"write JSON                  fhirkit {t_fk_out:.2f} s   fhir.resources {t_fr_out:.2f} s")

threads = list(range(1, 13))
fhirkit.parse_many(raws, threads=8)
many = {n: [] for n in threads}
for _ in range(max(a.rounds, 5)):
    for n in threads:
        t = time.perf_counter()
        fhirkit.parse_many(raws, threads=n)
        many[n].append(time.perf_counter() - t)
print("parse_many  " + "  ".join(f"{n}:{statistics.median(v):.2f}" for n, v in many.items()))

# Correctness and fidelity.
fk_bad = {docs[i][0] for i, r in enumerate(fk) if any(x.severity == "error" for x in r.issues)}
print(f"rejected    fhirkit {len(fk_bad)}   fhir.resources {len(fr_bad)}   "
      f"only fhirkit {len(fk_bad - set(fr_bad))}   only fhir.resources {len(set(fr_bad) - fk_bad)}")


def strip(v):
    if isinstance(v, dict):
        return {k: strip(x) for k, x in v.items() if k != "fhir_comments"}
    if isinstance(v, list):
        return [strip(x) for x in v]
    return v


def load(b):
    return strip(json.loads(b, parse_float=Decimal))


def decimals(v, path="$"):
    """Every decimal with its path, compared as written text."""
    if isinstance(v, dict):
        for k, x in v.items():
            yield from decimals(x, f"{path}.{k}")
    elif isinstance(v, list):
        for i, x in enumerate(v):
            yield from decimals(x, f"{path}[{i}]")
    elif isinstance(v, Decimal):
        yield path, str(v)


src = {n: load(raw) for n, _, raw in docs}
names = [d[0] for d in docs]
fk_changed = sum(1 for n, out in zip(names, fk_out) if n not in fk_bad and load(out) != src[n])
fr_changed = sum(1 for n, out in fr_out.items() if load(out) != src[n])
fk_dec = sum(1 for n, out in zip(names, fk_out) if n not in fk_bad
             for (p, s), (_, t) in zip(decimals(src[n]), decimals(load(out))) if s != t)
fr_dec = sum(1 for n, out in fr_out.items()
             for (p, s), (_, t) in zip(decimals(src[n]), decimals(load(out))) if s != t)
print(f"changed on write   fhirkit {fk_changed} of {len(docs) - len(fk_bad)}   fhir.resources {fr_changed} of {len(fr_ok)}")
print(f"decimals altered   fhirkit {fk_dec}   fhir.resources {fr_dec}")
