"""Fixtures are the OPG demo messages: same Study Instance UID via ZDS (ORM), IPC (OMI), OBX 110180 (ORU)."""
import pathlib, pytest, hl7kit

FX = pathlib.Path(__file__).parent / "fixtures"
UID = "1.2.826.0.1.3680043.8.498.79323663392379978206252132467273068460"


def load(name): return hl7kit.parse((FX / name).read_bytes())


@pytest.mark.parametrize("name,source", [
    ("opg_order_ORM_O01.hl7", "ZDS-1.1"),
    ("opg_order_OMI_O23.hl7", "IPC-3.1"),
    ("opg_report_ORU_R01.hl7", "OBX-110180"),
])
def test_three_uid_sources(name, source):
    o = load(name).order()
    assert o.study_uid.value == UID and o.study_uid.source == source
    assert o.warnings == []


def test_ihe_rad4_fields_orm():
    o = load("opg_order_ORM_O01.hl7").order()
    assert (o.accession_number, o.requested_procedure_id, o.scheduled_procedure_step_id) == \
           ("ACC-2026-000917", "RP-2026-000917", "SPS-2026-000917")


def test_conflict_is_reported_not_swallowed():
    o = load("conflict_zds_vs_obx.hl7").order()
    assert o.study_uid.value == UID          # resolution order keeps ZDS over OBX
    assert any("OBX" in w and "ZDS" in w for w in o.warnings)


def test_paths_and_spans():
    m = load("opg_order_ORM_O01.hl7")
    assert m["PID-5"] == "DENTEX^VAL27" and m.message_type == "ORM^O01"
    s, e = m.span("ZDS-1.1"); assert m.raw()[s:e].decode().startswith(UID)


def test_str_input_and_parse_error():
    assert hl7kit.parse("MSH|^~\\&|A|B|C|D|20260101||ADT^A01|1|P|2.5.1\r").message_type == "ADT^A01"
    with pytest.raises(hl7kit.ParseError): hl7kit.parse(b"not hl7")
