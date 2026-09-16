import pathlib, hl7kit, mwlkit, pydicom
FX = pathlib.Path(__file__).parent.parent.parent / "hl7kit-py" / "tests" / "fixtures"


def test_orm_to_worklist_roundtrips_through_pydicom():
    msg = hl7kit.parse((FX / "opg_order_ORM_O01.hl7").read_bytes())
    item = mwlkit.worklist_item(msg, default_station_ae="OPG1_AE")
    ds = mwlkit.to_pydicom(item)
    assert ds.AccessionNumber == "ACC-2026-000917"
    assert ds.StudyInstanceUID == item.study_uid
    sps = ds.ScheduledProcedureStepSequence[0]
    assert sps.Modality == "PX" and sps.ScheduledProcedureStepID == "SPS-2026-000917"
    assert ds.PatientName == "DENTEX^VAL27"
    assert item.study_uid_origin.startswith("ZDS")       # came from the order, not generated


def test_refuse_policy_needs_uid_in_order():
    msg = hl7kit.parse(b"MSH|^~\\&|A|B|C|D|20260916||ORM^O01|1|P|2.5.1\rPID|1||X||Y^Z\rORC|NW|1\rOBR|1|1||PAN^OPG^L\r")
    import pytest
    with pytest.raises(ValueError):
        mwlkit.worklist_item(msg, uid_policy="refuse")
