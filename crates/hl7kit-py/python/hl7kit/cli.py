"""`hl7kit inspect FILE...` : segments, linkage fields, UID source, warnings. Exit 1 on any warning."""
import argparse, json, sys
from . import parse, ParseError


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="hl7kit")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("inspect", help="show linkage fields of HL7 v2 messages")
    p.add_argument("files", nargs="+")
    p.add_argument("--json", action="store_true")
    a = ap.parse_args(argv)
    rc = 0
    for f in a.files:
        try:
            msg = parse(open(f, "rb").read())
        except ParseError as e:
            print(f"{f}: parse error: {e}", file=sys.stderr); rc = 2; continue
        o = msg.order()
        if a.json:
            print(json.dumps({"file": f, "type": msg.message_type, "segments": msg.segments(),
                              "order": o.to_dict() if o else None}, indent=2))
        else:
            print(f"{f}: {msg.message_type}  [{' '.join(msg.segments())}]")
            if o:
                uid = f"{o.study_uid.value} ({o.study_uid.source})" if o.study_uid else "-"
                print(f"  accession={o.accession_number}  rp={o.requested_procedure_id}  sps={o.scheduled_procedure_step_id}")
                print(f"  study_uid={uid}")
                for w in o.warnings: print(f"  WARNING {w}")
        if o and o.warnings: rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
