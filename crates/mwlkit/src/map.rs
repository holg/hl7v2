//! The mapping table, one function per DICOM module of the Modality
//! Worklist Information Model (PS3.4 Table K.6-1).
//!
//! Sources, cited per line as `IHE` (IHE Radiology Technical Framework
//! Volume 2, RAD-4 "Procedure Scheduled", HL7-to-DICOM mapping tables in
//! §4.4), `dcm4che` (dcm4chee-arc's HL7-to-MWL mapping), `PS3.x` (the DICOM
//! standard part) or `unsourced` (a choice of this crate, explained).
//! Attribute types (1, 2, 3) are those of Table K.6-1.

use crate::input::{Input, Options};
use crate::limits::{self, bounded, strict};
use crate::pn;
use crate::uid;
use crate::{MwlError, StudyUid, Warning, WorklistItem};
use dicom_core::value::DataSetSequence;
use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use hl7kit::{Message, Segment};

/// Build the worklist item for one order.
pub fn worklist_item(input: &Input<'_>, options: &Options) -> Result<WorklistItem, MwlError> {
    let msg = input.message;
    let mut ds = InMemDicomObject::new_empty();
    let mut warnings = Vec::new();

    character_set(&mut ds, msg, options, &mut warnings);
    patient(&mut ds, msg, options, &mut warnings)?;
    imaging_service_request(&mut ds, input, options, &mut warnings)?;
    let study_uid = requested_procedure(&mut ds, input, options, &mut warnings)?;
    scheduled_procedure_steps(&mut ds, input, options, &mut warnings)?;
    visit(&mut ds, msg);
    // One note per distinct decision: the station default repeats per SPS.
    warnings.dedup();

    Ok(WorklistItem {
        dataset: ds,
        study_uid,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Data set level
// ---------------------------------------------------------------------------

/// (0008,0005) Specific Character Set. IHE: from MSH-18. Unknown or absent
/// values become UTF-8 with a warning; `ASCII` means the default repertoire
/// and no attribute at all (PS3.5 6.1.2.5).
fn character_set(
    ds: &mut InMemDicomObject,
    msg: &Message,
    options: &Options,
    w: &mut Vec<Warning>,
) {
    if let Some(cs) = &options.character_set {
        put_str(ds, tags::SPECIFIC_CHARACTER_SET, VR::CS, cs);
        return;
    }
    let msh18 = field(msg, "MSH-18.1");
    let mapped = msh18.as_deref().map(|v| v.trim().to_ascii_uppercase());
    let term = match mapped.as_deref() {
        Some("ASCII") => return,
        Some("8859/1") => Some("ISO_IR 100"),
        Some("8859/2") => Some("ISO_IR 101"),
        Some("8859/5") => Some("ISO_IR 144"),
        Some("8859/15") => Some("ISO_IR 203"),
        Some("UNICODE UTF-8") | Some("UNICODE") | Some("UTF-8") => Some("ISO_IR 192"),
        _ => None,
    };
    match term {
        Some(t) => put_str(ds, tags::SPECIFIC_CHARACTER_SET, VR::CS, t),
        None => {
            w.push(Warning::CharacterSetAssumed {
                msh18,
                used: "ISO_IR 192".into(),
            });
            put_str(ds, tags::SPECIFIC_CHARACTER_SET, VR::CS, "ISO_IR 192");
        }
    }
}

// ---------------------------------------------------------------------------
// Patient Identification and Patient Demographic modules
// ---------------------------------------------------------------------------

fn patient(
    ds: &mut InMemDicomObject,
    msg: &Message,
    options: &Options,
    w: &mut Vec<Warning>,
) -> Result<(), MwlError> {
    // (0010,0020) Patient ID, LO, Type 1. IHE: PID-3.1. No substitute.
    let id = field(msg, "PID-3.1").ok_or(MwlError::MissingPatientId)?;
    let id = bounded(tags::PATIENT_ID, limits::LO, id, options.length_policy, w)?;
    put_str(ds, tags::PATIENT_ID, VR::LO, &id);
    // (0010,0021) Issuer of Patient ID, LO, Type 3. IHE: PID-3.4 (assigning authority namespace).
    if let Some(issuer) = field(msg, "PID-3.4") {
        let issuer = bounded(
            tags::ISSUER_OF_PATIENT_ID,
            limits::LO,
            issuer,
            options.length_policy,
            w,
        )?;
        put_str(ds, tags::ISSUER_OF_PATIENT_ID, VR::LO, &issuer);
    }

    // (0010,0010) Patient's Name, PN, Type 2. IHE: PID-5; component order per pn.rs.
    match field(msg, "PID-5").and_then(|v| pn::from_xpn(&v)) {
        Some(name) => {
            if name.reordered {
                w.push(Warning::NameComponentsReordered);
            }
            let pn = bounded(
                tags::PATIENT_NAME,
                limits::PN,
                name.pn,
                options.length_policy,
                w,
            )?;
            put_str(ds, tags::PATIENT_NAME, VR::PN, &pn);
        }
        None => put_empty(ds, tags::PATIENT_NAME, VR::PN),
    }

    // (0010,0030) Patient's Birth Date, DA, Type 2. IHE: PID-7, date part only.
    match field(msg, "PID-7").and_then(|v| hl7_date(&v)) {
        Some(d) => put_str(ds, tags::PATIENT_BIRTH_DATE, VR::DA, &d),
        None => put_empty(ds, tags::PATIENT_BIRTH_DATE, VR::DA),
    }

    // (0010,0040) Patient's Sex, CS, Type 2. IHE: PID-8; M/F/O pass, U and
    // absent are empty without comment, anything else empty with a warning
    // (HL7 table 0001 also has A ambiguous and N not applicable; DICOM has
    // no equivalent).
    let sex = field(msg, "PID-8").map(|s| s.to_ascii_uppercase());
    match sex.as_deref() {
        Some("M") | Some("F") | Some("O") => {
            put_str(ds, tags::PATIENT_SEX, VR::CS, sex.as_deref().unwrap_or(""));
        }
        Some("U") | None => put_empty(ds, tags::PATIENT_SEX, VR::CS),
        Some(other) => {
            w.push(Warning::SexMappedToOther {
                hl7: other.to_string(),
            });
            put_empty(ds, tags::PATIENT_SEX, VR::CS);
        }
    }
    // (0010,1030) Patient's Weight from an OBX with LOINC 29463-7, and
    // (0010,2000)/(0010,2110) Medical Alerts/Allergies from AL1: Type 3,
    // not mapped in this version.
    Ok(())
}

// ---------------------------------------------------------------------------
// Imaging Service Request module
// ---------------------------------------------------------------------------

fn imaging_service_request(
    ds: &mut InMemDicomObject,
    input: &Input<'_>,
    options: &Options,
    w: &mut Vec<Warning>,
) -> Result<(), MwlError> {
    let msg = input.message;
    // (0008,0050) Accession Number, SH, Type 2. IHE: OBR-18 (Placer Field 1);
    // hl7kit falls back to IPC-1.1. SH allows 16 characters; longer RIS
    // accession numbers follow the length policy.
    match &input.order.accession {
        Some(acc) => {
            let acc = bounded(
                tags::ACCESSION_NUMBER,
                limits::SH,
                acc.clone(),
                options.length_policy,
                w,
            )?;
            put_str(ds, tags::ACCESSION_NUMBER, VR::SH, &acc);
        }
        None => put_empty(ds, tags::ACCESSION_NUMBER, VR::SH),
    }
    // (0040,2016) Placer Order Number, LO, Type 3. IHE: ORC-2.1.
    if let Some(v) = field(msg, "ORC-2.1") {
        let v = bounded(
            tags::PLACER_ORDER_NUMBER_IMAGING_SERVICE_REQUEST,
            limits::LO,
            v,
            options.length_policy,
            w,
        )?;
        put_str(
            ds,
            tags::PLACER_ORDER_NUMBER_IMAGING_SERVICE_REQUEST,
            VR::LO,
            &v,
        );
    }
    // (0040,2017) Filler Order Number, LO, Type 3. IHE: ORC-3.1.
    if let Some(v) = field(msg, "ORC-3.1") {
        let v = bounded(
            tags::FILLER_ORDER_NUMBER_IMAGING_SERVICE_REQUEST,
            limits::LO,
            v,
            options.length_policy,
            w,
        )?;
        put_str(
            ds,
            tags::FILLER_ORDER_NUMBER_IMAGING_SERVICE_REQUEST,
            VR::LO,
            &v,
        );
    }
    // (0008,0090) Referring Physician's Name, PN, Type 2. IHE: PV1-8
    // (Referring Doctor); dcm4che also accepts ORC-12 when PV1-8 is absent.
    let referring = field(msg, "PV1-8")
        .or_else(|| field(msg, "ORC-12"))
        .and_then(|v| pn::from_xcn(&v));
    match referring {
        Some(name) => {
            let v = bounded(
                tags::REFERRING_PHYSICIAN_NAME,
                limits::PN,
                name.pn,
                options.length_policy,
                w,
            )?;
            put_str(ds, tags::REFERRING_PHYSICIAN_NAME, VR::PN, &v);
        }
        None => put_empty(ds, tags::REFERRING_PHYSICIAN_NAME, VR::PN),
    }
    // (0032,1032) Requesting Physician, PN, Type 3. IHE: OBR-16 (Ordering
    // Provider); ORC-12 carries the same provider.
    if let Some(name) = field(msg, "OBR-16")
        .or_else(|| field(msg, "ORC-12"))
        .and_then(|v| pn::from_xcn(&v))
    {
        let v = bounded(
            tags::REQUESTING_PHYSICIAN,
            limits::PN,
            name.pn,
            options.length_policy,
            w,
        )?;
        put_str(ds, tags::REQUESTING_PHYSICIAN, VR::PN, &v);
    }
    // (0008,0080) Institution Name: MSH-4 is a facility code, not a name;
    // Type 3, omitted (unsourced decision: nothing in the message says
    // whether MSH-4 is human-readable).
    // (0040,2400) Imaging Service Request Comments from NTE: Type 3, not
    // mapped in this version.
    Ok(())
}

// ---------------------------------------------------------------------------
// Requested Procedure module
// ---------------------------------------------------------------------------

fn requested_procedure(
    ds: &mut InMemDicomObject,
    input: &Input<'_>,
    options: &Options,
    w: &mut Vec<Warning>,
) -> Result<StudyUid, MwlError> {
    let msg = input.message;
    // (0040,1001) Requested Procedure ID, SH, Type 1. IHE: OBR-19 (Placer
    // Field 2); hl7kit falls back to IPC-2.1. When both are absent the
    // placer order number stands in, and the substitution is reported
    // (dcm4che does the same).
    let rp_id = match &input.order.procedure_id {
        Some(v) => v.clone(),
        None => {
            let placer = field(msg, "ORC-2.1").or_else(|| field(msg, "OBR-2.1"));
            w.push(Warning::MissingType1 {
                tag: tags::REQUESTED_PROCEDURE_ID,
                substituted: placer
                    .as_ref()
                    .map(|p| format!("placer order number {p} (ORC-2.1)")),
            });
            placer.ok_or(MwlError::MissingRequestedProcedureId)?
        }
    };
    let rp_id = bounded(
        tags::REQUESTED_PROCEDURE_ID,
        limits::SH,
        rp_id,
        options.length_policy,
        w,
    )?;
    put_str(ds, tags::REQUESTED_PROCEDURE_ID, VR::SH, &rp_id);

    // (0020,000D) Study Instance UID, UI, Type 1. See uid.rs.
    let (value, origin, warning) = uid::resolve(input.order, &options.uid_policy)?;
    let value = strict(tags::STUDY_INSTANCE_UID, limits::UI, value)?;
    if let Some(warning) = warning {
        w.push(warning);
    }
    put_str(ds, tags::STUDY_INSTANCE_UID, VR::UI, &value);

    // (0032,1060) Requested Procedure Description, LO. IHE: OBR-4.2. One of
    // description or code sequence is required.
    if let Some(desc) = field(msg, "OBR-4.2") {
        let desc = bounded(
            tags::REQUESTED_PROCEDURE_DESCRIPTION,
            limits::LO,
            desc,
            options.length_policy,
            w,
        )?;
        put_str(ds, tags::REQUESTED_PROCEDURE_DESCRIPTION, VR::LO, &desc);
    }
    // (0032,1064) Requested Procedure Code Sequence, SQ. IHE: OBR-4.1 code,
    // OBR-4.2 meaning, OBR-4.3 coding scheme. Only when OBR-4.3 is present:
    // a coding scheme designator is never invented.
    if let Some(item) = code_item(msg, "OBR-4", options, w)? {
        put_seq(ds, tags::REQUESTED_PROCEDURE_CODE_SEQUENCE, vec![item]);
    }
    // (0040,1003) Requested Procedure Priority, CS. IHE: OBR-27.6, else
    // TQ1-9 (HL7 table 0485). S→STAT, A→HIGH, R→ROUTINE per IHE. T
    // (timing critical)→HIGH and P (preop)→MEDIUM are unsourced choices
    // of this crate; anything else is omitted.
    let priority = field(msg, "OBR-27.6").or_else(|| field(msg, "TQ1-9.1"));
    if let Some(p) = priority.map(|p| p.to_ascii_uppercase()) {
        let dicom = match p.as_str() {
            "S" => Some("STAT"),
            "A" | "T" => Some("HIGH"),
            "R" => Some("ROUTINE"),
            "P" => Some("MEDIUM"),
            _ => None,
        };
        if let Some(d) = dicom {
            put_str(ds, tags::REQUESTED_PROCEDURE_PRIORITY, VR::CS, d);
        }
    }
    // (0008,1110) Referenced Study Sequence, SQ, Type 2: present and empty.
    ds.put(DataElement::new(
        tags::REFERENCED_STUDY_SEQUENCE,
        VR::SQ,
        DataSetSequence::<InMemDicomObject>::empty(),
    ));

    Ok(StudyUid { value, origin })
}

// ---------------------------------------------------------------------------
// Scheduled Procedure Step module: (0040,0100), one item per IPC segment,
// or one item from ORC/OBR/TQ1 when the message has no IPC (ORM^O01).
// ---------------------------------------------------------------------------

fn scheduled_procedure_steps(
    ds: &mut InMemDicomObject,
    input: &Input<'_>,
    options: &Options,
    w: &mut Vec<Warning>,
) -> Result<(), MwlError> {
    let msg = input.message;
    let ipcs: Vec<Segment<'_>> = msg.segments_named("IPC").collect();
    let mut items = Vec::new();
    if ipcs.is_empty() {
        items.push(sps_item(input, None, 1, options, w)?);
    } else {
        for (i, ipc) in ipcs.iter().enumerate() {
            items.push(sps_item(input, Some(ipc), i + 1, options, w)?);
        }
    }
    put_seq(ds, tags::SCHEDULED_PROCEDURE_STEP_SEQUENCE, items);
    Ok(())
}

fn sps_item(
    input: &Input<'_>,
    ipc: Option<&Segment<'_>>,
    index: usize,
    options: &Options,
    w: &mut Vec<Warning>,
) -> Result<InMemDicomObject, MwlError> {
    let msg = input.message;
    let ipc_field = |n: usize, c: usize| -> Option<String> {
        let seg = ipc?;
        let comp = seg.field(n)?.component(c)?;
        let v = msg.decode(comp.value()).trim().to_string();
        (!v.is_empty()).then_some(v)
    };
    let mut item = InMemDicomObject::new_empty();

    // (0008,0060) Modality, CS, Type 1. IHE: IPC-5 for OMI, OBR-24
    // (Diagnostic Service Section ID, HL7 table 0074) for ORM. Table 0074
    // codes are not DICOM modality codes; the few that map are translated
    // and the translation reported, anything else is refused. Never guessed
    // from the procedure text.
    let modality = match ipc_field(5, 1) {
        Some(m) => modality_code(&m, w).ok_or(MwlError::MissingModality { found: Some(m) })?,
        None => {
            let obr24 = field(msg, "OBR-24");
            match &obr24 {
                Some(v) => modality_code(v, w).ok_or(MwlError::MissingModality {
                    found: obr24.clone(),
                })?,
                None => return Err(MwlError::MissingModality { found: None }),
            }
        }
    };
    put_str(&mut item, tags::MODALITY, VR::CS, &modality);

    // (0040,0001) Scheduled Station AE Title, AE, Type 1. IHE: IPC-9. AE is
    // 16 characters, never truncated; when absent the caller's default is
    // written and reported.
    let ae = match ipc_field(9, 1) {
        Some(ae) => ae,
        None => {
            let ae = options
                .default_station_ae
                .clone()
                .ok_or(MwlError::MissingStationAe)?;
            w.push(Warning::StationAeDefaulted(ae.clone()));
            ae
        }
    };
    let ae = strict(
        tags::SCHEDULED_STATION_AE_TITLE,
        limits::AE,
        ae.trim().to_string(),
    )?;
    put_str(&mut item, tags::SCHEDULED_STATION_AE_TITLE, VR::AE, &ae);

    // (0040,0002)/(0040,0003) SPS Start Date and Time, DA/TM, Type 1. IHE:
    // OBR-27.4 (Quantity/Timing start), else TQ1-7 in 2.5+ messages.
    let start = field(msg, "OBR-27.4").or_else(|| field(msg, "TQ1-7"));
    let (date, time) = match start.as_deref().map(|s| (hl7_date(s), hl7_time(s))) {
        Some((Some(d), Some(t))) => (d, t),
        _ => return Err(MwlError::MissingStartDateTime),
    };
    put_str(
        &mut item,
        tags::SCHEDULED_PROCEDURE_STEP_START_DATE,
        VR::DA,
        &date,
    );
    put_str(
        &mut item,
        tags::SCHEDULED_PROCEDURE_STEP_START_TIME,
        VR::TM,
        &time,
    );

    // (0040,0006) Scheduled Performing Physician's Name, PN, Type 2. IHE:
    // OBR-34 (Technician, NDL: the name is a CNN in component 1 with
    // subcomponents id&family&given&middle&suffix&prefix).
    let performing = field(msg, "OBR-34.1").and_then(|cnn| {
        let xcn = cnn.split('&').collect::<Vec<_>>().join("^");
        pn::from_xcn(&xcn)
    });
    match performing {
        Some(name) => {
            let v = bounded(
                tags::SCHEDULED_PERFORMING_PHYSICIAN_NAME,
                limits::PN,
                name.pn,
                options.length_policy,
                w,
            )?;
            put_str(
                &mut item,
                tags::SCHEDULED_PERFORMING_PHYSICIAN_NAME,
                VR::PN,
                &v,
            );
        }
        None => put_empty(&mut item, tags::SCHEDULED_PERFORMING_PHYSICIAN_NAME, VR::PN),
    }

    // (0040,0007) SPS Description, LO. IHE: OBR-4.2. One of description or
    // protocol code sequence is required.
    if let Some(desc) = field(msg, "OBR-4.2") {
        let desc = bounded(
            tags::SCHEDULED_PROCEDURE_STEP_DESCRIPTION,
            limits::LO,
            desc,
            options.length_policy,
            w,
        )?;
        put_str(
            &mut item,
            tags::SCHEDULED_PROCEDURE_STEP_DESCRIPTION,
            VR::LO,
            &desc,
        );
    }
    // (0040,0008) Scheduled Protocol Code Sequence, SQ. IHE: IPC-6 (Protocol
    // Code), only with a coding scheme.
    if ipc.is_some() {
        if let (Some(code), Some(scheme)) = (ipc_field(6, 1), ipc_field(6, 3)) {
            let mut c = InMemDicomObject::new_empty();
            put_str(
                &mut c,
                tags::CODE_VALUE,
                VR::SH,
                &bounded(tags::CODE_VALUE, limits::SH, code, options.length_policy, w)?,
            );
            put_str(
                &mut c,
                tags::CODING_SCHEME_DESIGNATOR,
                VR::SH,
                &bounded(
                    tags::CODING_SCHEME_DESIGNATOR,
                    limits::SH,
                    scheme,
                    options.length_policy,
                    w,
                )?,
            );
            if let Some(meaning) = ipc_field(6, 2) {
                put_str(
                    &mut c,
                    tags::CODE_MEANING,
                    VR::LO,
                    &bounded(
                        tags::CODE_MEANING,
                        limits::LO,
                        meaning,
                        options.length_policy,
                        w,
                    )?,
                );
            }
            put_seq(&mut item, tags::SCHEDULED_PROTOCOL_CODE_SEQUENCE, vec![c]);
        }
    }

    // (0040,0009) SPS ID, SH, Type 1. IHE: IPC-4, else OBR-20 (Filler Field
    // 1). When both are absent the Requested Procedure ID with a step index
    // stands in, and the substitution is reported (unsourced).
    let sps_id = match ipc_field(4, 1).or_else(|| field(msg, "OBR-20")) {
        Some(v) => v,
        None => {
            let rp = input
                .order
                .procedure_id
                .clone()
                .or_else(|| field(msg, "ORC-2.1"))
                .unwrap_or_else(|| "SPS".to_string());
            let id = format!("{rp}-{index}");
            w.push(Warning::MissingType1 {
                tag: tags::SCHEDULED_PROCEDURE_STEP_ID,
                substituted: Some(id.clone()),
            });
            id
        }
    };
    let sps_id = bounded(
        tags::SCHEDULED_PROCEDURE_STEP_ID,
        limits::SH,
        sps_id,
        options.length_policy,
        w,
    )?;
    put_str(
        &mut item,
        tags::SCHEDULED_PROCEDURE_STEP_ID,
        VR::SH,
        &sps_id,
    );

    // (0040,0010) Scheduled Station Name, SH, Type 2. IHE: IPC-7.
    match ipc_field(7, 1) {
        Some(v) => put_str(
            &mut item,
            tags::SCHEDULED_STATION_NAME,
            VR::SH,
            &bounded(
                tags::SCHEDULED_STATION_NAME,
                limits::SH,
                v,
                options.length_policy,
                w,
            )?,
        ),
        None => put_empty(&mut item, tags::SCHEDULED_STATION_NAME, VR::SH),
    }
    // (0040,0011) SPS Location, SH, Type 2. IHE: IPC-8.
    match ipc_field(8, 1) {
        Some(v) => put_str(
            &mut item,
            tags::SCHEDULED_PROCEDURE_STEP_LOCATION,
            VR::SH,
            &bounded(
                tags::SCHEDULED_PROCEDURE_STEP_LOCATION,
                limits::SH,
                v,
                options.length_policy,
                w,
            )?,
        ),
        None => put_empty(&mut item, tags::SCHEDULED_PROCEDURE_STEP_LOCATION, VR::SH),
    }
    // (0040,0020) SPS Status, CS, Type 3. Unsourced: SCHEDULED for a new
    // order (ORC-1 NW), omitted otherwise.
    if field(msg, "ORC-1")
        .map(|s| s.to_ascii_uppercase())
        .as_deref()
        == Some("NW")
    {
        put_str(
            &mut item,
            tags::SCHEDULED_PROCEDURE_STEP_STATUS,
            VR::CS,
            "SCHEDULED",
        );
    }
    Ok(item)
}

/// A DICOM modality code from IPC-5 or OBR-24: DICOM codes (PS3.3 C.7.3.1.1.1)
/// pass through, the HL7 table 0074 codes with a single DICOM meaning are
/// translated, everything else is `None`.
fn modality_code(value: &str, w: &mut Vec<Warning>) -> Option<String> {
    const DICOM: &[&str] = &[
        "AR", "AU", "BDUS", "BI", "BMD", "CR", "CT", "DG", "DX", "ECG", "EPS", "ES", "GM", "HC",
        "HD", "IO", "IVOCT", "IVUS", "KER", "LEN", "LS", "MG", "MR", "NM", "OAM", "OCT", "OP",
        "OPM", "OPT", "OPV", "OSS", "OT", "PT", "PX", "REG", "RF", "RG", "RTIMAGE", "SM", "SR",
        "TG", "US", "VA", "XA", "XC",
    ];
    // HL7 table 0074 codes that name exactly one DICOM modality. RAD
    // (radiology) and others are ambiguous and are refused.
    const TABLE_0074: &[(&str, &str)] = &[
        ("MRI", "MR"),
        ("NMR", "MR"),
        ("NMS", "NM"),
        ("RUS", "US"),
        ("VUS", "US"),
        ("XRC", "XA"),
        ("OTH", "OT"),
    ];
    let v = value.trim().to_ascii_uppercase();
    if DICOM.contains(&v.as_str()) {
        return Some(v);
    }
    let to = TABLE_0074.iter().find(|(from, _)| *from == v)?.1;
    w.push(Warning::ModalityTranslated {
        from: value.trim().to_string(),
        to: to.to_string(),
    });
    Some(to.to_string())
}

// ---------------------------------------------------------------------------
// Visit modules
// ---------------------------------------------------------------------------

/// IHE: (0038,0010) Admission ID from PV1-19.1, (0038,0300) Current Patient
/// Location from PV1-3 (PL: point of care^room^bed). Both Type 2.
fn visit(ds: &mut InMemDicomObject, msg: &Message) {
    match field(msg, "PV1-19.1") {
        Some(v) => put_str(
            ds,
            tags::ADMISSION_ID,
            VR::LO,
            &v.chars().take(limits::LO).collect::<String>(),
        ),
        None => put_empty(ds, tags::ADMISSION_ID, VR::LO),
    }
    let location: Vec<String> = ["PV1-3.1", "PV1-3.2", "PV1-3.3"]
        .iter()
        .filter_map(|p| field(msg, p))
        .collect();
    if location.is_empty() {
        put_empty(ds, tags::CURRENT_PATIENT_LOCATION, VR::LO);
    } else {
        let joined = location.join(" ");
        put_str(
            ds,
            tags::CURRENT_PATIENT_LOCATION,
            VR::LO,
            &joined.chars().take(limits::LO).collect::<String>(),
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A CWE-shaped field (`code^meaning^scheme`) as a code sequence item, only
/// when the scheme is present.
fn code_item(
    msg: &Message,
    path: &str,
    options: &Options,
    w: &mut Vec<Warning>,
) -> Result<Option<InMemDicomObject>, MwlError> {
    let code = field(msg, &format!("{path}.1"));
    let scheme = field(msg, &format!("{path}.3"));
    let (Some(code), Some(scheme)) = (code, scheme) else {
        return Ok(None);
    };
    let mut item = InMemDicomObject::new_empty();
    put_str(
        &mut item,
        tags::CODE_VALUE,
        VR::SH,
        &bounded(tags::CODE_VALUE, limits::SH, code, options.length_policy, w)?,
    );
    put_str(
        &mut item,
        tags::CODING_SCHEME_DESIGNATOR,
        VR::SH,
        &bounded(
            tags::CODING_SCHEME_DESIGNATOR,
            limits::SH,
            scheme,
            options.length_policy,
            w,
        )?,
    );
    if let Some(meaning) = field(msg, &format!("{path}.2")) {
        put_str(
            &mut item,
            tags::CODE_MEANING,
            VR::LO,
            &bounded(
                tags::CODE_MEANING,
                limits::LO,
                meaning,
                options.length_policy,
                w,
            )?,
        );
    }
    Ok(Some(item))
}

/// A decoded, trimmed, non-empty field or component.
fn field(msg: &Message, path: &str) -> Option<String> {
    msg.get_decoded(path)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// `YYYYMMDD` from an HL7 DTM, validated.
pub fn hl7_date(dtm: &str) -> Option<String> {
    let d: String = dtm.trim().chars().take(8).collect();
    if d.len() != 8 || !d.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let month: u8 = d[4..6].parse().ok()?;
    let day: u8 = d[6..8].parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some(d)
}

/// `HHMMSS` from an HL7 DTM (`YYYYMMDDHH[MM[SS]]…`), seconds and minutes
/// padded with zeros when absent, `None` when there is no hour.
pub fn hl7_time(dtm: &str) -> Option<String> {
    let s = dtm.trim();
    let end = s.find(['+', '-', '.']).unwrap_or(s.len());
    let digits = &s[..end];
    if digits.len() < 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let t = &digits[8..digits.len().min(14)];
    let mut t = t.to_string();
    while t.len() < 6 {
        t.push('0');
    }
    let h: u8 = t[..2].parse().ok()?;
    let m: u8 = t[2..4].parse().ok()?;
    let sec: u8 = t[4..6].parse().ok()?;
    (h < 24 && m < 60 && sec < 61).then_some(t)
}

fn put_str(ds: &mut InMemDicomObject, tag: Tag, vr: VR, value: &str) {
    ds.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
}

fn put_empty(ds: &mut InMemDicomObject, tag: Tag, vr: VR) {
    ds.put(DataElement::new(tag, vr, PrimitiveValue::Empty));
}

fn put_seq(ds: &mut InMemDicomObject, tag: Tag, items: Vec<InMemDicomObject>) {
    ds.put(DataElement::new(tag, VR::SQ, DataSetSequence::from(items)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_times() {
        assert_eq!(hl7_date("20260905113000").as_deref(), Some("20260905"));
        assert_eq!(hl7_date("2026090"), None);
        assert_eq!(hl7_date("20261305"), None);
        assert_eq!(hl7_time("20260905113000").as_deref(), Some("113000"));
        assert_eq!(hl7_time("202609051130").as_deref(), Some("113000"));
        assert_eq!(hl7_time("2026090511").as_deref(), Some("110000"));
        assert_eq!(hl7_time("20260905113000+0200").as_deref(), Some("113000"));
        assert_eq!(hl7_time("20260905"), None);
        assert_eq!(hl7_time("2026090525"), None);
    }

    #[test]
    fn modality_codes() {
        let mut w = Vec::new();
        assert_eq!(modality_code("MR", &mut w).as_deref(), Some("MR"));
        assert_eq!(modality_code("ct", &mut w).as_deref(), Some("CT"));
        assert!(w.is_empty());
        assert_eq!(modality_code("MRI", &mut w).as_deref(), Some("MR"));
        assert_eq!(w.len(), 1);
        assert_eq!(
            modality_code("RAD", &mut w),
            None,
            "ambiguous table 0074 code"
        );
        assert_eq!(modality_code("", &mut w), None);
    }
}
