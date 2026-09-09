//! `ImagingStudy` from the study header and the existing series grouping,
//! declaring the MII Modul Bildgebung profile.
//!
//! Sources: FHIR R4 `ImagingStudy`; MII Bildgebung ImagingStudy profile
//! (see `ids::MII_IMAGING_STUDY_VERSION` for the version read); DICOM
//! PS3.3 study, series and image modules. The profile is declared and its
//! core elements are populated; conformance is not claimed and the MII
//! modality-specific extensions are not emitted.

use super::datetime::dicom_datetime;
use super::ids::{
    oid_urn, reference, typed_identifier, uid_identifier, Authority, Refs, DCM, IMAGING_STUDY_ID,
    MII_IMAGING_STUDY_PROFILE, MII_IMAGING_STUDY_VERSION, RFC3986,
};
use crate::dicom::{Series, Study};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// `linked` says whether the study resolved to the order; `basedOn` is
/// asserted only then, because an unlinked study has no known order.
pub fn imaging_study(study: &Study, series: &[Series], linked: bool, refs: &Refs) -> Value {
    let mut is = json!({
        "resourceType": "ImagingStudy",
        "id": IMAGING_STUDY_ID,
        // `canonical|version`: the profile version this mapping was read
        // against, so a validator checks the right edition.
        "meta": { "profile": [format!("{MII_IMAGING_STUDY_PROFILE}|{MII_IMAGING_STUDY_VERSION}")] },
        "status": "available",
        "subject": reference(&refs.patient),
    });

    let mut identifiers = Vec::new();
    if let Some(uid) = &study.study_uid {
        identifiers.push(uid_identifier(uid));
    }
    if let Some(acc) = &study.accession_number {
        // (0008,0051) Issuer of Accession Number Sequence is not read; no
        // system is fabricated.
        identifiers.push(typed_identifier(
            "ACSN",
            "Accession ID",
            acc,
            &Authority::default(),
        ));
    }
    if !identifiers.is_empty() {
        is["identifier"] = json!(identifiers);
    }

    if linked {
        is["basedOn"] = json!([reference(&refs.service_request)]);
    }

    // (0008,0020)+(0008,0030), with the time only when (0008,0201) is present.
    if let Some(started) = study.study_date.as_deref().and_then(|da| {
        dicom_datetime(
            da,
            study.study_time.as_deref(),
            study.timezone_offset.as_deref(),
        )
    }) {
        is["started"] = json!(started);
    }

    // Distinct modalities across series, DICOM controlled terminology.
    let modalities: BTreeSet<&str> = series
        .iter()
        .map(|s| s.modality.trim())
        .filter(|m| !m.is_empty() && *m != "?")
        .collect();
    let modalities: BTreeSet<&str> = if modalities.is_empty() {
        study.modality.iter().map(|m| m.as_str()).collect()
    } else {
        modalities
    };
    if !modalities.is_empty() {
        is["modality"] = json!(modalities
            .iter()
            .map(|m| json!({ "system": DCM, "code": m }))
            .collect::<Vec<_>>());
    }

    if let Some(desc) = &study.study_description {
        is["description"] = json!(desc);
    }

    let mut total_instances = 0usize;
    let mut series_json = Vec::new();
    for s in series {
        // Multi-frame files appear once per frame in `slices`; an instance
        // is one SOP Instance UID.
        let mut seen = BTreeSet::new();
        let mut instances = Vec::new();
        for slice in &s.slices {
            if slice.sop_instance_uid.is_empty() || !seen.insert(slice.sop_instance_uid.as_str()) {
                continue;
            }
            let mut inst = json!({
                "uid": slice.sop_instance_uid,
                "sopClass": { "system": RFC3986, "code": oid_urn(&slice.sop_class_uid) },
            });
            if let Some(n) = slice.instance_number {
                inst["number"] = json!(n);
            }
            instances.push(inst);
        }
        total_instances += instances.len();
        let mut sj = json!({
            "uid": s.uid,
            "numberOfInstances": instances.len(),
        });
        if let Some(n) = s.number {
            sj["number"] = json!(n);
        }
        if !s.modality.is_empty() && s.modality != "?" {
            sj["modality"] = json!({ "system": DCM, "code": s.modality });
        }
        if !s.description.is_empty() {
            sj["description"] = json!(s.description);
        }
        if !instances.is_empty() {
            sj["instance"] = json!(instances);
        }
        series_json.push(sj);
    }
    is["numberOfSeries"] = json!(series.len());
    is["numberOfInstances"] = json!(total_instances);
    if !series_json.is_empty() {
        is["series"] = json!(series_json);
    }
    is
}
