//! FHIR R4 output: one `Bundle` of type `collection` holding `Patient`,
//! `ServiceRequest` and `ImagingStudy` for the loaded study and order.
//!
//! Built as `serde_json::Value`, no typed FHIR crate, no validation call,
//! no terminology lookup: everything is generated locally from the two
//! inputs. Free of Leptos and web-sys, so it is tested on the host.

pub mod datetime;
pub mod ids;
pub mod imaging_study;
pub mod patient;
pub mod service_request;

use crate::dicom::{Series, Study};
use crate::link::{LinkPath, Linkage};
use hl7kit::order::Order;
use hl7kit::Message;
use serde_json::{json, Value};

/// What the mapping reads.
pub struct FhirInput<'a> {
    pub study: &'a Study,
    pub series: &'a [Series],
    pub message: &'a Message,
    pub order: &'a Order,
    pub linkage: &'a Linkage,
}

/// The three resources, in bundle order.
pub struct Resources {
    pub patient: Value,
    pub service_request: Value,
    pub imaging_study: Value,
}

pub fn resources(input: &FhirInput<'_>) -> Resources {
    Resources {
        patient: patient::patient(input.study, Some(input.message)),
        service_request: service_request::service_request(input.message, input.order),
        imaging_study: imaging_study::imaging_study(
            input.study,
            input.series,
            input.linkage.path != LinkPath::None,
        ),
    }
}

/// A `collection` bundle with relative references. `fullUrl` is omitted
/// on purpose: it would need real UUIDs, and a collection bundle does not
/// require it.
pub fn bundle(input: &FhirInput<'_>) -> Value {
    let r = resources(input);
    json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            { "resource": r.patient },
            { "resource": r.service_request },
            { "resource": r.imaging_study },
        ],
    })
}

