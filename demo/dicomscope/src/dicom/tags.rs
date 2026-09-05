//! Flat display list of every data element, sequences included.

use dicom_core::dictionary::DataDictionary;
use dicom_core::header::{HasLength, Header};
use dicom_core::value::Value;
use dicom_core::{Tag, VR};
use dicom_dictionary_std::StandardDataDictionary;
use dicom_object::{DefaultDicomObject, InMemDicomObject};

const MAX_VALUE_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    /// `(gggg,eeee)`
    pub tag: String,
    /// Dictionary keyword, empty for private tags.
    pub keyword: String,
    pub vr: String,
    /// Truncated value, or a length for binary VRs.
    pub value: String,
    /// Nesting depth; 0 for top-level elements.
    pub depth: usize,
}

impl TagRow {
    /// Case-insensitive match against tag, keyword or value.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let n = needle.to_ascii_lowercase();
        self.tag.to_ascii_lowercase().contains(&n)
            || self.keyword.to_ascii_lowercase().contains(&n)
            || self.value.to_ascii_lowercase().contains(&n)
    }
}

pub fn tag_rows(obj: &DefaultDicomObject) -> Vec<TagRow> {
    let mut rows = Vec::new();
    for e in obj.meta().to_element_iter() {
        rows.push(row(e.tag(), e.vr(), &format_value(e.vr(), e.value()), 0));
    }
    push_dataset(&mut rows, obj, 0);
    rows
}

fn push_dataset(rows: &mut Vec<TagRow>, ds: &InMemDicomObject, depth: usize) {
    for e in ds.iter() {
        match e.value() {
            Value::Sequence(seq) => {
                let items = seq.items();
                rows.push(row(
                    e.tag(),
                    e.vr(),
                    &format!("sequence, {} item(s)", items.len()),
                    depth,
                ));
                for (i, item) in items.iter().enumerate() {
                    rows.push(TagRow {
                        tag: String::new(),
                        keyword: format!("item {}", i + 1),
                        vr: String::new(),
                        value: String::new(),
                        depth: depth + 1,
                    });
                    push_dataset(rows, item, depth + 1);
                }
            }
            Value::PixelSequence(px) => {
                rows.push(row(
                    e.tag(),
                    e.vr(),
                    &format!(
                        "encapsulated pixel data, {} fragment(s)",
                        px.fragments().len()
                    ),
                    depth,
                ));
            }
            Value::Primitive(_) => rows.push(row(
                e.tag(),
                e.vr(),
                &format_value(e.vr(), e.value()),
                depth,
            )),
        }
    }
}

fn row(tag: Tag, vr: VR, value: &str, depth: usize) -> TagRow {
    TagRow {
        tag: tag.to_string(),
        keyword: keyword(tag),
        vr: vr.to_string().to_owned(),
        value: value.to_string(),
        depth,
    }
}

pub fn keyword(tag: Tag) -> String {
    StandardDataDictionary
        .by_tag(tag)
        .map(|e| e.alias.to_string())
        .unwrap_or_else(|| {
            if tag.group() % 2 == 1 {
                "(private)".to_string()
            } else {
                String::new()
            }
        })
}

/// Binary VRs show their length only; everything else is a truncated string.
pub fn format_value<I, P>(vr: VR, value: &Value<I, P>) -> String
where
    I: HasLength,
{
    let Some(prim) = value.primitive() else {
        return String::new();
    };
    if matches!(
        vr,
        VR::OB | VR::OW | VR::UN | VR::OF | VR::OD | VR::OL | VR::OV
    ) {
        return format!("<{} bytes>", prim.calculate_byte_len());
    }
    truncate(&prim.to_str(), MAX_VALUE_CHARS)
}

pub fn truncate(text: &str, max_chars: usize) -> String {
    let text = text.trim_end_matches('\0').trim_end();
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => format!(
            "{}… (+{} chars)",
            &text[..idx],
            text.chars().count() - max_chars
        ),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_lookup() {
        assert_eq!(keyword(Tag(0x0010, 0x0020)), "PatientID");
        assert_eq!(keyword(Tag(0x0009, 0x1001)), "(private)");
        assert_eq!(keyword(Tag(0x0008, 0xFFFE)), "");
    }

    #[test]
    fn truncation_marks_itself() {
        assert_eq!(truncate("short  ", 10), "short");
        let long = "x".repeat(130);
        let t = truncate(&long, 120);
        assert!(t.starts_with(&"x".repeat(120)));
        assert!(t.ends_with("… (+10 chars)"));
    }

    #[test]
    fn row_filter() {
        let r = TagRow {
            tag: "(0010,0020)".into(),
            keyword: "PatientID".into(),
            vr: "LO".into(),
            value: "4MR1".into(),
            depth: 0,
        };
        assert!(r.matches("patientid"));
        assert!(r.matches("0010,00"));
        assert!(r.matches("4mr"));
        assert!(r.matches(""));
        assert!(!r.matches("accession"));
    }
}
