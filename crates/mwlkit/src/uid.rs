//! Study Instance UID resolution and generation, the one value this crate
//! is allowed to invent.
//!
//! Resolution follows `hl7kit`: `IPC-3.1`, then `ZDS-1.1`, then the OBX
//! carrying DCM 110180. When all are absent, [`UidPolicy`] decides.
//!
//! Generated UIDs use the `2.25.` root followed by a 128-bit UUID written
//! as a decimal integer (DICOM PS3.5 B.2, "UUID derived UID"). That form
//! needs no registered root, is at most 5 + 39 = 44 characters and so fits
//! the 64-character UI limit. No made-up root, no real vendor's root.

use crate::{GeneratedFrom as From_, MwlError, UidPolicy, Warning};
use hl7kit::order::{Order, StudyUidSource};

/// Where the Study Instance UID came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudyUidOrigin {
    /// Carried by the order; the RIS knows it.
    FromOrder(StudyUidSource),
    /// Generated here; the RIS does not know it.
    Generated(GeneratedFrom),
}

/// What a generated UID was derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedFrom {
    /// Name-based, from OBR-19 / IPC-2 (dcm4che's first choice).
    RequestedProcedureId,
    /// Name-based, from the accession number (dcm4che's second choice).
    AccessionNumber,
    /// From caller-supplied random bits.
    Random,
}

impl std::fmt::Display for GeneratedFrom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeneratedFrom::RequestedProcedureId => write!(f, "from the Requested Procedure ID"),
            GeneratedFrom::AccessionNumber => write!(f, "from the Accession Number"),
            GeneratedFrom::Random => write!(f, "at random"),
        }
    }
}

/// Namespace mixed into every name-based UID this crate derives, so a UID
/// derived here never collides with one derived elsewhere from the same
/// string.
pub const NAMESPACE: &str = "mwlkit.study-instance-uid";

/// `2.25.<uuid as decimal>` from 128 bits.
pub fn uuid_uid(bits: u128) -> String {
    format!("2.25.{bits}")
}

/// A deterministic UUID-derived UID for `name` under `namespace`: RFC 9562
/// version 8, built from two FNV-1a 64-bit hashes with different offsets.
pub fn name_based_uid(namespace: &str, name: &str) -> String {
    let text = format!("{namespace}|{name}");
    let hi = fnv1a(text.as_bytes(), 0xcbf2_9ce4_8422_2325);
    let lo = fnv1a(text.as_bytes(), 0x84222325_cbf29ce4 ^ 0x5bd1_e995_9a2b_f347);
    let hi = (hi & 0xffff_ffff_ffff_0fff) | 0x0000_0000_0000_8000; // version 8
    let lo = (lo & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000; // variant 10
    uuid_uid((u128::from(hi) << 64) | u128::from(lo))
}

fn fnv1a(bytes: &[u8], offset: u64) -> u64 {
    let mut h = offset;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The UID for the order, its origin, and a warning when it was generated.
pub fn resolve(
    order: &Order,
    policy: &UidPolicy,
) -> Result<(String, StudyUidOrigin, Option<Warning>), MwlError> {
    if let (Some(uid), Some(source)) = (&order.study_uid, order.study_uid_source) {
        return Ok((uid.clone(), StudyUidOrigin::FromOrder(source), None));
    }
    let (uid, from) = match policy {
        UidPolicy::Refuse => return Err(MwlError::NoStudyUid),
        UidPolicy::Random(bits) => (uuid_uid(*bits), From_::Random),
        // dcm4che derives a name-based UID from the Requested Procedure ID
        // when present, else from the Accession Number.
        UidPolicy::Dcm4cheStyle => match (&order.procedure_id, &order.accession) {
            (Some(rp), _) => (
                name_based_uid(NAMESPACE, &format!("rp|{rp}")),
                From_::RequestedProcedureId,
            ),
            (None, Some(acc)) => (
                name_based_uid(NAMESPACE, &format!("acc|{acc}")),
                From_::AccessionNumber,
            ),
            (None, None) => return Err(MwlError::NoStudyUid),
        },
    };
    Ok((
        uid,
        StudyUidOrigin::Generated(from),
        Some(Warning::StudyUidGenerated(from)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_uids_are_2_25_decimal_and_short_enough() {
        let u = name_based_uid(NAMESPACE, "rp|RP-1");
        assert!(u.starts_with("2.25."));
        assert!(u[5..].chars().all(|c| c.is_ascii_digit()));
        assert!(u.len() <= 44, "{u} is {} chars", u.len());
        assert_eq!(u, name_based_uid(NAMESPACE, "rp|RP-1"), "deterministic");
        assert_ne!(u, name_based_uid(NAMESPACE, "rp|RP-2"));
        assert_ne!(u, name_based_uid("other", "rp|RP-1"));
        assert_eq!(uuid_uid(0), "2.25.0");
        assert_eq!(uuid_uid(u128::MAX).len(), 44);
    }

    #[test]
    fn policies() {
        let mut order = Order::default();
        assert_eq!(
            resolve(&order, &UidPolicy::Refuse),
            Err(MwlError::NoStudyUid)
        );
        assert_eq!(
            resolve(&order, &UidPolicy::Dcm4cheStyle),
            Err(MwlError::NoStudyUid)
        );
        let (u, origin, w) = resolve(&order, &UidPolicy::Random(7)).unwrap();
        assert_eq!(u, "2.25.7");
        assert_eq!(origin, StudyUidOrigin::Generated(GeneratedFrom::Random));
        assert_eq!(w, Some(Warning::StudyUidGenerated(GeneratedFrom::Random)));

        order.accession = Some("ACC-1".into());
        let (a, origin, _) = resolve(&order, &UidPolicy::Dcm4cheStyle).unwrap();
        assert_eq!(
            origin,
            StudyUidOrigin::Generated(GeneratedFrom::AccessionNumber)
        );
        order.procedure_id = Some("RP-1".into());
        let (r, origin, _) = resolve(&order, &UidPolicy::Dcm4cheStyle).unwrap();
        assert_eq!(
            origin,
            StudyUidOrigin::Generated(GeneratedFrom::RequestedProcedureId)
        );
        assert_ne!(a, r);

        order.study_uid = Some("1.2.3".into());
        order.study_uid_source = Some(StudyUidSource::Ipc3);
        let (u, origin, w) = resolve(&order, &UidPolicy::Refuse).unwrap();
        assert_eq!(u, "1.2.3");
        assert_eq!(origin, StudyUidOrigin::FromOrder(StudyUidSource::Ipc3));
        assert_eq!(w, None);
    }
}
