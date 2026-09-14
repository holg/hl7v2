//! Generate an HL7 ORM^O01 that matches a DICOM study, using the `hl7kit`
//! builder. The output is what a RIS would have sent for the study, so
//! loading it next to the images exercises the linkage with a real study.
//!
//! Host only in practice (the CLI uses it), but it has no browser types, so
//! it is tested on the host like the rest of the domain code.

use crate::dicom::Study;
use hl7kit::builder::{Builder, Value};

/// Everything the message needs beyond [`Study`]. All optional; blanks stay
/// blank so the message is honest about what the study carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderDetails {
    /// Override for PID-3.1. Use a different value to produce the
    /// linked-but-patient-mismatch case.
    pub patient_id: Option<String>,
    /// Override for OBR-18. Anonymised studies usually have none.
    pub accession: Option<String>,
    /// Override for OBR-19.
    pub procedure_id: Option<String>,
    /// `Last^First`, PID-5.
    pub patient_name: Option<String>,
    /// `YYYYMMDD`, PID-7.
    pub birth_date: Option<String>,
    /// PID-8.
    pub sex: Option<String>,
    /// `YYYYMMDDHHMMSS`, OBR-7 and MSH-7.
    pub study_datetime: Option<String>,
    /// OBR-4.2, e.g. the Study Description or Protocol Name.
    pub procedure_description: Option<String>,
    /// MSH-10.
    pub control_id: Option<String>,
    /// Sending application and facility, MSH-3 and MSH-4.
    pub sending: Option<(String, String)>,
    /// Emit an OMI^O23 with IPC segments instead of an ORM^O01 with ZDS.
    pub omi: bool,
    /// Number of IPC segments (scheduled procedure steps) in an OMI; 0 and
    /// 1 both mean one. Ignored for ORM.
    pub steps: usize,
    /// Leave the Study Instance UID out (no ZDS, empty IPC-3) so the
    /// worklist has to generate one.
    pub no_uid: bool,
}

