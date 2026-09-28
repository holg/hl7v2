//! `ImagingStudy` from the study header and the existing series grouping,
//! declaring the MII Modul Bildgebung profile.
//!
//! Sources: FHIR R4 `ImagingStudy`; MII Bildgebung ImagingStudy profile
//! (see `ids::MII_IMAGING_STUDY_VERSION` for the version read); DICOM
//! PS3.3 study, series and image modules. The profile is declared and its
//! core elements are populated; conformance is not claimed and the MII
//! modality-specific extensions are not emitted.
//!
//! Input hygiene: `series.uid`, `instance.uid` and `instance.sopClass` are
//! required elements typed `id` or `urn:oid:`, so a series or instance
//! whose UID is not a UID (see [`is_uid`]) is left out rather than emitted
//! as an element the resource cannot legally carry; the counts reflect
//! what is listed. Free-text elements go through [`fhir_string`].

use super::datetime::dicom_datetime;
use super::ids::{
    fhir_string, is_uid, oid_urn, reference, typed_identifier, uid_identifier, Authority, Refs,
    DCM, IMAGING_STUDY_ID, MII_IMAGING_STUDY_PROFILE, MII_IMAGING_STUDY_VERSION, RFC3986,
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
        identifiers.extend(uid_identifier(uid));
    }
    if let Some(acc) = study.accession_number.as_deref().and_then(fhir_string) {
        // (0008,0051) Issuer of Accession Number Sequence is not read; no
        // system is fabricated.
        identifiers.push(typed_identifier(
            "ACSN",
            "Accession ID",
            &acc,
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
    let modalities: BTreeSet<String> = series
        .iter()
        .filter_map(|s| modality_code(&s.modality))
        .collect();
    let modalities: BTreeSet<String> = if modalities.is_empty() {
        study
            .modality
            .as_deref()
            .and_then(modality_code)
            .into_iter()
            .collect()
    } else {
        modalities
    };
    if !modalities.is_empty() {
        is["modality"] = json!(modalities
            .iter()
            .map(|m| json!({ "system": DCM, "code": m }))
            .collect::<Vec<_>>());
    }

    if let Some(desc) = study.study_description.as_deref().and_then(fhir_string) {
        is["description"] = json!(desc);
    }

    let mut total_instances = 0usize;
    let mut series_json = Vec::new();
    for s in series.iter().filter(|s| is_uid(&s.uid)) {
        // Multi-frame files appear once per frame in `slices`; an instance
        // is one SOP Instance UID.
        let mut seen = BTreeSet::new();
        let mut instances = Vec::new();
        for slice in &s.slices {
            if !is_uid(&slice.sop_instance_uid)
                || !is_uid(&slice.sop_class_uid)
                || !seen.insert(slice.sop_instance_uid.trim_end_matches('\0').trim())
            {
                continue;
            }
            let mut inst = json!({
                "uid": slice.sop_instance_uid.trim_end_matches('\0').trim(),
                "sopClass": { "system": RFC3986, "code": oid_urn(&slice.sop_class_uid) },
            });
            if let Some(n) = slice.instance_number {
                inst["number"] = json!(n);
            }
            instances.push(inst);
        }
        total_instances += instances.len();
        let mut sj = json!({
            "uid": s.uid.trim_end_matches('\0').trim(),
            "numberOfInstances": instances.len(),
        });
        if let Some(n) = s.number {
            sj["number"] = json!(n);
        }
        if let Some(m) = modality_code(&s.modality) {
            sj["modality"] = json!({ "system": DCM, "code": m });
        }
        if let Some(desc) = fhir_string(&s.description) {
            sj["description"] = json!(desc);
        }
        if !instances.is_empty() {
            sj["instance"] = json!(instances);
        }
        series_json.push(sj);
    }
    is["numberOfSeries"] = json!(series_json.len());
    is["numberOfInstances"] = json!(total_instances);
    if !series_json.is_empty() {
        is["series"] = json!(series_json);
    }
    is
}

/// A (0008,0060) value as a DCM code: a CS token (uppercase letters,
/// digits, `_`), which every defined modality code is. The scanner's `?`
/// placeholder for a missing modality and anything else yield `None`.
pub fn modality_code(raw: &str) -> Option<String> {
    let code = raw.trim_end_matches('\0').trim();
    let cs = !code.is_empty()
        && code.len() <= 16
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    cs.then(|| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dicom::series::Slice;

    fn study() -> Study {
        Study {
            study_uid: Some("1.2.3".into()),
            accession_number: Some("ACC-1".into()),
            modality: Some("CT".into()),
            study_date: Some("20240229".into()),
            study_time: Some("101500".into()),
            ..Study::default()
        }
    }

    fn slice(n: i32, uid: &str) -> Slice {
        Slice {
            file: 0,
            frame: 0,
            instance_number: Some(n),
            position: None,
            sop_instance_uid: uid.into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
        }
    }

    fn series(uid: &str, modality: &str, slices: Vec<Slice>) -> Series {
        Series {
            uid: uid.into(),
            number: Some(1),
            description: String::new(),
            modality: modality.into(),
            rows: 1,
            cols: 1,
            slices,
        }
    }

    fn refs() -> Refs {
        Refs::for_study("1.2.3")
    }

    #[test]
    fn modality_codes_are_cs_tokens() {
        assert_eq!(modality_code("CT").as_deref(), Some("CT"));
        assert_eq!(modality_code(" MR ").as_deref(), Some("MR"));
        assert_eq!(modality_code("PT\0").as_deref(), Some("PT"));
        assert_eq!(modality_code("SEG").as_deref(), Some("SEG"));
        assert_eq!(modality_code("?"), None, "scanner placeholder");
        assert_eq!(modality_code(""), None);
        assert_eq!(modality_code("   "), None);
        assert_eq!(modality_code("ct"), None, "CS is upper case");
        assert_eq!(modality_code("C T"), None);
        assert_eq!(modality_code("CT;DROP"), None);
        assert_eq!(modality_code(&"X".repeat(17)), None);
    }

    #[test]
    fn multi_frame_and_duplicate_slices_count_one_instance() {
        let s = vec![series(
            "1.2.3.1",
            "US",
            vec![
                Slice {
                    frame: 0,
                    ..slice(1, "1.2.3.1.1")
                },
                Slice {
                    frame: 1,
                    ..slice(1, "1.2.3.1.1")
                },
                Slice {
                    frame: 2,
                    ..slice(1, "1.2.3.1.1")
                },
                slice(2, "1.2.3.1.2"),
                // Padded duplicate of the second instance, as a second copy
                // of the same file in a zip would give.
                slice(2, "1.2.3.1.2\0"),
            ],
        )];
        let is = imaging_study(&study(), &s, true, &refs());
        assert_eq!(is["numberOfSeries"], 1);
        assert_eq!(is["numberOfInstances"], 2);
        assert_eq!(is["series"][0]["numberOfInstances"], 2);
        assert_eq!(is["series"][0]["instance"].as_array().unwrap().len(), 2);
        assert_eq!(is["series"][0]["instance"][1]["uid"], "1.2.3.1.2");
    }

    #[test]
    fn mixed_modalities_are_listed_once_each_sorted() {
        let s = vec![
            series("1.2.3.1", "PT", vec![slice(1, "1.2.3.1.1")]),
            series("1.2.3.2", "CT", vec![slice(1, "1.2.3.2.1")]),
            series("1.2.3.3", "CT", vec![slice(1, "1.2.3.3.1")]),
            series("1.2.3.4", "?", vec![slice(1, "1.2.3.4.1")]),
        ];
        let is = imaging_study(&study(), &s, true, &refs());
        let codes: Vec<&str> = is["modality"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["code"].as_str().unwrap())
            .collect();
        assert_eq!(codes, ["CT", "PT"]);
        assert!(is["modality"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["system"] == DCM));
        assert!(
            is["series"][3].get("modality").is_none(),
            "`?` gives no series modality"
        );
        assert_eq!(is["numberOfSeries"], 4);
    }

    #[test]
    fn study_modality_is_the_fallback_only() {
        let unknown = vec![series("1.2.3.1", "?", vec![slice(1, "1.2.3.1.1")])];
        let is = imaging_study(&study(), &unknown, true, &refs());
        assert_eq!(is["modality"][0]["code"], "CT", "from (0008,0060)");

        let none = Study {
            modality: None,
            ..study()
        };
        let is = imaging_study(&none, &unknown, true, &refs());
        assert!(is.get("modality").is_none());

        let bad = Study {
            modality: Some("?".into()),
            ..study()
        };
        let is = imaging_study(&bad, &unknown, true, &refs());
        assert!(is.get("modality").is_none());

        let mr = vec![series("1.2.3.1", "MR", vec![slice(1, "1.2.3.1.1")])];
        let is = imaging_study(&study(), &mr, true, &refs());
        assert_eq!(is["modality"][0]["code"], "MR", "series wins over study");
        assert_eq!(is["modality"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn based_on_only_when_linked() {
        let s = vec![series("1.2.3.1", "CT", vec![slice(1, "1.2.3.1.1")])];
        let linked = imaging_study(&study(), &s, true, &refs());
        assert_eq!(linked["basedOn"][0]["reference"], refs().service_request);
        let unlinked = imaging_study(&study(), &s, false, &refs());
        assert!(unlinked.get("basedOn").is_none());
        assert_eq!(unlinked["subject"]["reference"], refs().patient);
    }

    #[test]
    fn malformed_uids_are_left_out_not_emitted() {
        let s = vec![
            series("not a uid", "CT", vec![slice(1, "1.2.3.1.1")]),
            series(
                "1.2.3.2",
                "CT",
                vec![
                    slice(1, ""),
                    slice(2, "garbage"),
                    Slice {
                        sop_class_uid: "".into(),
                        ..slice(3, "1.2.3.2.3")
                    },
                    slice(4, "1.2.3.2.4"),
                ],
            ),
        ];
        let bad_study = Study {
            study_uid: Some("urn:oid:1.2.3".into()),
            ..study()
        };
        let is = imaging_study(&bad_study, &s, true, &refs());
        assert_eq!(is["numberOfSeries"], 1, "the garbage series is not listed");
        assert_eq!(is["numberOfInstances"], 1);
        assert_eq!(is["series"][0]["uid"], "1.2.3.2");
        assert_eq!(is["series"][0]["instance"][0]["uid"], "1.2.3.2.4");
        let ids = is["identifier"].as_array().unwrap();
        assert_eq!(
            ids.len(),
            1,
            "only the accession: the study UID is not a UID"
        );
        assert_eq!(ids[0]["type"]["coding"][0]["code"], "ACSN");
    }

    #[test]
    fn empty_and_padded_text_is_omitted_or_trimmed() {
        let mut s = series("1.2.3.1", "CT", vec![slice(1, "1.2.3.1.1")]);
        s.description = "  \0".into();
        let blank = Study {
            study_description: Some("   ".into()),
            accession_number: Some("\0\0".into()),
            ..study()
        };
        let is = imaging_study(&blank, &[s.clone()], true, &refs());
        assert!(is.get("description").is_none());
        assert!(is["series"][0].get("description").is_none());
        assert_eq!(
            is["identifier"].as_array().unwrap().len(),
            1,
            "study UID only; a blank accession is no accession"
        );

        s.description = " Axial 2mm \u{1}".into();
        let is = imaging_study(&study(), &[s], true, &refs());
        assert_eq!(is["series"][0]["description"], "Axial 2mm");
    }

    #[test]
    fn empty_study_is_still_a_resource() {
        let is = imaging_study(&Study::default(), &[], false, &refs());
        assert_eq!(is["resourceType"], "ImagingStudy");
        assert_eq!(is["status"], "available");
        assert_eq!(is["numberOfSeries"], 0);
        assert_eq!(is["numberOfInstances"], 0);
        for absent in [
            "identifier",
            "basedOn",
            "started",
            "modality",
            "description",
            "series",
        ] {
            assert!(is.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn started_uses_the_leap_day_and_the_offset() {
        let s = vec![series("1.2.3.1", "CT", vec![slice(1, "1.2.3.1.1")])];
        let is = imaging_study(&study(), &s, true, &refs());
        assert_eq!(is["started"], "2024-02-29", "no offset: date only");
        let zoned = Study {
            timezone_offset: Some("-0500".into()),
            ..study()
        };
        let is = imaging_study(&zoned, &s, true, &refs());
        assert_eq!(is["started"], "2024-02-29T10:15:00-05:00");
        let invalid = Study {
            study_date: Some("20230229".into()),
            ..study()
        };
        let is = imaging_study(&invalid, &s, true, &refs());
        assert!(is.get("started").is_none(), "2023-02-29 does not exist");
    }
}