/// Where each linkage row lands in the bundle, for the link panel.
pub fn fhir_element(field: hl7kit::order::OrderField) -> &'static str {
    use hl7kit::order::OrderField;
    match field {
        OrderField::StudyUid => {
            "ImagingStudy.identifier[urn:dicom:uid], ServiceRequest.identifier[urn:dicom:uid]"
        }
        OrderField::Accession => "ImagingStudy.identifier[ACSN], ServiceRequest.identifier[ACSN]",
        OrderField::PatientId => "Patient.identifier[MR]",
        OrderField::ProcedureId => "ServiceRequest.identifier (untyped: carried, not standardised)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dicom::series::Slice;
    use crate::link;

    const UID: &str = "1.3.6.1.4.1.5962.1.2.4.20040826185059.5457";

    fn sample(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../samples/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn study() -> Study {
        Study {
            patient_id: Some("4MR1".into()),
            accession_number: Some("ACC-2026-0001".into()),
            study_uid: Some(UID.into()),
            requested_procedure_id: Some("RP-2026-0001".into()),
            transfer_syntax: "1.2.840.10008.1.2.1".into(),
            modality: Some("MR".into()),
            rows: 64,
            cols: 64,
            patient_name: Some("CompressedSamples^MR1".into()),
            patient_birth_date: None,
            patient_sex: Some("O".into()),
            study_date: Some("20040826".into()),
            study_time: Some("185059".into()),
            timezone_offset: None,
            study_description: Some("Head".into()),
        }
    }

    fn slice(file: usize, frame: u32, n: i32, uid: &str) -> Slice {
        Slice {
            file,
            frame,
            instance_number: Some(n),
            position: Some(n as f64),
            sop_instance_uid: uid.into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.1.4".into(),
        }
    }

    fn series() -> Vec<Series> {
        vec![
            Series {
                uid: "1.2.3.1".into(),
                number: Some(1),
                description: "Localizer".into(),
                modality: "MR".into(),
                rows: 64,
                cols: 64,
                slices: vec![slice(0, 0, 1, "1.2.3.1.1")],
            },
            Series {
                uid: "1.2.3.2".into(),
                number: Some(2),
                description: "Axial".into(),
                modality: "MR".into(),
                rows: 64,
                cols: 64,
                // Two instances, the second multi-frame with two frames.
                slices: vec![
                    slice(1, 0, 1, "1.2.3.2.1"),
                    slice(2, 0, 2, "1.2.3.2.2"),
                    slice(2, 1, 2, "1.2.3.2.2"),
                ],
            },
        ]
    }

    fn build(msg_text: &str, study: &Study) -> (Value, Linkage) {
        let msg = Message::parse(msg_text).unwrap();
        let order = Order::extract(&msg);
        let linkage = link::resolve(study, &order);
        let series = series();
        let input = FhirInput {
            study,
            series: &series,
            message: &msg,
            order: &order,
            linkage: &linkage,
        };
        (bundle(&input), linkage)
    }

    fn entry<'a>(b: &'a Value, resource_type: &str) -> &'a Value {
        b["entry"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| &e["resource"])
            .find(|r| r["resourceType"] == resource_type)
            .unwrap()
    }

    #[test]
    fn orm_bundle_links_and_populates_core_elements() {
        let (b, linkage) = build(&sample("order.hl7"), &study());
        assert_eq!(linkage.path, LinkPath::StudyUid);
        assert_eq!(b["resourceType"], "Bundle");
        assert_eq!(b["type"], "collection");
        assert_eq!(b["entry"].as_array().unwrap().len(), 3);

        let p = entry(&b, "Patient");
        assert_eq!(p["id"], "patient-1");
        assert_eq!(p["identifier"][0]["type"]["coding"][0]["code"], "MR");
        assert_eq!(p["identifier"][0]["value"], "4MR1");
        assert_eq!(
            p["identifier"][0]["assigner"]["display"], "HOSP",
            "from PID-3.4"
        );
        assert!(p["identifier"][0].get("system").is_none());
        assert_eq!(p["name"][0]["family"], "CompressedSamples");
        assert_eq!(p["name"][0]["given"], json!(["MR1"]));
        assert_eq!(p["gender"], "other");
        // DICOM had no birth date; PID-7 supplies it.
        assert_eq!(p["birthDate"], "1970-01-01");

        let sr = entry(&b, "ServiceRequest");
        assert_eq!(sr["subject"]["reference"], "Patient/patient-1");
        let ids = sr["identifier"].as_array().unwrap();
        assert!(ids
            .iter()
            .any(|i| i["system"] == "urn:dicom:uid" && i["value"] == format!("urn:oid:{UID}")));
        assert!(ids
            .iter()
            .any(|i| i["type"]["coding"][0]["code"] == "ACSN" && i["value"] == "ACC-2026-0001"));
        assert!(ids
            .iter()
            .any(|i| i["type"]["coding"][0]["code"] == "PLAC" && i["value"] == "ORD-2026-0001"));
        assert!(ids
            .iter()
            .any(|i| i["type"]["coding"][0]["code"] == "FILL" && i["value"] == "FIL-2026-0001"));
        assert!(ids
            .iter()
            .any(|i| i.get("type").is_none() && i["value"] == "RP-2026-0001"));
        assert!(ids.iter().all(|i| i
            .get("system")
            .map(|s| s.as_str() != Some(""))
            .unwrap_or(true)));
        assert_eq!(sr["code"]["text"], "MRI head without contrast");
        assert_eq!(
            sr["authoredOn"], "2026-09-05",
            "ORC-9 has no zone: date only"
        );

        let is = entry(&b, "ImagingStudy");
        let profile = is["meta"]["profile"][0].as_str().unwrap();
        assert_eq!(
            profile,
            format!(
                "{}|{}",
                ids::MII_IMAGING_STUDY_PROFILE,
                ids::MII_IMAGING_STUDY_VERSION
            )
        );
        assert_eq!(
            is["basedOn"][0]["reference"],
            "ServiceRequest/servicerequest-1"
        );
        assert_eq!(is["started"], "2004-08-26", "no (0008,0201): date only");
        assert_eq!(is["numberOfSeries"], 2);
        assert_eq!(is["numberOfInstances"], 3, "multi-frame counts once");
        assert_eq!(is["modality"][0]["code"], "MR");
        assert_eq!(is["modality"][0]["system"], ids::DCM);
        assert_eq!(is["series"][1]["numberOfInstances"], 2);
        assert_eq!(
            is["series"][1]["instance"][1]["sopClass"]["code"],
            "urn:oid:1.2.840.10008.5.1.4.1.1.4"
        );
        assert_eq!(
            is["series"][1]["instance"][1]["sopClass"]["system"],
            ids::RFC3986
        );
        assert_eq!(is["series"][1]["instance"][1]["number"], 2);
        assert_eq!(is["description"], "Head");
    }

    #[test]
    fn omi_bundle_reads_ipc_and_labels_the_source() {
        let (b, linkage) = build(&sample("order-omi.hl7"), &study());
        assert_eq!(linkage.path, LinkPath::StudyUid);
        let sr = entry(&b, "ServiceRequest");
        let ids = sr["identifier"].as_array().unwrap();
        assert!(ids.iter().any(|i| i["system"] == "urn:dicom:uid"));
        let acsn = ids
            .iter()
            .find(|i| i["type"]["coding"][0]["code"] == "ACSN")
            .unwrap();
        assert_eq!(acsn["value"], "ACC-2026-0003");
        assert_eq!(acsn["assigner"]["display"], "HOSP", "IPC-1.2 namespace");
        let msg = Message::parse(sample("order-omi.hl7")).unwrap();
        let order = Order::extract(&msg);
        assert_eq!(
            order.source_path(hl7kit::order::OrderField::StudyUid),
            Some("IPC-3.1")
        );
    }

    #[test]
    fn unlinked_study_has_no_based_on() {
        let other = Study {
            study_uid: Some("9.9.9".into()),
            accession_number: Some("OTHER".into()),
            ..study()
        };
        let (b, linkage) = build(&sample("order.hl7"), &other);
        assert_eq!(linkage.path, LinkPath::None);
        assert!(entry(&b, "ImagingStudy").get("basedOn").is_none());
    }

    #[test]
    fn started_carries_the_time_when_the_offset_is_known() {
        let with_zone = Study {
            timezone_offset: Some("+0200".into()),
            ..study()
        };
        let (b, _) = build(&sample("order.hl7"), &with_zone);
        assert_eq!(
            entry(&b, "ImagingStudy")["started"],
            "2004-08-26T18:50:59+02:00"
        );
    }

    #[test]
    fn gender_absent_and_unknown_are_omitted() {
        for sex in [None, Some("U".to_string())] {
            let s = Study {
                patient_sex: sex,
                ..study()
            };
            // The PID in the sample says `O`; DICOM wins when present, and
            // an absent DICOM value falls through to PID.
            let (b, _) = build(&sample("order.hl7"), &s);
            let p = entry(&b, "Patient");
            match s.patient_sex.as_deref() {
                None => assert_eq!(p["gender"], "other", "PID-8 fills in"),
                Some(_) => assert!(
                    p.get("gender").is_none(),
                    "DICOM `U` is omitted, not `unknown`"
                ),
            }
        }
    }

    #[test]
    fn fhir_elements_per_row() {
        use hl7kit::order::OrderField;
        assert!(fhir_element(OrderField::StudyUid).contains("urn:dicom:uid"));
        assert!(fhir_element(OrderField::ProcedureId).contains("not standardised"));
    }
}
