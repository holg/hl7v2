//! `ServiceRequest` from ORC/OBR plus IPC or ZDS.
//!
//! Sources: HL7 "Version 2 to FHIR" IG, segment maps ORC → ServiceRequest
//! and OBR → ServiceRequest; the study-UID identifier is the one those maps
//! do not carry and the reason this output exists.

use super::datetime::hl7_datetime;
use super::ids::{
    plain_identifier, reference, typed_identifier, uid_identifier, Authority, Refs,
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
    if let Some(acc) = &order.accession {
        let authority = match order.source_path(hl7kit::order::OrderField::Accession) {
            Some("IPC-1.1") => Authority::from_hd(ei_authority(msg, "IPC-1").as_deref()),
            _ => Authority::default(),
        };
        identifiers.push(typed_identifier("ACSN", "Accession ID", acc, &authority));
    }
    // IPC-3.1 or ZDS-1.1: the Study Instance UID as urn:dicom:uid. This is
    // the row generic v2-to-FHIR mappers leave out.
    if let Some(uid) = &order.study_uid {
        identifiers.push(uid_identifier(uid));
    }
    // OBR-19 / IPC-2.1: Requested Procedure ID. v2-0203 has no type code for
    // it, so it is carried untyped rather than given an invented type.
    if let Some(rp) = &order.procedure_id {
        identifiers.push(plain_identifier(rp));
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

fn component(msg: &Message, path: &str) -> Option<String> {
    msg.get_decoded(path)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
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
        assert_eq!(status(Some("CA")), "revoked");
        assert_eq!(status(Some("dc")), "revoked");
        assert_eq!(status(None), "active");
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
