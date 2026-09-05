//! Imaging-order identifiers as carried by an ORM^O01 / OMI^O23 message.
//!
//! The four fields that link an HL7 order to a DICOM study, per the IHE
//! Radiology Technical Framework (Scheduled Workflow):
//!
//! | Field | Location | DICOM counterpart |
//! | --- | --- | --- |
//! | Patient identifier | `PID-3.1` (first repetition) | Patient ID (0010,0020) |
//! | Accession number | `OBR-18` (Placer Field 1) | Accession Number (0008,0050) |
//! | Requested procedure ID | `OBR-19` (Placer Field 2) | Requested Procedure ID (0040,1001) |
//! | Study instance UID | `ZDS-1.1` | Study Instance UID (0020,000D) |
//!
//! `ZDS` is a vendor-defined Z-segment from the IHE profile, not part of HL7 v2
//! proper, so many sites never populate it. `ZDS-1` is a composite
//! `uid^^Application^DICOM`; only the first component is the UID.

use crate::{Message, Span};

/// One of the four order identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderField {
    /// `PID-3.1`.
    PatientId,
    /// `OBR-18`.
    Accession,
    /// `OBR-19`.
    ProcedureId,
    /// `ZDS-1.1`.
    StudyUid,
}

impl OrderField {
    /// All four fields, in display order.
    pub const ALL: [OrderField; 4] = [
        OrderField::PatientId,
        OrderField::Accession,
        OrderField::ProcedureId,
        OrderField::StudyUid,
    ];

    /// The query path this field is read from.
    pub fn path(self) -> &'static str {
        match self {
            OrderField::PatientId => "PID-3.1",
            OrderField::Accession => "OBR-18",
            OrderField::ProcedureId => "OBR-19",
            OrderField::StudyUid => "ZDS-1.1",
        }
    }

    /// Human readable label.
    pub fn label(self) -> &'static str {
        match self {
            OrderField::PatientId => "Patient ID",
            OrderField::Accession => "Accession number",
            OrderField::ProcedureId => "Requested procedure ID",
            OrderField::StudyUid => "Study instance UID",
        }
    }
}

/// The identifiers extracted from an order message.
///
/// Values are escape-decoded and whitespace-trimmed; empty values become
/// `None`. `spans` point at the raw (undecoded) text for highlighting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Order {
    /// `PID-3.1`.
    pub patient_id: Option<String>,
    /// `OBR-18`.
    pub accession: Option<String>,
    /// `OBR-19`.
    pub procedure_id: Option<String>,
    /// `ZDS-1.1`.
    pub study_uid: Option<String>,
    /// Byte spans of the fields that were present, for highlighting.
    pub spans: Vec<(Span, OrderField)>,
}

impl Order {
    /// Extract the order identifiers from a message.
    pub fn extract(msg: &Message) -> Order {
        let mut order = Order::default();
        for field in OrderField::ALL {
            let Some(span) = msg.get_span(field.path()) else {
                continue;
            };
            let value = msg.decode(span.slice(msg.raw()));
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            order.spans.push((span, field));
            let slot = match field {
                OrderField::PatientId => &mut order.patient_id,
                OrderField::Accession => &mut order.accession,
                OrderField::ProcedureId => &mut order.procedure_id,
                OrderField::StudyUid => &mut order.study_uid,
            };
            *slot = Some(value.to_string());
        }
        // Some sites carry the study UID in OBR-3 (Filler Order Number) or in
        // a ZDS placed before OBR. If a fallback is ever needed it belongs
        // here, gated behind an explicit option so the source stays auditable.
        order
    }

    /// Value of one field.
    pub fn get(&self, field: OrderField) -> Option<&str> {
        match field {
            OrderField::PatientId => self.patient_id.as_deref(),
            OrderField::Accession => self.accession.as_deref(),
            OrderField::ProcedureId => self.procedure_id.as_deref(),
            OrderField::StudyUid => self.study_uid.as_deref(),
        }
    }
}