/// Build the message text (CR-terminated segments).
pub fn order_message(study: &Study, details: &OrderDetails) -> String {
    let patient_id = details
        .patient_id
        .clone()
        .or_else(|| study.patient_id.clone())
        .unwrap_or_default();
    let accession = details
        .accession
        .clone()
        .or_else(|| study.accession_number.clone())
        .unwrap_or_default();
    let procedure_id = details
        .procedure_id
        .clone()
        .or_else(|| study.requested_procedure_id.clone())
        .unwrap_or_default();
    let modality = study.modality.clone().unwrap_or_default();
    let datetime = details.study_datetime.clone().unwrap_or_default();
    let (app, facility) = details
        .sending
        .clone()
        .unwrap_or_else(|| ("RIS".into(), "HOSP".into()));
    let control_id = details
        .control_id
        .clone()
        .unwrap_or_else(|| "MSG0001".into());

    let mut b = Builder::new();
    b.segment("MSH")
        .set(3, app.as_str())
        .set(4, facility.as_str())
        .set(5, "PACS")
        .set(6, facility.as_str())
        .set(7, datetime.as_str())
        .set(
            9,
            if details.omi {
                Value::components(["OMI", "O23", "OMI_O23"])
            } else {
                Value::components(["ORM", "O01", "ORM_O01"])
            },
        )
        .set(10, control_id.as_str())
        .set(11, "P")
        .set(12, "2.5.1");

    let pid = b.segment("PID");
    pid.set(1, "1").set(
        3,
        Value::components([patient_id.as_str(), "", "", facility.as_str(), "MR"]),
    );
    if let Some(name) = &details.patient_name {
        pid.set(5, Value::components(name.split('^')));
    }
    if let Some(dob) = &details.birth_date {
        pid.set(7, dob.as_str());
    }
    if let Some(sex) = &details.sex {
        pid.set(8, sex.as_str());
    }

    b.segment("PV1").set(1, "1").set(2, "O");

    let placer = if accession.is_empty() {
        "ORD-0001".to_string()
    } else {
        format!("ORD-{accession}")
    };
    b.segment("ORC")
        .set(1, "NW")
        .set(2, placer.as_str())
        .set(5, "SC")
        .set(9, datetime.as_str());

    let obr = b.segment("OBR");
    obr.set(1, "1").set(2, placer.as_str());
    let mut procedure = Value::components([procedure_id.as_str()]);
    if let Some(desc) = &details.procedure_description {
        procedure.set(1, 2, 1, desc.as_str());
        procedure.set(1, 3, 1, "L");
    }
    obr.set(4, procedure)
        .set(7, datetime.as_str())
        .set(18, accession.as_str())
        .set(19, procedure_id.as_str())
        .set(24, modality.as_str());
    // OBR-27 Quantity/Timing: start date/time in component 4, priority in
    // 6. This is what a worklist item takes its SPS start from.
    let mut timing = Value::components(["", "", "", datetime.as_str(), "", "R"]);
    if datetime.is_empty() {
        timing = Value::components(["", "", "", "", "", "R"]);
    }
    obr.set(27, timing);

    let uid = if details.no_uid {
        None
    } else {
        study.study_uid.as_deref()
    };
    if details.omi {
        // TQ1-7 start, TQ1-9 priority: the 2.5 form of OBR-27.
        b.segment("TQ1")
            .set(1, "1")
            .set(7, datetime.as_str())
            .set(9, "R");
        let steps = details.steps.max(1);
        for n in 1..=steps {
            let sps = if procedure_id.is_empty() {
                format!("SPS-{n}")
            } else {
                format!("SPS-{procedure_id}-{n}")
            };
            let ipc = b.segment("IPC");
            ipc.set(
                1,
                Value::components([accession.as_str(), facility.as_str()]),
            )
            .set(
                2,
                Value::components([procedure_id.as_str(), facility.as_str()]),
            )
            .set(3, Value::components([uid.unwrap_or(""), facility.as_str()]))
            .set(4, Value::components([sps.as_str(), facility.as_str()]))
            .set(5, modality.as_str());
            if let Some(desc) = &details.procedure_description {
                ipc.set(
                    6,
                    Value::components([procedure_id.as_str(), desc.as_str(), "L"]),
                );
            }
        }
    } else if let Some(uid) = uid {
        b.segment("ZDS")
            .set(1, Value::components([uid, "", "Application", "DICOM"]));
    }
    b.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{self, LinkPath};
    use hl7kit::order::Order;
    use hl7kit::Message;

    fn study() -> Study {
        Study {
            patient_id: Some("0".into()),
            accession_number: None,
            study_uid: Some(
                "1.3.6.1.4.1.44316.6.102.1.20250704114423696.61158672119535771932".into(),
            ),
            requested_procedure_id: None,
            transfer_syntax: "1.2.840.10008.1.2".into(),
            modality: Some("CT".into()),
            rows: 512,
            cols: 512,
            ..Study::default()
        }
    }

    #[test]
    fn generated_order_links_by_study_uid() {
        let text = order_message(
            &study(),
            &OrderDetails {
                patient_name: Some("Anonymized^^".into()),
                sex: Some("M".into()),
                study_datetime: Some("20151207073153".into()),
                procedure_description: Some("KUNAS | dvi fazes".into()),
                ..OrderDetails::default()
            },
        );
        assert!(text.ends_with('\r') && !text.contains('\n'));
        let msg = Message::parse(&text).unwrap();
        assert!(msg.warnings().is_empty());
        assert_eq!(msg.message_type().unwrap().code, "ORM");
        assert_eq!(msg.get("MSH-7"), Some("20151207073153"));
        assert_eq!(msg.get("PID-3.1"), Some("0"));
        assert_eq!(msg.get("PID-8"), Some("M"));
        assert_eq!(msg.get("OBR-18"), Some(""));
        assert_eq!(msg.get("OBR-24"), Some("CT"));
        assert_eq!(msg.get_decoded("OBR-4.2").unwrap(), "KUNAS | dvi fazes");

        let order = Order::extract(&msg);
        assert_eq!(order.accession, None, "blank accession stays blank");
        let linkage = link::resolve(&study(), &order);
        assert_eq!(linkage.path, LinkPath::StudyUid);
        assert_eq!(linkage.patient_match, Some(true));
    }

    /// The library ships its own copy of `samples/order.hl7` so its tests
    /// work from the packaged crate; the two must not drift apart.
    #[test]
    fn sample_order_matches_the_crate_fixture() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        for name in ["order.hl7", "order-omi.hl7", "order-oru.hl7"] {
            let sample = std::fs::read(format!("{root}/samples/{name}")).unwrap();
            let fixture =
                std::fs::read(format!("{root}/crates/hl7kit/tests/fixtures/{name}")).unwrap();
            assert_eq!(
                sample, fixture,
                "samples/{name} differs from crates/hl7kit/tests/fixtures/{name}"
            );
        }
    }

    #[test]
    fn patient_override_produces_the_mismatch_case() {
        let text = order_message(
            &study(),
            &OrderDetails {
                patient_id: Some("9ZZ9".into()),
                ..OrderDetails::default()
            },
        );
        let msg = Message::parse(&text).unwrap();
        let linkage = link::resolve(&study(), &Order::extract(&msg));
        assert_eq!(linkage.path, LinkPath::StudyUid);
        assert!(linkage.linked_with_patient_mismatch());
    }

    #[test]
    fn accession_override_enables_the_fallback_path() {
        let text = order_message(
            &Study {
                study_uid: None,
                accession_number: Some("ACC-1".into()),
                ..study()
            },
            &OrderDetails::default(),
        );
        let msg = Message::parse(&text).unwrap();
        assert!(msg.segment("ZDS").is_none());
        assert_eq!(msg.get("ORC-2"), Some("ORD-ACC-1"));
        let linkage = link::resolve(
            &Study {
                study_uid: None,
                accession_number: Some("ACC-1".into()),
                ..study()
            },
            &Order::extract(&msg),
        );
        assert_eq!(linkage.path, LinkPath::Accession);
    }
}

