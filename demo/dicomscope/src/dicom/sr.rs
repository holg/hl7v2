//! Non-image instances that carry the report: DICOM Structured Reports,
//! rendered as an indented tree of text, and Encapsulated PDF, handed to the
//! browser's PDF viewer as a blob. Host-tested against the pydicom corpus.

use crate::dicom::study::string;
use dicom_core::value::Value;
use dicom_core::PrimitiveValue;
use dicom_dictionary_std::tags;
use dicom_object::{DefaultDicomObject, InMemDicomObject};

const SR_SOP_PREFIX: &str = "1.2.840.10008.5.1.4.1.1.88.";
const KEY_OBJECT_SOP: &str = "1.2.840.10008.5.1.4.1.1.88.59";
const PDF_SOP: &str = "1.2.840.10008.5.1.4.1.1.104.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    StructuredReport,
    KeyObjectSelection,
    EncapsulatedPdf,
}

impl DocumentKind {
    pub fn label(self) -> &'static str {
        match self {
            DocumentKind::StructuredReport => "SR",
            DocumentKind::KeyObjectSelection => "KOS",
            DocumentKind::EncapsulatedPdf => "PDF",
        }
    }
}

/// What kind of document an instance is, from its SOP Class UID.
pub fn document_kind(obj: &DefaultDicomObject) -> Option<DocumentKind> {
    let sop = string(obj, tags::SOP_CLASS_UID).unwrap_or_else(|| {
        obj.meta()
            .media_storage_sop_class_uid()
            .trim_end_matches('\0')
            .to_string()
    });
    if sop == PDF_SOP {
        Some(DocumentKind::EncapsulatedPdf)
    } else if sop == KEY_OBJECT_SOP {
        Some(DocumentKind::KeyObjectSelection)
    } else if sop.starts_with(SR_SOP_PREFIX) {
        Some(DocumentKind::StructuredReport)
    } else {
        None
    }
}

/// A title for the document list: the root concept name, the Document
/// Title, or the series description.
pub fn document_title(obj: &DefaultDicomObject) -> String {
    concept_name(obj)
        .or_else(|| string(obj, tags::DOCUMENT_TITLE))
        .or_else(|| string(obj, tags::SERIES_DESCRIPTION))
        .unwrap_or_else(|| "Document".to_string())
}

/// One line of a rendered report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrLine {
    pub depth: usize,
    /// Concept name, or the value type when there is none.
    pub name: String,
    /// The value, already formatted.
    pub value: String,
}

/// Render the content tree of a Structured Report (or Key Object
/// Selection). Unknown value types are shown by type so nothing is lost.
pub fn render_sr(obj: &DefaultDicomObject) -> Vec<SrLine> {
    let mut lines = Vec::new();
    let mut header = Vec::new();
    if let Some(c) = string(obj, tags::COMPLETION_FLAG) {
        header.push(format!("completion {c}"));
    }
    if let Some(v) = string(obj, tags::VERIFICATION_FLAG) {
        header.push(format!("verification {v}"));
    }
    lines.push(SrLine {
        depth: 0,
        name: concept_name(obj).unwrap_or_else(|| "Report".into()),
        value: header.join(", "),
    });
    render_items(obj, 1, &mut lines);
    lines
}

fn render_items(container: &InMemDicomObject, depth: usize, out: &mut Vec<SrLine>) {
    for item in sequence_items(container, tags::CONTENT_SEQUENCE) {
        let value_type = string(item, tags::VALUE_TYPE).unwrap_or_default();
        let name = concept_name(item).unwrap_or_else(|| value_type.clone());
        let value = match value_type.as_str() {
            "TEXT" => string(item, tags::TEXT_VALUE).unwrap_or_default(),
            "NUM" => numeric(item),
            "CODE" => sequence_items(item, tags::CONCEPT_CODE_SEQUENCE)
                .next()
                .and_then(|c| string(c, tags::CODE_MEANING))
                .unwrap_or_default(),
            "DATE" => string(item, tags::DATE).unwrap_or_default(),
            "TIME" => string(item, tags::TIME).unwrap_or_default(),
            "DATETIME" => string(item, tags::DATE_TIME).unwrap_or_default(),
            "PNAME" => string(item, tags::PERSON_NAME).unwrap_or_default(),
            "UIDREF" => string(item, tags::UID).unwrap_or_default(),
            "IMAGE" | "COMPOSITE" | "WAVEFORM" => {
                sequence_items(item, tags::REFERENCED_SOP_SEQUENCE)
                    .next()
                    .and_then(|r| string(r, tags::REFERENCED_SOP_INSTANCE_UID))
                    .map(|uid| format!("reference to {uid}"))
                    .unwrap_or_default()
            }
            "SCOORD" | "SCOORD3D" => string(item, tags::GRAPHIC_TYPE)
                .map(|g| format!("{g} coordinates"))
                .unwrap_or_default(),
            "CONTAINER" => string(item, tags::CONTINUITY_OF_CONTENT)
                .unwrap_or_default()
                .to_ascii_lowercase(),
            other => format!("({other})"),
        };
        out.push(SrLine { depth, name, value });
        render_items(item, depth + 1, out);
    }
}

