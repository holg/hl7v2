//! End-to-end: fixture message in, worklist item out, checked attribute by
//! attribute against PS3.4 Table K.6-1 and IHE RAD-4.

use dicom_core::header::HasLength;
use dicom_core::Tag;
use dicom_dictionary_std::{tags, uids};
use dicom_object::file::ReadPreamble;
use dicom_object::{InMemDicomObject, OpenFileOptions};
use hl7kit::order::{Order, StudyUidSource};
use hl7kit::Message;
use mwlkit::{
    to_bytes, worklist_item, GeneratedFrom, Input, LengthPolicy, MwlError, Options, StudyUidOrigin,
    UidPolicy, Warning,
};
use std::io::Cursor;

const ORM_ZDS: &str = include_str!("fixtures/orm-zds.hl7");
const ORM_NO_UID: &str = include_str!("fixtures/orm-no-uid.hl7");
const OMI_ONE: &str = include_str!("fixtures/omi-one-ipc.hl7");
const OMI_TWO: &str = include_str!("fixtures/omi-two-ipc.hl7");
const ORM_LONG_ACC: &str = include_str!("fixtures/orm-long-accession.hl7");
const UID: &str = "1.3.6.1.4.1.5962.1.2.4.20040826185059.5457";

fn with_options(text: &str, options: &Options) -> Result<mwlkit::WorklistItem, MwlError> {
    let msg = Message::parse(text).expect("fixture parses");
    assert!(msg.warnings().is_empty(), "{:?}", msg.warnings());
    let order = Order::extract(&msg);
    worklist_item(
        &Input {
            message: &msg,
            order: &order,
        },
        options,
    )
}

fn defaulted_station() -> Options {
    Options {
        default_station_ae: Some("MRSCP".into()),
        ..Options::default()
    }
}

fn str_of(ds: &InMemDicomObject, tag: Tag) -> String {
    ds.get(tag)
        .unwrap_or_else(|| panic!("{tag} present"))
        .value()
        .to_str()
        .unwrap()
        .trim()
        .to_string()
}

fn sps(ds: &InMemDicomObject) -> Vec<InMemDicomObject> {
    ds.get(tags::SCHEDULED_PROCEDURE_STEP_SEQUENCE)
        .expect("SPS sequence")
        .items()
        .expect("SPS items")
        .to_vec()
}

