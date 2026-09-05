//! Generate an HL7 ORM^O01 that matches a DICOM study, using the `hl7v2`
//! builder. The output is what a RIS would have sent for the study, so
//! loading it next to the images exercises the linkage with a real study.
//!
//! Host only in practice (the CLI uses it), but it has no browser types, so
//! it is tested on the host like the rest of the domain code.

use crate::dicom::Study;
use hl7v2::builder::{Builder, Value};

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
        .set(9, Value::components(["ORM", "O01", "ORM_O01"]))
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

    if let Some(uid) = &study.study_uid {
        b.segment("ZDS").set(
            1,
            Value::components([uid.as_str(), "", "Application", "DICOM"]),
        );
    }
    b.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{self, LinkPath};
    use hl7v2::order::Order;
    use hl7v2::Message;

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
