//! `ServiceRequest` from ORC/OBR plus IPC or ZDS.
//!
//! Sources: HL7 "Version 2 to FHIR" IG, segment maps ORC → ServiceRequest
//! and OBR → ServiceRequest; the study-UID identifier is the one those maps
//! do not carry and the reason this output exists.

use super::datetime::hl7_datetime;
use super::ids::{
    fhir_string, plain_identifier, reference, typed_identifier, uid_identifier, Authority, Refs,
    SERVICE_REQUEST_ID,
};
use hl7kit::order::Order;
use hl7kit::Message;
use serde_json::{json, Value};

pub fn service_request(msg: &Message, order: &Order, refs: &Refs) -> Value {
    let mut sr = json!({
        "resourceType": "ServiceRequest",
        "id": SERVICE_REQUEST_ID,
        // ORC-1 order control → status; v2-to-FHIR maps NW/XO to active and
        // CA/DC to revoked. Anything else is treated as active, which is what
        // an order that reached the imaging system is.
        "status": status(msg.get("ORC-1")),
        "intent": "order",
        "subject": reference(&refs.patient),
    });

    let mut identifiers = Vec::new();
    // ORC-2 placer, ORC-3 filler: EI, first component is the number, the
    // namespace (EI.2..4) becomes the authority.
    for (path, code, display) in [
        ("ORC-2", "PLAC", "Placer Identifier"),
        ("ORC-3", "FILL", "Filler Identifier"),
    ] {
        if let Some(value) = component(msg, &format!("{path}.1")) {
            let authority = Authority::from_hd(ei_authority(msg, path).as_deref());
            identifiers.push(typed_identifier(code, display, &value, &authority));
        }
    }
    // OBR-18 (or IPC-1.1 for OMI): accession. No universal system exists for
    // a hospital's accession namespace; IPC-1.2 supplies one when present.
    if let Some(acc) = order.accession.as_deref().and_then(fhir_string) {
        let authority = match order.source_path(hl7kit::order::OrderField::Accession) {
            Some("IPC-1.1") => Authority::from_hd(ei_authority(msg, "IPC-1").as_deref()),
            _ => Authority::default(),
        };
        identifiers.push(typed_identifier("ACSN", "Accession ID", &acc, &authority));
    }
    // IPC-3.1 or ZDS-1.1: the Study Instance UID as urn:dicom:uid. This is
    // the row generic v2-to-FHIR mappers leave out. Not a UID, not emitted.
    if let Some(uid) = &order.study_uid {
        identifiers.extend(uid_identifier(uid));
    }
    // OBR-19 / IPC-2.1: Requested Procedure ID. v2-0203 has no type code for
    // it, so it is carried untyped rather than given an invented type.
    if let Some(rp) = order.procedure_id.as_deref().and_then(fhir_string) {
        identifiers.push(plain_identifier(&rp));
    }
    if !identifiers.is_empty() {
        sr["identifier"] = json!(identifiers);
    }

    // OBR-4 universal service identifier (CWE): code^text^coding system.
    // The coding system is used as `system` only when it is a URI; HL7
    // table names like `L` are not, and are dropped rather than faked.
    let code = component(msg, "OBR-4.1");
    let text = component(msg, "OBR-4.2");
    let system = component(msg, "OBR-4.3").filter(|s| super::ids::is_uri(s));
    if code.is_some() || text.is_some() {
        let mut cc = json!({});
        if let Some(code) = &code {
            let mut coding = json!({ "code": code });
            if let Some(system) = &system {
                coding["system"] = json!(system);
            }
            if let Some(text) = &text {
                coding["display"] = json!(text);
            }
            cc["coding"] = json!([coding]);
        }
        if let Some(text) = &text {
            cc["text"] = json!(text);
        }
        sr["code"] = cc;
    }

    // ORC-9 date/time of transaction → authoredOn (date only without a zone).
    if let Some(when) = msg.get("ORC-9").and_then(hl7_datetime) {
        sr["authoredOn"] = json!(when);
    }

    // ORC-12 ordering provider, else OBR-16 (XCN: id^family^given).
    let requester = ["ORC-12", "OBR-16"]
        .iter()
        .find_map(|p| xcn_display(msg, p));
    if let Some(display) = requester {
        sr["requester"] = json!({ "display": display });
    }
    sr
}

fn status(orc1: Option<&str>) -> &'static str {
    match orc1.map(|s| s.trim().to_ascii_uppercase()).as_deref() {
        Some("CA") | Some("DC") | Some("OC") => "revoked",
        Some("HD") => "on-hold",
        _ => "active",
    }
}