#[cfg(test)]
mod worklist_tests {
    use super::*;
    use crate::worklist;
    use hl7kit::order::Order;
    use hl7kit::Message;
    use mwlkit::{GeneratedFrom, StudyUidOrigin};

    fn study() -> Study {
        Study {
            patient_id: Some("0".into()),
            accession_number: Some("ACC-9".into()),
            study_uid: Some("1.2.3.4".into()),
            requested_procedure_id: Some("RP-9".into()),
            modality: Some("CT".into()),
            ..Study::default()
        }
    }
    fn details() -> OrderDetails {
        OrderDetails {
            study_datetime: Some("20151207073153".into()),
            procedure_description: Some("CT abdomen".into()),
            ..OrderDetails::default()
        }
    }
    fn item(text: &str) -> worklist::WorklistOutput {
        let msg = Message::parse(text).unwrap();
        assert!(msg.warnings().is_empty(), "{:?}", msg.warnings());
        let order = Order::extract(&msg);
        worklist::build(&msg, &order).unwrap()
    }

    #[test]
    fn generated_orm_carries_a_start_time_so_it_becomes_a_worklist_item() {
        let text = order_message(&study(), &details());
        let msg = Message::parse(&text).unwrap();
        assert_eq!(msg.get("OBR-27.4"), Some("20151207073153"));
        let out = item(&text);
        assert_eq!(
            out.item.study_uid.origin,
            StudyUidOrigin::FromOrder(hl7kit::order::StudyUidSource::Zds1)
        );
    }

    #[test]
    fn omi_variant_has_one_ipc_per_step() {
        let two = OrderDetails {
            omi: true,
            steps: 2,
            ..details()
        };
        let text = order_message(&study(), &two);
        let msg = Message::parse(&text).unwrap();
        assert_eq!(msg.message_type().unwrap().code, "OMI");
        assert_eq!(msg.segments_named("IPC").count(), 2);
        assert!(msg.segment("ZDS").is_none());
        assert_eq!(msg.get("IPC-3.1"), Some("1.2.3.4"));
        let out = item(&text);
        assert_eq!(
            out.item.study_uid.origin,
            StudyUidOrigin::FromOrder(hl7kit::order::StudyUidSource::Ipc3)
        );
        let sps = out
            .item
            .dataset
            .get(dicom_dictionary_std::tags::SCHEDULED_PROCEDURE_STEP_SEQUENCE)
            .unwrap()
            .items()
            .unwrap();
        assert_eq!(sps.len(), 2);
    }

    #[test]
    fn no_uid_variant_makes_the_worklist_generate_one() {
        let text = order_message(
            &study(),
            &OrderDetails {
                no_uid: true,
                ..details()
            },
        );
        let msg = Message::parse(&text).unwrap();
        assert!(msg.segment("ZDS").is_none());
        let out = item(&text);
        assert_eq!(
            out.item.study_uid.origin,
            StudyUidOrigin::Generated(GeneratedFrom::RequestedProcedureId)
        );

        let text = order_message(
            &study(),
            &OrderDetails {
                no_uid: true,
                omi: true,
                ..details()
            },
        );
        let msg = Message::parse(&text).unwrap();
        assert_eq!(msg.get("IPC-3.1"), Some(""));
        let out = item(&text);
        assert!(matches!(
            out.item.study_uid.origin,
            StudyUidOrigin::Generated(_)
        ));
    }
}
