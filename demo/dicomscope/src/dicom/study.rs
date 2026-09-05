//! The handful of study-level attributes the linkage needs.

use dicom_core::Tag;
use dicom_dictionary_std::tags;
use dicom_object::{DefaultDicomObject, InMemDicomObject};

/// Identifiers taken from the DICOM header. Free of dicom-rs types so the
/// linkage module can be tested with literals.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Study {
    /// (0010,0020)
    pub patient_id: Option<String>,
    /// (0008,0050)
    pub accession_number: Option<String>,
    /// (0020,000D)
    pub study_uid: Option<String>,
    /// (0040,1001)
    pub requested_procedure_id: Option<String>,
    /// (0002,0010)
    pub transfer_syntax: String,
    /// (0008,0060)
    pub modality: Option<String>,
    /// (0028,0010)
    pub rows: u32,
    /// (0028,0011)
    pub cols: u32,
}

impl Study {
    pub fn from_object(obj: &DefaultDicomObject) -> Study {
        Study {
            patient_id: string(obj, tags::PATIENT_ID),
            accession_number: string(obj, tags::ACCESSION_NUMBER),
            study_uid: string(obj, tags::STUDY_INSTANCE_UID),
            requested_procedure_id: string(obj, tags::REQUESTED_PROCEDURE_ID),
            transfer_syntax: obj
                .meta()
                .transfer_syntax()
                .trim_end_matches('\0')
                .trim()
                .to_string(),
            modality: string(obj, tags::MODALITY),
            rows: number(obj, tags::ROWS).unwrap_or(0),
            cols: number(obj, tags::COLUMNS).unwrap_or(0),
        }
    }
}

/// A trimmed string attribute, `None` when absent or empty. DICOM pads string
/// values to even length with spaces (or NUL for UIDs), so trimming is not
/// optional.
pub fn string(obj: &InMemDicomObject, tag: Tag) -> Option<String> {
    let e = obj.get(tag)?;
    // Files that lost their VR (UN/OB for a text attribute) carry the text as
    // bytes; render those as text instead of a byte list.
    let s = match e.value().primitive() {
        Some(dicom_core::PrimitiveValue::U8(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        _ => e.to_str().ok()?.into_owned(),
    };
    let s = s.trim_end_matches('\0').trim();
    (!s.is_empty()).then(|| s.to_string())
}

pub fn number(obj: &InMemDicomObject, tag: Tag) -> Option<u32> {
    obj.get(tag)?.to_int::<u32>().ok()
}

/// First value of a possibly multi-valued numeric string attribute such as
/// Window Center, which may be `40\80` for two presets.
pub fn first_float(obj: &InMemDicomObject, tag: Tag) -> Option<f32> {
    let e = obj.get(tag)?;
    let values = e.to_multi_str().ok()?;
    first_number(values.first()?)
}

/// Parse the first number of a `\`-separated decimal string.
pub fn first_number(text: &str) -> Option<f32> {
    text.split('\\').next()?.trim().parse::<f32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_number_takes_first_value() {
        assert_eq!(first_number("40\\80"), Some(40.0));
        assert_eq!(first_number(" -600 "), Some(-600.0));
        assert_eq!(first_number("1.5e2"), Some(150.0));
        assert_eq!(first_number(""), None);
        assert_eq!(first_number("abc"), None);
    }
}
