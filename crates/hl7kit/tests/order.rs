//! Order extraction: IPC-3 and ZDS-1 precedence, multi-IPC agreement, and
//! the fallbacks for accession and procedure ID in OMI^O23 messages.

use hl7kit::order::{Order, OrderField, StudyUidSource};
use hl7kit::{Message, Warning};

const ORM: &str = include_str!("fixtures/order.hl7");
const OMI: &str = include_str!("fixtures/order-omi.hl7");
const UID: &str = "1.3.6.1.4.1.5962.1.2.4.20040826185059.5457";

fn parse(text: &str) -> Message {
    Message::parse(text).expect("parses")
}

#[test]
fn omi_takes_study_uid_from_ipc3() {
    let msg = parse(OMI);
    let o = Order::extract(&msg);
    assert_eq!(o.study_uid.as_deref(), Some(UID));
    assert_eq!(o.study_uid_source, Some(StudyUidSource::Ipc3));
    assert_eq!(o.source_path(OrderField::StudyUid), Some("IPC-3.1"));
    // The recorded span is the IPC-3.1 component, inside the IPC segment.
    let (span, _) = o
        .spans
        .iter()
        .find(|(_, f)| *f == OrderField::StudyUid)
        .unwrap();
    assert_eq!(span.slice(msg.raw()), UID);
    let ipc = msg.segment("IPC").unwrap().span();
    assert!(span.start >= ipc.start && span.end <= ipc.end);
    assert!(o.warnings.is_empty());
}

#[test]
fn omi_accession_and_procedure_fall_back_to_ipc() {
    let o = Order::extract(&parse(OMI));
    assert_eq!(o.accession.as_deref(), Some("ACC-2026-0003"));
    assert_eq!(o.source_path(OrderField::Accession), Some("IPC-1.1"));
    assert_eq!(o.procedure_id.as_deref(), Some("RP-2026-0003"));
    assert_eq!(o.source_path(OrderField::ProcedureId), Some("IPC-2.1"));
    assert_eq!(o.patient_id.as_deref(), Some("4MR1"));
    assert_eq!(o.source_path(OrderField::PatientId), Some("PID-3.1"));
}

#[test]
fn orm_with_zds_is_unchanged() {
    let o = Order::extract(&parse(ORM));
    assert_eq!(o.study_uid.as_deref(), Some(UID));
    assert_eq!(o.study_uid_source, Some(StudyUidSource::Zds1));
    assert_eq!(o.source_path(OrderField::StudyUid), Some("ZDS-1.1"));
    assert_eq!(o.accession.as_deref(), Some("ACC-2026-0001"));
    assert_eq!(o.source_path(OrderField::Accession), Some("OBR-18"));
    assert_eq!(o.spans.len(), 4);
}

#[test]
fn ipc_wins_over_zds_when_both_are_present() {
    let text = format!("{OMI}ZDS|9.9.9^^Application^DICOM\r");
    let o = Order::extract(&parse(&text));
    assert_eq!(o.study_uid.as_deref(), Some(UID));
    assert_eq!(o.study_uid_source, Some(StudyUidSource::Ipc3));
}

#[test]
fn empty_ipc3_falls_through_to_zds() {
    let text = OMI.replace(&format!("|{UID}^HOSP|"), "||");
    assert!(!text.contains(UID));
    let text = format!("{text}ZDS|{UID}^^Application^DICOM\r");
    let o = Order::extract(&parse(&text));
    assert_eq!(o.study_uid.as_deref(), Some(UID));
    assert_eq!(o.study_uid_source, Some(StudyUidSource::Zds1));
}

#[test]
fn only_the_first_component_of_ipc3_is_the_uid() {
    // `1.2.3^HOSP^ISO` must not leak `HOSP` into the value.
    let text = OMI.replace(&format!("|{UID}^HOSP|"), "|1.2.3^HOSP^ISO|");
    let o = Order::extract(&parse(&text));
    assert_eq!(o.study_uid.as_deref(), Some("1.2.3"));
}

#[test]
fn identical_ipc3_across_procedure_steps_is_fine() {
    let text =
        format!("{OMI}IPC|ACC-2026-0003^HOSP|RP-2026-0003^HOSP|{UID}^HOSP|SPS-2026-0004^HOSP|MR\r");
    let o = Order::extract(&parse(&text));
    assert_eq!(o.study_uid.as_deref(), Some(UID));
    assert!(o.warnings.is_empty());
}