#[test]
fn orm_with_zds_fills_every_type_1_attribute() {
    let item = with_options(ORM_ZDS, &defaulted_station()).unwrap();
    let ds = &item.dataset;
    assert_eq!(str_of(ds, tags::PATIENT_ID), "4MR1");
    assert_eq!(str_of(ds, tags::ISSUER_OF_PATIENT_ID), "HOSP");
    assert_eq!(str_of(ds, tags::PATIENT_NAME), "CompressedSamples^MR1");
    assert_eq!(str_of(ds, tags::PATIENT_BIRTH_DATE), "19700101");
    assert_eq!(str_of(ds, tags::PATIENT_SEX), "O");
    assert_eq!(str_of(ds, tags::ACCESSION_NUMBER), "ACC-2026-0001");
    assert_eq!(
        str_of(ds, tags::PLACER_ORDER_NUMBER_IMAGING_SERVICE_REQUEST),
        "ORD-2026-0001"
    );
    assert_eq!(
        str_of(ds, tags::FILLER_ORDER_NUMBER_IMAGING_SERVICE_REQUEST),
        "FIL-2026-0001"
    );
    assert_eq!(
        str_of(ds, tags::REFERRING_PHYSICIAN_NAME),
        "Curie^Marie^^Prof"
    );
    assert_eq!(str_of(ds, tags::REQUESTED_PROCEDURE_ID), "RP-2026-0001");
    assert_eq!(str_of(ds, tags::STUDY_INSTANCE_UID), UID);
    assert_eq!(
        str_of(ds, tags::REQUESTED_PROCEDURE_DESCRIPTION),
        "MRI head without contrast"
    );
    assert_eq!(str_of(ds, tags::REQUESTED_PROCEDURE_PRIORITY), "ROUTINE");
    assert_eq!(str_of(ds, tags::CURRENT_PATIENT_LOCATION), "RAD");
    assert!(ds
        .get(tags::REFERENCED_STUDY_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()
        .is_empty());
    assert!(
        ds.get(tags::INSTITUTION_NAME).is_none(),
        "MSH-4 is a code, not a name"
    );
    let code = ds
        .get(tags::REQUESTED_PROCEDURE_CODE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert_eq!(str_of(&code[0], tags::CODE_VALUE), "MR-HEAD");
    assert_eq!(str_of(&code[0], tags::CODING_SCHEME_DESIGNATOR), "L");

    let steps = sps(ds);
    assert_eq!(steps.len(), 1);
    let s = &steps[0];
    assert_eq!(str_of(s, tags::MODALITY), "MR");
    assert_eq!(str_of(s, tags::SCHEDULED_STATION_AE_TITLE), "MRSCP");
    assert_eq!(
        str_of(s, tags::SCHEDULED_PROCEDURE_STEP_START_DATE),
        "20260905"
    );
    assert_eq!(
        str_of(s, tags::SCHEDULED_PROCEDURE_STEP_START_TIME),
        "110000"
    );
    assert_eq!(
        str_of(s, tags::SCHEDULED_PROCEDURE_STEP_DESCRIPTION),
        "MRI head without contrast"
    );
    assert_eq!(
        str_of(s, tags::SCHEDULED_PROCEDURE_STEP_ID),
        "RP-2026-0001-1"
    );
    assert_eq!(
        str_of(s, tags::SCHEDULED_PROCEDURE_STEP_STATUS),
        "SCHEDULED"
    );
    assert!(
        s.get(tags::SCHEDULED_PERFORMING_PHYSICIAN_NAME)
            .unwrap()
            .is_empty(),
        "Type 2, empty"
    );

    assert_eq!(item.study_uid.value, UID);
    assert_eq!(
        item.study_uid.origin,
        StudyUidOrigin::FromOrder(StudyUidSource::Zds1)
    );
    // Every invented value is reported: the AE default, the SPS ID, the
    // character set.
    assert_eq!(
        item.warnings,
        [
            Warning::CharacterSetAssumed {
                msh18: None,
                used: "ISO_IR 192".into()
            },
            Warning::StationAeDefaulted("MRSCP".into()),
            Warning::MissingType1 {
                tag: tags::SCHEDULED_PROCEDURE_STEP_ID,
                substituted: Some("RP-2026-0001-1".into()),
            },
        ]
    );
}

#[test]
fn omi_takes_modality_station_and_sps_id_from_ipc() {
    let item = with_options(OMI_ONE, &Options::default()).unwrap();
    let ds = &item.dataset;
    assert_eq!(str_of(ds, tags::SPECIFIC_CHARACTER_SET), "ISO_IR 192");
    assert_eq!(str_of(ds, tags::ACCESSION_NUMBER), "ACC-2026-0003");
    assert_eq!(str_of(ds, tags::REQUESTED_PROCEDURE_ID), "RP-2026-0003");
    assert_eq!(
        str_of(ds, tags::REQUESTED_PROCEDURE_PRIORITY),
        "ROUTINE",
        "TQ1-9"
    );
    let steps = sps(ds);
    assert_eq!(steps.len(), 1);
    let s = &steps[0];
    assert_eq!(str_of(s, tags::MODALITY), "MR", "IPC-5");
    assert_eq!(
        str_of(s, tags::SCHEDULED_STATION_AE_TITLE),
        "MR1AE",
        "IPC-9"
    );
    assert_eq!(
        str_of(s, tags::SCHEDULED_PROCEDURE_STEP_ID),
        "SPS-2026-0003",
        "IPC-4"
    );
    assert_eq!(str_of(s, tags::SCHEDULED_STATION_NAME), "MR1", "IPC-7");
    assert_eq!(
        str_of(s, tags::SCHEDULED_PROCEDURE_STEP_LOCATION),
        "RAD-1",
        "IPC-8"
    );
    assert_eq!(
        str_of(s, tags::SCHEDULED_PROCEDURE_STEP_START_DATE),
        "20260905",
        "TQ1-7"
    );
    let protocol = s
        .get(tags::SCHEDULED_PROTOCOL_CODE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert_eq!(str_of(&protocol[0], tags::CODE_VALUE), "MR-HEAD", "IPC-6");
    assert_eq!(
        item.study_uid.origin,
        StudyUidOrigin::FromOrder(StudyUidSource::Ipc3)
    );
    assert!(item.warnings.is_empty(), "{:?}", item.warnings);
}

#[test]
fn two_ipc_segments_give_two_steps_under_one_study() {
    let item = with_options(OMI_TWO, &Options::default()).unwrap();
    let steps = sps(&item.dataset);
    assert_eq!(steps.len(), 2);
    assert_eq!(
        str_of(&steps[0], tags::SCHEDULED_PROCEDURE_STEP_ID),
        "SPS-2026-0004-1"
    );
    assert_eq!(
        str_of(&steps[1], tags::SCHEDULED_PROCEDURE_STEP_ID),
        "SPS-2026-0004-2"
    );
    let p1 = steps[1]
        .get(tags::SCHEDULED_PROTOCOL_CODE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert_eq!(str_of(&p1[0], tags::CODE_VALUE), "MR-HEAD-C");
    assert_eq!(str_of(&item.dataset, tags::STUDY_INSTANCE_UID), UID);
    assert!(item.warnings.is_empty(), "{:?}", item.warnings);
}

#[test]
fn missing_uid_is_derived_dcm4che_style_and_reported() {
    let item = with_options(ORM_NO_UID, &defaulted_station()).unwrap();
    let uid = &item.study_uid.value;
    assert_eq!(
        item.study_uid.origin,
        StudyUidOrigin::Generated(GeneratedFrom::RequestedProcedureId)
    );
    assert_eq!(
        uid,
        &mwlkit::uid::name_based_uid(mwlkit::uid::NAMESPACE, "rp|RP-2026-0002")
    );
    assert!(uid.starts_with("2.25."));
    assert!(uid.len() <= 44);
    assert!(uid[5..].bytes().all(|b| b.is_ascii_digit()));
    assert_eq!(str_of(&item.dataset, tags::STUDY_INSTANCE_UID), *uid);
    assert!(item.warnings.contains(&Warning::StudyUidGenerated(
        GeneratedFrom::RequestedProcedureId
    )));
    // Same order, same UID.
    let again = with_options(ORM_NO_UID, &defaulted_station()).unwrap();
    assert_eq!(again.study_uid.value, *uid);

    // Without a Requested Procedure ID the accession number is the name.
    let no_rp = ORM_NO_UID.replace("|RP-2026-0002|", "||");
    let item = with_options(&no_rp, &defaulted_station()).unwrap();
    assert_eq!(
        item.study_uid.origin,
        StudyUidOrigin::Generated(GeneratedFrom::AccessionNumber)
    );
    assert_ne!(item.study_uid.value, *uid);
    // ... and the placer order number stands in for the RP ID, reported.
    assert_eq!(
        str_of(&item.dataset, tags::REQUESTED_PROCEDURE_ID),
        "ORD-2026-0002"
    );
    assert!(item.warnings.iter().any(|w| matches!(
        w,
        Warning::MissingType1 { tag, substituted: Some(_) } if *tag == tags::REQUESTED_PROCEDURE_ID
    )));
}

#[test]
fn uid_policies_refuse_and_random() {
    let refuse = Options {
        uid_policy: UidPolicy::Refuse,
        ..defaulted_station()
    };
    assert_eq!(
        with_options(ORM_NO_UID, &refuse).unwrap_err(),
        MwlError::NoStudyUid
    );
    let random = Options {
        uid_policy: UidPolicy::Random(0x1234_5678_9abc_def0_1234_5678_9abc_def0),
        ..defaulted_station()
    };
    let item = with_options(ORM_NO_UID, &random).unwrap();
    assert_eq!(
        item.study_uid.origin,
        StudyUidOrigin::Generated(GeneratedFrom::Random)
    );
    assert_eq!(
        item.study_uid.value,
        format!("2.25.{}", 0x1234_5678_9abc_def0_1234_5678_9abc_def0_u128)
    );
    // An order that carries a UID ignores the policy.
    assert_eq!(with_options(ORM_ZDS, &refuse).unwrap().study_uid.value, UID);
}

#[test]
fn orm_without_ipc_translates_obr24_and_priority() {
    let item = with_options(ORM_NO_UID, &defaulted_station()).unwrap();
    let steps = sps(&item.dataset);
    assert_eq!(
        str_of(&steps[0], tags::MODALITY),
        "MR",
        "OBR-24 MRI is HL7 table 0074"
    );
    assert!(item.warnings.contains(&Warning::ModalityTranslated {
        from: "MRI".into(),
        to: "MR".into()
    }));
    assert_eq!(
        str_of(&item.dataset, tags::REQUESTED_PROCEDURE_PRIORITY),
        "STAT",
        "OBR-27.6 S"
    );

    let rad = ORM_NO_UID.replace("|MRI|", "|RAD|");
    assert_eq!(
        with_options(&rad, &defaulted_station()).unwrap_err(),
        MwlError::MissingModality {
            found: Some("RAD".into())
        },
        "RAD names no single DICOM modality"
    );
}

#[test]
fn hl7_name_prefix_and_suffix_are_reordered_for_pn() {
    let item = with_options(ORM_NO_UID, &defaulted_station()).unwrap();
    assert_eq!(
        str_of(&item.dataset, tags::PATIENT_NAME),
        "Doe^Jane^Marie^Dr^Jr"
    );
    assert!(item.warnings.contains(&Warning::NameComponentsReordered));
    assert_eq!(str_of(&item.dataset, tags::PATIENT_SEX), "F");
}

#[test]
fn long_accession_is_refused_by_default_and_truncated_on_request() {
    let err = with_options(ORM_LONG_ACC, &defaulted_station()).unwrap_err();
    assert_eq!(
        err,
        MwlError::TooLong {
            tag: tags::ACCESSION_NUMBER,
            max: 16,
            actual: 20
        }
    );

    let lenient = Options {
        length_policy: LengthPolicy::TruncateAndWarn,
        ..defaulted_station()
    };
    let item = with_options(ORM_LONG_ACC, &lenient).unwrap();
    assert_eq!(
        str_of(&item.dataset, tags::ACCESSION_NUMBER),
        "ACC-2026-0000000"
    );
    assert!(item.warnings.contains(&Warning::Truncated {
        tag: tags::ACCESSION_NUMBER,
        max: 16,
        actual: 20
    }));
}

#[test]
fn station_ae_title_is_never_truncated() {
    let long_ae = OMI_ONE.replace("|MR1AE\r", "|MR1AE_WAY_TOO_LONG\r");
    let lenient = Options {
        length_policy: LengthPolicy::TruncateAndWarn,
        ..Options::default()
    };
    assert_eq!(
        with_options(&long_ae, &lenient).unwrap_err(),
        MwlError::TooLong {
            tag: tags::SCHEDULED_STATION_AE_TITLE,
            max: 16,
            actual: 18
        }
    );
    let no_ae = OMI_ONE.replace("|MR1AE\r", "|\r");
    assert_eq!(
        with_options(&no_ae, &Options::default()).unwrap_err(),
        MwlError::MissingStationAe
    );
}

#[test]
fn type_1_attributes_are_refused_not_invented() {
    let no_pid = ORM_ZDS.replace("|4MR1^^^HOSP^MR|", "|^^^HOSP^MR|");
    assert_eq!(
        with_options(&no_pid, &defaulted_station()).unwrap_err(),
        MwlError::MissingPatientId
    );
    let no_start = ORM_ZDS.replace("|^^^20260905110000^^R\r", "|^^^^^R\r");
    assert_eq!(
        with_options(&no_start, &defaulted_station()).unwrap_err(),
        MwlError::MissingStartDateTime
    );
    let no_modality = ORM_ZDS.replace("|MR|||^^^", "||||^^^");
    assert_eq!(
        with_options(&no_modality, &defaulted_station()).unwrap_err(),
        MwlError::MissingModality { found: None }
    );
}

#[test]
fn patient_sex_u_is_empty_silently_and_unknown_codes_warn() {
    let u = ORM_ZDS.replace("|19700101|O\r", "|19700101|U\r");
    let item = with_options(&u, &defaulted_station()).unwrap();
    assert!(item.dataset.get(tags::PATIENT_SEX).unwrap().is_empty());
    assert!(!item
        .warnings
        .iter()
        .any(|w| matches!(w, Warning::SexMappedToOther { .. })));

    let x = ORM_ZDS.replace("|19700101|O\r", "|19700101|X\r");
    let item = with_options(&x, &defaulted_station()).unwrap();
    assert!(item.dataset.get(tags::PATIENT_SEX).unwrap().is_empty());
    assert!(item
        .warnings
        .contains(&Warning::SexMappedToOther { hl7: "X".into() }));
}

#[test]
fn written_file_reads_back_with_the_same_data_set() {
    let item = with_options(OMI_TWO, &Options::default()).unwrap();
    let bytes = to_bytes(&item).unwrap();
    assert_eq!(&bytes[128..132], b"DICM");
    let file = OpenFileOptions::new()
        .read_preamble(ReadPreamble::Always)
        .from_reader(Cursor::new(&bytes))
        .unwrap();
    assert_eq!(
        file.meta()
            .media_storage_sop_class_uid()
            .trim_end_matches('\0'),
        uids::MODALITY_WORKLIST_INFORMATION_MODEL_FIND
    );
    assert_eq!(
        file.meta().transfer_syntax().trim_end_matches('\0'),
        uids::EXPLICIT_VR_LITTLE_ENDIAN
    );
    assert!(file
        .meta()
        .media_storage_sop_instance_uid()
        .starts_with("2.25."));
    let read: &InMemDicomObject = &file;
    assert_eq!(str_of(read, tags::PATIENT_ID), "4MR1");
    assert_eq!(str_of(read, tags::STUDY_INSTANCE_UID), UID);
    assert_eq!(str_of(read, tags::ACCESSION_NUMBER), "ACC-2026-0004");
    let steps = sps(read);
    assert_eq!(steps.len(), 2);
    assert_eq!(str_of(&steps[1], tags::SCHEDULED_STATION_AE_TITLE), "MR1AE");
    assert_eq!(
        str_of(&steps[1], tags::SCHEDULED_PROCEDURE_STEP_ID),
        "SPS-2026-0004-2"
    );
    // Same item, same bytes.
    assert_eq!(to_bytes(&item).unwrap(), bytes);
}
