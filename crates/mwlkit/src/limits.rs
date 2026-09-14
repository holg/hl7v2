//! VR length limits (DICOM PS3.5 Table 6.2-1), enforced when the item is
//! built so that every warning names its attribute.
//!
//! Policy: SH, LO and PN follow [`LengthPolicy`]. AE and UI are never cut,
//! whatever the policy: a truncated AE title is a wrong destination and a
//! truncated UID is a different study. This is a design decision of this
//! crate, not a rule of the standard.

use crate::{LengthPolicy, MwlError, Warning};
use dicom_core::Tag;

/// Short String.
pub const SH: usize = 16;
/// Long String.
pub const LO: usize = 64;
/// Person Name, per component group.
pub const PN: usize = 64;
/// Application Entity.
pub const AE: usize = 16;
/// Code String.
pub const CS: usize = 16;
/// Unique Identifier.
pub const UI: usize = 64;

/// Apply the policy to a SH, LO, PN or CS value. Lengths are in characters,
/// which equals bytes for the single-byte character sets the standard's
/// limits were written for and is the lenient reading under UTF-8.
pub fn bounded(
    tag: Tag,
    max: usize,
    value: String,
    policy: LengthPolicy,
    warnings: &mut Vec<Warning>,
) -> Result<String, MwlError> {
    let actual = value.chars().count();
    if actual <= max {
        return Ok(value);
    }
    match policy {
        LengthPolicy::Refuse => Err(MwlError::TooLong { tag, max, actual }),
        LengthPolicy::TruncateAndWarn => {
            warnings.push(Warning::Truncated { tag, max, actual });
            Ok(value.chars().take(max).collect())
        }
    }
}

/// AE and UI: refuse regardless of policy.
pub fn strict(tag: Tag, max: usize, value: String) -> Result<String, MwlError> {
    let actual = value.chars().count();
    if actual <= max {
        Ok(value)
    } else {
        Err(MwlError::TooLong { tag, max, actual })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_dictionary_std::tags;

    #[test]
    fn bounded_truncates_only_when_allowed() {
        let long = "ACC-2026-00000000001".to_string(); // 20 chars
        let mut w = Vec::new();
        assert_eq!(
            bounded(
                tags::ACCESSION_NUMBER,
                SH,
                long.clone(),
                LengthPolicy::Refuse,
                &mut w
            ),
            Err(MwlError::TooLong {
                tag: tags::ACCESSION_NUMBER,
                max: 16,
                actual: 20
            })
        );
        assert!(w.is_empty());
        let cut = bounded(
            tags::ACCESSION_NUMBER,
            SH,
            long,
            LengthPolicy::TruncateAndWarn,
            &mut w,
        )
        .unwrap();
        assert_eq!(cut, "ACC-2026-0000000");
        assert_eq!(
            w,
            [Warning::Truncated {
                tag: tags::ACCESSION_NUMBER,
                max: 16,
                actual: 20
            }]
        );
        let ok = bounded(
            tags::ACCESSION_NUMBER,
            SH,
            "short".into(),
            LengthPolicy::Refuse,
            &mut w,
        )
        .unwrap();
        assert_eq!(ok, "short");
    }

    #[test]
    fn strict_never_truncates() {
        assert!(strict(tags::SCHEDULED_STATION_AE_TITLE, AE, "A".repeat(16)).is_ok());
        assert!(matches!(
            strict(tags::SCHEDULED_STATION_AE_TITLE, AE, "A".repeat(17)),
            Err(MwlError::TooLong {
                max: 16,
                actual: 17,
                ..
            })
        ));
    }
}
