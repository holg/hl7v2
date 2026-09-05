//! Linking a DICOM study to an HL7 order. This is the reason the demo exists.
//!
//! Resolution order: Study Instance UID first (`ZDS-1.1` against
//! (0020,000D)), Accession Number second (`OBR-18` against (0008,0050)),
//! otherwise no link. Independently of the link, the patient identifier
//! (`PID-3.1` against (0010,0020)) is compared, because a study that links but
//! whose patient disagrees is exactly the case integration engineers hunt for.
//!
//! The fallback exists because `ZDS` is a vendor-defined Z-segment from the IHE
//! Radiology Technical Framework, not part of HL7 v2 proper, so many sites never
//! populate it; and when the study UID is absent, archives such as dcm4che
//! derive one from the requested procedure ID or the accession number, or
//! generate a random one. The identifier that looks canonical may have been
//! invented downstream.

use crate::dicom::Study;
use hl7kit::order::Order;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkPath {
    /// `ZDS-1.1` == (0020,000D), exact after trimming.
    StudyUid,
    /// `OBR-18` == (0008,0050), fallback.
    Accession,
    /// Neither identifier matched or both sides were absent.
    None,
}

impl LinkPath {
    pub fn label(self) -> &'static str {
        match self {
            LinkPath::StudyUid => "Study Instance UID (ZDS-1.1 = (0020,000D))",
            LinkPath::Accession => "Accession Number (OBR-18 = (0008,0050))",
            LinkPath::None => "no link",
        }
    }
}

/// One compared pair: the DICOM value and the HL7 value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pair {
    pub dicom: Option<String>,
    pub hl7: Option<String>,
}

impl Pair {
    fn new(dicom: Option<&str>, hl7: Option<&str>) -> Pair {
        Pair {
            dicom: clean(dicom),
            hl7: clean(hl7),
        }
    }
    /// `None` when either side is absent.
    pub fn matches(&self) -> Option<bool> {
        Some(self.dicom.as_deref()? == self.hl7.as_deref()?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linkage {
    pub path: LinkPath,
    /// `None` when either side lacks a patient identifier.
    pub patient_match: Option<bool>,
    pub study_uid: Pair,
    pub accession: Pair,
    pub patient_id: Pair,
    pub procedure_id: Pair,
}

impl Linkage {
    /// The headline case: linked, but the patient identifiers disagree.
    pub fn linked_with_patient_mismatch(&self) -> bool {
        self.path != LinkPath::None && self.patient_match == Some(false)
    }
}

pub fn resolve(study: &Study, order: &Order) -> Linkage {
    let study_uid = Pair::new(study.study_uid.as_deref(), order.study_uid.as_deref());
    let accession = Pair::new(
        study.accession_number.as_deref(),
        order.accession.as_deref(),
    );
    let patient_id = Pair::new(study.patient_id.as_deref(), order.patient_id.as_deref());
    let procedure_id = Pair::new(
        study.requested_procedure_id.as_deref(),
        order.procedure_id.as_deref(),
    );
    let path = if study_uid.matches() == Some(true) {
        LinkPath::StudyUid
    } else if accession.matches() == Some(true) {
        LinkPath::Accession
    } else {
        LinkPath::None
    };
    Linkage {
        path,
        patient_match: patient_id.matches(),
        study_uid,
        accession,
        patient_id,
        procedure_id,
    }
}

/// DICOM string values are space-padded to even length and UIDs NUL-padded;
/// HL7 values may carry stray whitespace. Compare after trimming both.
fn clean(v: Option<&str>) -> Option<String> {
    let s = v?.trim_end_matches('\0').trim();
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn study(uid: Option<&str>, acc: Option<&str>, pid: Option<&str>) -> Study {
        Study {
            patient_id: pid.map(String::from),
            accession_number: acc.map(String::from),
            study_uid: uid.map(String::from),
            ..Study::default()
        }
    }

    fn order(uid: Option<&str>, acc: Option<&str>, pid: Option<&str>) -> Order {
        Order {
            patient_id: pid.map(String::from),
            accession: acc.map(String::from),
            study_uid: uid.map(String::from),
            ..Order::default()
        }
    }

    #[test]
    fn study_uid_path_wins() {
        let l = resolve(
            &study(Some("1.2.3"), Some("A1"), Some("P1")),
            &order(Some("1.2.3"), Some("OTHER"), Some("P1")),
        );
        assert_eq!(l.path, LinkPath::StudyUid);
        assert_eq!(l.patient_match, Some(true));
        assert!(!l.linked_with_patient_mismatch());
        assert_eq!(l.accession.matches(), Some(false));
    }

    #[test]
    fn accession_fallback() {
        let l = resolve(
            &study(Some("1.2.3"), Some("A1"), Some("P1")),
            &order(None, Some("A1"), Some("P1")),
        );
        assert_eq!(l.path, LinkPath::Accession);
        assert_eq!(l.study_uid.matches(), None);
    }

    #[test]
    fn no_link() {
        let l = resolve(
            &study(Some("1.2.3"), Some("A1"), Some("P1")),
            &order(Some("9.9.9"), Some("A2"), Some("P1")),
        );
        assert_eq!(l.path, LinkPath::None);
        assert_eq!(l.patient_match, Some(true));
        assert!(!l.linked_with_patient_mismatch());
        let l = resolve(&study(None, None, None), &order(None, None, None));
        assert_eq!(l.path, LinkPath::None);
        assert_eq!(l.patient_match, None);
    }

    #[test]
    fn linked_but_patient_mismatch_is_flagged() {
        let l = resolve(
            &study(Some("1.2.3"), None, Some("4MR1")),
            &order(Some("1.2.3"), None, Some("9ZZ9")),
        );
        assert_eq!(l.path, LinkPath::StudyUid);
        assert_eq!(l.patient_match, Some(false));
        assert!(l.linked_with_patient_mismatch());
        let l = resolve(
            &study(None, Some("A1"), Some("4MR1")),
            &order(None, Some("A1"), Some("9ZZ9")),
        );
        assert_eq!(l.path, LinkPath::Accession);
        assert!(l.linked_with_patient_mismatch());
    }

    #[test]
    fn padded_dicom_values_still_match() {
        let l = resolve(
            &study(Some("1.2.3\0"), Some("A1 "), Some(" P1 ")),
            &order(Some("1.2.3"), Some("A1"), Some("P1")),
        );
        assert_eq!(l.path, LinkPath::StudyUid);
        assert_eq!(l.patient_match, Some(true));
        assert_eq!(l.accession.matches(), Some(true));
        assert_eq!(l.study_uid.dicom.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn absent_patient_on_one_side_is_unknown_not_mismatch() {
        let l = resolve(
            &study(Some("1.2.3"), None, Some("")),
            &order(Some("1.2.3"), None, Some("P1")),
        );
        assert_eq!(l.patient_match, None);
        assert!(!l.linked_with_patient_mismatch());
    }
}
