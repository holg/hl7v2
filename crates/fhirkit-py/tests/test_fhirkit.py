"""fhirkit's Python surface: parsing, issues with paths, lossless decimals."""
import decimal, json
import pytest, fhirkit

PATIENT = {
    "resourceType": "Patient",
    "id": "example",
    "name": [{"family": "Chalmers", "given": ["Peter", "James"]}],
    "birthDate": "1974-12-25",
    "_birthDate": {"extension": [{
        "url": "http://hl7.org/fhir/StructureDefinition/patient-birthTime",
        "valueDateTime": "1974-12-25T14:35:45-05:00"}]},
}


def test_valid_resource_from_dict_str_and_bytes():
    for data in (PATIENT, json.dumps(PATIENT), json.dumps(PATIENT).encode()):
        r = fhirkit.parse(data, strict=True)
        assert (r.resource_type, r.id, r.issues) == ("Patient", "example", [])
    assert fhirkit.validate(PATIENT) == []


def test_round_trip_keeps_primitive_extensions():
    out = fhirkit.parse(PATIENT).to_dict()
    assert out == PATIENT


def test_decimals_keep_their_precision():
    obs = '{"resourceType":"Observation","status":"final","code":{"text":"w"},"valueQuantity":{"value":72.50}}'
    r = fhirkit.parse(obs, strict=True)
    value = r.to_dict()["valueQuantity"]["value"]
    assert isinstance(value, decimal.Decimal) and str(value) == "72.50"
    assert '"value":72.50' in r.to_json()
    # And back in from a dict holding a Decimal.
    assert '"value":72.50' in fhirkit.parse(r.to_dict()).to_json()


def test_every_problem_is_reported_with_its_path():
    bad = {"resourceType": "Observation", "valueString": "x", "valueBoolean": True,
           "subject": {"reference": "Patient/1"}, "foo": 1}
    issues = fhirkit.validate(bad)
    paths = [i.path for i in issues]
    assert "Observation.value[x]" in paths
    assert "Observation.status" in paths and "Observation.code" in paths
    assert "Observation.foo" in paths
    assert all(i.severity == "error" for i in issues)
    r = fhirkit.parse(bad)                      # lenient: still a resource
    assert r.resource_type == "Observation" and len(r.issues) == len(issues)


def test_strict_raises_with_issues():
    with pytest.raises(fhirkit.ValidationError) as e:
        fhirkit.parse({"resourceType": "Patient", "gender": "x y "}, strict=True)
    assert [i.path for i in e.value.issues] == ["Patient.gender"]


def test_not_json_and_not_a_resource():
    with pytest.raises(fhirkit.ParseError):
        fhirkit.parse("{not json")
    with pytest.raises(fhirkit.ValidationError):
        fhirkit.parse({"resourceType": "NoSuchThing"})
    with pytest.raises(TypeError):
        fhirkit.parse({"resourceType": "Patient", "id": object()})


def test_module_metadata():
    assert fhirkit.FHIR_VERSION == "4.0.1"
    assert "Patient" in fhirkit.RESOURCE_TYPES and len(fhirkit.RESOURCE_TYPES) == 146
    assert fhirkit.__version__


def test_cli(tmp_path, capsys):
    from fhirkit.cli import main
    good = tmp_path / "p.json"; good.write_text(json.dumps(PATIENT))
    bad = tmp_path / "b.json"; bad.write_text('{"resourceType": "Patient", "gender": 1}')
    assert main(["validate", str(good)]) == 0
    assert main(["validate", str(good), str(bad)]) == 1
    assert "Patient.gender" in capsys.readouterr().out
    assert main(["format", str(good)]) == 0


def test_parse_many_keeps_order_and_names_bad_items():
    items = [dict(PATIENT, id=f"p{i}") for i in range(300)]
    items[7] = json.dumps(items[7])                 # str and bytes mix in
    items[8] = json.dumps(items[8]).encode()
    for threads in (1, 4, 0):
        out = fhirkit.parse_many(items, threads=threads)
        assert [r.id for r in out] == [f"p{i}" for i in range(300)]
    with pytest.raises(fhirkit.ParseError, match="item 2"):
        fhirkit.parse_many([PATIENT, PATIENT, "{nope"])
    with pytest.raises(fhirkit.ValidationError, match="item 1") as e:
        fhirkit.parse_many([PATIENT, {"resourceType": "Patient", "gender": "x y "}], strict=True)
    assert [i.path for i in e.value.issues] == ["Patient.gender"]