/// A decoded, trimmed component with control characters removed; `None`
/// when absent, blank or the HL7 null `""`.
fn component(msg: &Message, path: &str) -> Option<String> {
    msg.get_decoded(path).and_then(|v| fhir_string(&v))
}

/// EI components 2..4 as an HD (`namespace^universal id^type`).
fn ei_authority(msg: &Message, field: &str) -> Option<String> {
    let ns = component(msg, &format!("{field}.2"));
    let uid = component(msg, &format!("{field}.3"));
    let ty = component(msg, &format!("{field}.4"));
    if ns.is_none() && uid.is_none() {
        return None;
    }
    Some(format!(
        "{}^{}^{}",
        ns.unwrap_or_default(),
        uid.unwrap_or_default(),
        ty.unwrap_or_default()
    ))
}

/// `Family, Given` from an XCN, or the bare ID when no name is present.
fn xcn_display(msg: &Message, field: &str) -> Option<String> {
    let family = component(msg, &format!("{field}.2"));
    let given = component(msg, &format!("{field}.3"));
    match (family, given) {
        (Some(f), Some(g)) => Some(format!("{f}, {g}")),
        (Some(f), None) => Some(f),
        (None, Some(g)) => Some(g),
        (None, None) => component(msg, &format!("{field}.1")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses() {
        assert_eq!(status(Some("NW")), "active");
        assert_eq!(status(Some("XO")), "active");
        assert_eq!(status(Some("SC")), "active", "status change");
        assert_eq!(status(Some("SN")), "active");
        assert_eq!(status(Some("OK")), "active");
        assert_eq!(status(Some("CA")), "revoked");
        assert_eq!(status(Some("dc")), "revoked");
        assert_eq!(status(Some("OC")), "revoked");
        assert_eq!(status(Some(" ca ")), "revoked", "padding");
        assert_eq!(status(Some("HD")), "on-hold");
        assert_eq!(status(Some("hd")), "on-hold");
        assert_eq!(status(Some("")), "active");
        assert_eq!(
            status(Some("ZZ")),
            "active",
            "unknown code: the order reached imaging"
        );
        assert_eq!(status(None), "active");
    }

    fn parse(segments: &str) -> (Message, Order) {
        let text = format!(
            "MSH|^~\\&|RIS|HOSP|||20260905||ORM^O01|1|P|2.5.1\rPID|1||P1^^^HOSP^MR\r{segments}"
        );
        let msg = Message::parse(text).unwrap();
        let order = Order::extract(&msg);
        (msg, order)
    }

    fn sr(segments: &str) -> Value {
        let (msg, order) = parse(segments);
        service_request(&msg, &order, &Refs::for_study("x"))
    }

    #[test]
    fn order_control_drives_status_end_to_end() {
        for (code, expect) in [
            ("NW", "active"),
            ("XO", "active"),
            ("CA", "revoked"),
            ("DC", "revoked"),
            ("OC", "revoked"),
            ("HD", "on-hold"),
            ("", "active"),
        ] {
            let v = sr(&format!("ORC|{code}|ORD-1\rOBR|1|ORD-1||X^Y\r"));
            assert_eq!(v["status"], expect, "ORC-1 {code:?}");
        }
        let v = sr("OBR|1|ORD-1||X^Y\r");
        assert_eq!(v["status"], "active", "no ORC at all");
    }

    #[test]
    fn obr4_permutations() {
        let v = sr("ORC|NW\rOBR|1|||CT-HEAD\r");
        assert_eq!(v["code"]["coding"][0]["code"], "CT-HEAD");
        assert!(v["code"]["coding"][0].get("display").is_none());
        assert!(v["code"].get("text").is_none());

        let v = sr("ORC|NW\rOBR|1|||^CT head\r");
        assert!(v["code"].get("coding").is_none(), "text without a code");
        assert_eq!(v["code"]["text"], "CT head");

        let v = sr("ORC|NW\rOBR|1|||CT-HEAD^CT head\r");
        assert_eq!(v["code"]["coding"][0]["code"], "CT-HEAD");
        assert_eq!(v["code"]["coding"][0]["display"], "CT head");
        assert!(v["code"]["coding"][0].get("system").is_none());
        assert_eq!(v["code"]["text"], "CT head");

        let v = sr("ORC|NW\rOBR|1|||CT-HEAD^CT head^L\r");
        assert!(
            v["code"]["coding"][0].get("system").is_none(),
            "`L` is a table name"
        );
        let v = sr("ORC|NW\rOBR|1|||CT-HEAD^CT head^LN\r");
        assert!(
            v["code"]["coding"][0].get("system").is_none(),
            "`LN` is not a URI either"
        );

        let v = sr("ORC|NW\rOBR|1|||24725-4^CT head^http://loinc.org\r");
        assert_eq!(v["code"]["coding"][0]["system"], "http://loinc.org");
        let v = sr("ORC|NW\rOBR|1|||24725-4^CT head^urn:oid:2.16.840.1.113883.6.1\r");
        assert_eq!(
            v["code"]["coding"][0]["system"],
            "urn:oid:2.16.840.1.113883.6.1"
        );

        let v = sr("ORC|NW\rOBR|1|||^^L\r");
        assert!(v.get("code").is_none(), "a system alone is nothing");
        let v = sr("ORC|NW\rOBR|1\r");
        assert!(v.get("code").is_none());
        let v = sr("ORC|NW\rOBR|1|||\"\"^\"\"\r");
        assert!(v.get("code").is_none(), "HL7 null values");

        // Escapes are decoded, padding and control characters go.
        let v = sr("ORC|NW\rOBR|1|||CT\\S\\HEAD^ CT \\T\\ head \r");
        assert_eq!(v["code"]["coding"][0]["code"], "CT^HEAD");
        assert_eq!(v["code"]["text"], "CT & head");
    }

    #[test]
    fn requester_precedence_and_shapes() {
        let v =
            sr("ORC|NW|||||||||||123^Curie^Marie\rOBR|1|||X^Y||||||||||||456^Roentgen^Wilhelm\r");
        assert_eq!(v["requester"]["display"], "Curie, Marie", "ORC-12 first");
        let v = sr("ORC|NW\rOBR|1|||X^Y||||||||||||456^Roentgen^Wilhelm\r");
        assert_eq!(
            v["requester"]["display"], "Roentgen, Wilhelm",
            "OBR-16 fallback"
        );
        let v = sr("ORC|NW|||||||||||^Curie\rOBR|1|||X^Y\r");
        assert_eq!(v["requester"]["display"], "Curie", "family only");
        let v = sr("ORC|NW|||||||||||^^Marie\rOBR|1|||X^Y\r");
        assert_eq!(v["requester"]["display"], "Marie", "given only");
        let v = sr("ORC|NW|||||||||||123\rOBR|1|||X^Y\r");
        assert_eq!(
            v["requester"]["display"], "123",
            "bare identifier when there is no name"
        );
        let v = sr("ORC|NW|||||||||||^^^^\rOBR|1|||X^Y||||||||||||456^Roentgen\r");
        assert_eq!(
            v["requester"]["display"], "Roentgen",
            "an empty ORC-12 does not block OBR-16"
        );
        let v = sr("ORC|NW\rOBR|1|||X^Y\r");
        assert!(v.get("requester").is_none());
    }

    #[test]
    fn authored_on_precision_follows_orc9() {
        let v = sr("ORC|NW||||||||20260905\rOBR|1|||X^Y\r");
        assert_eq!(v["authoredOn"], "2026-09-05");
        let v = sr("ORC|NW||||||||202609\rOBR|1|||X^Y\r");
        assert_eq!(v["authoredOn"], "2026-09");
        let v = sr("ORC|NW||||||||20260905113000\rOBR|1|||X^Y\r");
        assert_eq!(v["authoredOn"], "2026-09-05", "no zone: time dropped");
        let v = sr("ORC|NW||||||||20260905113000-0400\rOBR|1|||X^Y\r");
        assert_eq!(v["authoredOn"], "2026-09-05T11:30:00-04:00");
        let v = sr("ORC|NW||||||||20260931\rOBR|1|||X^Y\r");
        assert!(v.get("authoredOn").is_none(), "September 31st");
        let v = sr("ORC|NW||||||||yesterday\rOBR|1|||X^Y\r");
        assert!(v.get("authoredOn").is_none());
        let v = sr("ORC|NW\rOBR|1|||X^Y\r");
        assert!(v.get("authoredOn").is_none());
    }

    #[test]
    fn identifier_hygiene() {
        // A study UID that is not a UID is not carried as urn:dicom:uid.
        let v = sr("ORC|NW\rOBR|1|||X^Y\rZDS|not-a-uid^^Application^DICOM\r");
        assert!(
            v["identifier"]
                .as_array()
                .map(|ids| ids.iter().all(|i| i["system"] != "urn:dicom:uid"))
                .unwrap_or(true),
            "{v}"
        );
        // Placer and filler numbers with a blank EI.1 are not identifiers.
        let v = sr("ORC|NW|^RIS|^PACS\rOBR|1|||X^Y\r");
        assert!(v.get("identifier").is_none());
        // Whitespace-only accession and procedure IDs are omitted.
        let v = sr("ORC|NW\rOBR|1|||X^Y||||||||||||||   |  \r");
        assert!(v.get("identifier").is_none());
        // Assigning authority of an EI: namespace alone is a display, an ISO
        // OID is a system, a URI is a system, a bad OID is a display only.
        let v = sr("ORC|NW|A^RIS|B^PACS^1.2.3^ISO|||||||||\rOBR|1|||X^Y\r");
        let ids = v["identifier"].as_array().unwrap();
        assert_eq!(ids[0]["assigner"]["display"], "RIS");
        assert!(ids[0].get("system").is_none());
        assert_eq!(ids[1]["system"], "urn:oid:1.2.3");
        assert_eq!(ids[1]["assigner"]["display"], "PACS");
        let v = sr("ORC|NW|A^^http://ris.example/orders^URI|B^PACS^bad.oid.^ISO\rOBR|1|||X^Y\r");
        let ids = v["identifier"].as_array().unwrap();
        assert_eq!(ids[0]["system"], "http://ris.example/orders");
        assert!(ids[0].get("assigner").is_none());
        assert!(ids[1].get("system").is_none());
        assert_eq!(ids[1]["assigner"]["display"], "PACS");
        // No empty `identifier` array when nothing is there.
        let v = sr("ORC|NW\rOBR|1|||X^Y\r");
        assert!(v.get("identifier").is_none());
    }

    #[test]
    fn omi_accession_carries_the_ipc_namespace() {
        let v = sr("ORC|NW\rOBR|1\rIPC|ACC-9^HOSP|RP-9|1.2.3.9|SPS-9|CT\r");
        let ids = v["identifier"].as_array().unwrap();
        let acsn = ids
            .iter()
            .find(|i| i["type"]["coding"][0]["code"] == "ACSN")
            .unwrap();
        assert_eq!(acsn["value"], "ACC-9");
        assert_eq!(acsn["assigner"]["display"], "HOSP");
        assert!(ids
            .iter()
            .any(|i| i["system"] == "urn:dicom:uid" && i["value"] == "urn:oid:1.2.3.9"));
        assert!(ids
            .iter()
            .any(|i| i.get("type").is_none() && i["value"] == "RP-9"));
    }

    #[test]
    fn identifiers_and_code_from_an_orm() {
        let text = "MSH|^~\\&|RIS|HOSP|||20260905||ORM^O01|1|P|2.5.1\r\
                    PID|1||P1^^^HOSP^MR\r\
                    ORC|NW|ORD-1^RIS|FIL-1^PACS^1.2.3^ISO||||||20260905113000+0200|||^Curie^Marie\r\
                    OBR|1|ORD-1|FIL-1|CT-HEAD^CT head^L||||||||||||||ACC-1|RP-1\r\
                    ZDS|1.2.3.4^^Application^DICOM\r";
        let msg = Message::parse(text).unwrap();
        let order = Order::extract(&msg);
        let refs = Refs::for_study("1.2.3.4");
        let sr = service_request(&msg, &order, &refs);
        assert_eq!(sr["status"], "active");
        assert_eq!(sr["intent"], "order");
        let ids = sr["identifier"].as_array().unwrap();
        let by_code = |c: &str| {
            ids.iter()
                .find(|i| i["type"]["coding"][0]["code"] == c)
                .cloned()
        };
        let plac = by_code("PLAC").unwrap();
        assert_eq!(plac["value"], "ORD-1");
        assert_eq!(plac["assigner"]["display"], "RIS");
        assert!(plac.get("system").is_none());
        let fill = by_code("FILL").unwrap();
        assert_eq!(fill["system"], "urn:oid:1.2.3");
        assert_eq!(by_code("ACSN").unwrap()["value"], "ACC-1");
        let uid = ids.iter().find(|i| i["system"] == "urn:dicom:uid").unwrap();
        assert_eq!(uid["value"], "urn:oid:1.2.3.4");
        let untyped = ids
            .iter()
            .find(|i| i.get("type").is_none() && i.get("system").is_none())
            .unwrap();
        assert_eq!(untyped["value"], "RP-1");
        assert!(ids
            .iter()
            .all(|i| i.get("system").map(|s| s != "").unwrap_or(true)));
        assert_eq!(sr["code"]["coding"][0]["code"], "CT-HEAD");
        assert!(
            sr["code"]["coding"][0].get("system").is_none(),
            "`L` is not a URI"
        );
        assert_eq!(sr["code"]["text"], "CT head");
        assert_eq!(sr["authoredOn"], "2026-09-05T11:30:00+02:00");
        assert_eq!(sr["requester"]["display"], "Curie, Marie");
        assert_eq!(sr["subject"]["reference"], refs.patient);
    }
}