#[test]
fn differing_ipc3_takes_first_and_warns() {
    let text =
        format!("{OMI}IPC|ACC-2026-0003^HOSP|RP-2026-0003^HOSP|9.9.9^HOSP|SPS-2026-0004^HOSP|MR\r");
    let msg = parse(&text);
    let o = Order::extract(&msg);
    assert_eq!(o.study_uid.as_deref(), Some(UID), "first IPC wins");
    assert_eq!(o.warnings.len(), 1);
    match &o.warnings[0] {
        Warning::ConflictingStudyUid { first, other } => {
            assert_eq!(first.slice(msg.raw()), UID);
            assert_eq!(other.slice(msg.raw()), "9.9.9");
        }
        w => panic!("unexpected warning {w:?}"),
    }
    assert!(o.warnings[0].to_string().contains("study instance UID"));
}

#[test]
fn absent_everything_is_none() {
    let o = Order::extract(&parse("MSH|^~\\&|A|B|||||OMI^O23|1|P|2.5.1\rPID|1\r"));
    assert_eq!(o, Order::default());
    assert_eq!(o.source_path(OrderField::StudyUid), None);
}

const ORU: &str = include_str!("fixtures/order-oru.hl7");

#[test]
fn oru_takes_study_uid_from_the_dcm_110180_obx() {
    let msg = parse(ORU);
    let o = Order::extract(&msg);
    assert_eq!(o.study_uid.as_deref(), Some(UID));
    assert_eq!(o.study_uid_source, Some(StudyUidSource::Obx));
    assert!(o
        .source_path(OrderField::StudyUid)
        .unwrap()
        .starts_with("OBX-5"));
    let (span, _) = o
        .spans
        .iter()
        .find(|(_, f)| *f == OrderField::StudyUid)
        .unwrap();
    assert_eq!(span.slice(msg.raw()), UID);
    // The other OBX segments (series and instance counts) are ignored.
    assert_eq!(o.accession.as_deref(), Some("ACC-2026-0001"));
    assert_eq!(o.source_path(OrderField::Accession), Some("OBR-18"));
    assert!(o.warnings.is_empty());
}

#[test]
fn obx_matches_by_text_when_the_code_is_local() {
    let text = ORU.replace("110180^Study Instance UID^DCM", "SUID^Study Instance UID^L");
    let o = Order::extract(&parse(&text));
    assert_eq!(o.study_uid.as_deref(), Some(UID));
    assert_eq!(o.study_uid_source, Some(StudyUidSource::Obx));
    let none = ORU.replace("110180^Study Instance UID^DCM", "X^Something else^L");
    let o = Order::extract(&parse(&none));
    assert_eq!(o.study_uid, None);
    assert_eq!(o.study_uid_source, None);
}

#[test]
fn ipc_and_zds_win_over_obx_and_disagreement_warns() {
    let agree = format!("{ORU}ZDS|{UID}^^Application^DICOM\r");
    let o = Order::extract(&parse(&agree));
    assert_eq!(o.study_uid_source, Some(StudyUidSource::Zds1));
    assert!(o.warnings.is_empty());

    let disagree = format!("{ORU}ZDS|9.9.9^^Application^DICOM\r");
    let msg = parse(&disagree);
    let o = Order::extract(&msg);
    assert_eq!(o.study_uid.as_deref(), Some("9.9.9"), "ZDS precedes OBX");
    assert_eq!(o.warnings.len(), 1);
    match &o.warnings[0] {
        Warning::ConflictingStudyUid { first, other } => {
            assert_eq!(first.slice(msg.raw()), "9.9.9");
            assert_eq!(other.slice(msg.raw()), UID);
        }
        w => panic!("unexpected {w:?}"),
    }
}

#[test]
#[allow(deprecated)]
fn deprecated_path_is_the_first_candidate() {
    assert_eq!(OrderField::StudyUid.path(), "IPC-3.1");
    assert_eq!(OrderField::StudyUid.paths(), ["IPC-3.1", "ZDS-1.1"]);
}