fn numeric(item: &InMemDicomObject) -> String {
    let Some(m) = sequence_items(item, tags::MEASURED_VALUE_SEQUENCE).next() else {
        return String::new();
    };
    let value = string(m, tags::NUMERIC_VALUE).unwrap_or_default();
    let unit = sequence_items(m, tags::MEASUREMENT_UNITS_CODE_SEQUENCE)
        .next()
        .and_then(|u| string(u, tags::CODE_VALUE).or_else(|| string(u, tags::CODE_MEANING)))
        .unwrap_or_default();
    format!("{value} {unit}").trim().to_string()
}

fn concept_name(item: &InMemDicomObject) -> Option<String> {
    sequence_items(item, tags::CONCEPT_NAME_CODE_SEQUENCE)
        .next()
        .and_then(|c| string(c, tags::CODE_MEANING))
}

fn sequence_items(
    obj: &InMemDicomObject,
    tag: dicom_core::Tag,
) -> impl Iterator<Item = &InMemDicomObject> + '_ {
    obj.get(tag)
        .and_then(|e| match e.value() {
            Value::Sequence(seq) => Some(seq.items().iter()),
            _ => None,
        })
        .into_iter()
        .flatten()
}

/// The bytes of an Encapsulated Document (0042,0011) and its MIME type.
pub fn encapsulated_document(obj: &DefaultDicomObject) -> Option<(Vec<u8>, String)> {
    let e = obj.get(tags::ENCAPSULATED_DOCUMENT)?;
    let bytes = match e.value().primitive()? {
        PrimitiveValue::U8(b) => b.to_vec(),
        PrimitiveValue::U16(w) => w.iter().flat_map(|w| w.to_le_bytes()).collect(),
        _ => return None,
    };
    let mime = string(obj, tags::MIME_TYPE_OF_ENCAPSULATED_DOCUMENT)
        .unwrap_or_else(|| "application/pdf".to_string());
    Some((bytes, mime))
}

/// Plain-text rendering, for the host tool and for copying out of the UI.
pub fn sr_to_text(lines: &[SrLine]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&"  ".repeat(l.depth));
        out.push_str(&l.name);
        if !l.value.is_empty() {
            out.push_str(": ");
            out.push_str(&l.value);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dicom::load;

    fn corpus(name: &str) -> Option<Vec<u8>> {
        std::fs::read(format!(
            "{}/../../samples/dicom/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .ok()
    }

    #[test]
    fn basic_text_sr_renders_as_tree() {
        // pydicom's test-SR.dcm is a Basic Text SR; skip when the corpus
        // has not been fetched.
        let Some(bytes) = corpus("test-SR.dcm") else {
            eprintln!("samples/dicom/test-SR.dcm not fetched; skipping");
            return;
        };
        let obj = load(&bytes).unwrap();
        assert_eq!(document_kind(&obj), Some(DocumentKind::StructuredReport));
        let lines = render_sr(&obj);
        assert!(lines.len() > 3, "{}", sr_to_text(&lines));
        assert!(lines.iter().any(|l| l.depth >= 2), "nesting expected");
        let text = sr_to_text(&lines);
        assert!(text.contains("TEXT") || text.contains(':'), "{text}");
        assert!(!document_title(&obj).is_empty());
    }

    #[test]
    fn image_is_not_a_document() {
        let Some(bytes) = corpus("CT_small.dcm") else {
            return;
        };
        let obj = load(&bytes).unwrap();
        assert_eq!(document_kind(&obj), None);
        assert!(encapsulated_document(&obj).is_none());
    }
}
