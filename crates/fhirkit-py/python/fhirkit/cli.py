"""`fhirkit validate FILE...` lists problems (exit 1 if any error);
`fhirkit format FILE` prints the resource as canonical FHIR JSON."""
import argparse, json, sys
from . import ParseError, ValidationError, parse, validate


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="fhirkit")
    sub = ap.add_subparsers(dest="cmd", required=True)
    v = sub.add_parser("validate", help="check FHIR R4 JSON resources")
    v.add_argument("files", nargs="+")
    v.add_argument("--json", action="store_true", help="issues as JSON")
    f = sub.add_parser("format", help="print a resource as canonical FHIR JSON")
    f.add_argument("file")
    f.add_argument("--compact", action="store_true")
    a = ap.parse_args(argv)

    if a.cmd == "format":
        try:
            r = parse(open(a.file, "rb").read())
        except (ParseError, ValidationError) as e:
            print(f"{a.file}: {e}", file=sys.stderr)
            return 2
        print(r.to_json(pretty=not a.compact))
        return 1 if any(i.severity == "error" for i in r.issues) else 0

    rc = 0
    for name in a.files:
        try:
            issues = validate(open(name, "rb").read())
        except ParseError as e:
            print(f"{name}: not JSON: {e}", file=sys.stderr)
            rc = 2
            continue
        if a.json:
            print(json.dumps({"file": name, "issues": [
                {"severity": i.severity, "path": i.path, "message": i.message} for i in issues]}))
        else:
            print(f"{name}: {'ok' if not issues else f'{len(issues)} issue(s)'}")
            for i in issues:
                print(f"  {i}")
        if any(i.severity == "error" for i in issues) and rc == 0:
            rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
