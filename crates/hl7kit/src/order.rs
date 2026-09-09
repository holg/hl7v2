//! Imaging-order identifiers as carried by an ORM^O01 or OMI^O23 message.
//!
//! The four fields that link an HL7 order to a DICOM study, per the IHE
//! Radiology Technical Framework (Scheduled Workflow) and the HL7 v2.5.1
//! imaging order messages:
//!
//! | Field | Candidates, in precedence order | DICOM counterpart |
//! | --- | --- | --- |
//! | Patient identifier | `PID-3.1` (first repetition) | Patient ID (0010,0020) |
//! | Accession number | `OBR-18` (Placer Field 1), then `IPC-1.1` | Accession Number (0008,0050) |
//! | Requested procedure ID | `OBR-19` (Placer Field 2), then `IPC-2.1` | Requested Procedure ID (0040,1001) |
//! | Study instance UID | `IPC-3.1`, then `ZDS-1.1` | Study Instance UID (0020,000D) |
//!
//! `IPC` (Imaging Procedure Control) is a standard segment of the imaging
//! order messages (OMI^O23, HL7 2.5.1 and later): `IPC-1` is the accession
//! identifier, `IPC-2` the requested procedure ID, `IPC-3` the Study
//! Instance UID, `IPC-4` the scheduled procedure step ID. When an order has
//! several procedure steps, several `IPC` segments follow one `ORC`/`OBR`
//! pair and their `IPC-3` must agree; when they do not, the first is taken
//! and a [`Warning::ConflictingStudyUid`] is recorded on the [`Order`].
//!
//! `ZDS` is a vendor-defined Z-segment from the IHE profile used with
//! ORM^O01 on older interfaces, not part of HL7 v2 proper, so many sites
//! never populate it. `ZDS-1` is a composite `uid^^Application^DICOM`; only
//! the first component is the UID.

use crate::{Message, Span, Warning};

/// One of the four order identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderField {
    /// `PID-3.1`.
    PatientId,
    /// `OBR-18`, else `IPC-1.1`.
    Accession,
    /// `OBR-19`, else `IPC-2.1`.
    ProcedureId,
    /// `IPC-3.1`, else `ZDS-1.1`.
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

    /// Candidate query paths in precedence order. The first path with a
    /// non-empty value wins.
    pub fn paths(self) -> &'static [&'static str] {
        match self {
            OrderField::PatientId => &["PID-3.1"],
            OrderField::Accession => &["OBR-18", "IPC-1.1"],
            OrderField::ProcedureId => &["OBR-19", "IPC-2.1"],
            OrderField::StudyUid => &["IPC-3.1", "ZDS-1.1"],
        }
    }

    /// The first candidate path. Prefer [`OrderField::paths`].
    #[deprecated(since = "0.2.0", note = "use `paths()`; several paths are consulted")]
    pub fn path(self) -> &'static str {
        self.paths()[0]
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

/// Which segment supplied the study instance UID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StudyUidSource {
    /// `IPC-3.1`, the standard imaging-order segment.
    Ipc3,
    /// `ZDS-1.1`, the IHE vendor Z-segment.
    Zds1,
}

impl StudyUidSource {
    /// The path that was read.
    pub fn path(self) -> &'static str {
        match self {
            StudyUidSource::Ipc3 => "IPC-3.1",
            StudyUidSource::Zds1 => "ZDS-1.1",
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
    /// `OBR-18` or `IPC-1.1`.
    pub accession: Option<String>,
    /// `OBR-19` or `IPC-2.1`.
    pub procedure_id: Option<String>,
    /// `IPC-3.1` or `ZDS-1.1`.
    pub study_uid: Option<String>,
    /// Which segment supplied `study_uid`, when present.
    pub study_uid_source: Option<StudyUidSource>,
    /// Byte spans of the fields that were present, for highlighting. For
    /// each field this is the span of the path that matched.
    pub spans: Vec<(Span, OrderField)>,
    /// Non-fatal findings about the order, such as disagreeing `IPC-3`
    /// values across procedure steps.
    pub warnings: Vec<Warning>,
    /// The path that supplied each present field; see [`Order::source_path`].
    pub sources: Vec<(OrderField, &'static str)>,
}

impl Order {
    /// Extract the order identifiers from a message.
    pub fn extract(msg: &Message) -> Order {
        let mut order = Order::default();
        for field in OrderField::ALL {
            for path in field.paths() {
                let Some(span) = msg.get_span(path) else {
                    continue;
                };
                let value = msg.decode(span.slice(msg.raw()));
                let value = value.trim();
                if value.is_empty() {
                    continue;
                }
                order.spans.push((span, field));
                order.sources.push((field, path));
                let slot = match field {
                    OrderField::PatientId => &mut order.patient_id,
                    OrderField::Accession => &mut order.accession,
                    OrderField::ProcedureId => &mut order.procedure_id,
                    OrderField::StudyUid => {
                        order.study_uid_source = Some(match *path {
                            "IPC-3.1" => StudyUidSource::Ipc3,
                            _ => StudyUidSource::Zds1,
                        });
                        &mut order.study_uid
                    }
                };
                *slot = Some(value.to_string());
                break;
            }
        }
        // Several procedure steps, several IPC segments: their study UIDs
        // must agree. Take the first, say when they do not.
        let mut ipc_uids = msg
            .segments_named("IPC")
            .filter_map(|seg| seg.field(3)?.component(1))
            .map(|c| (c.span(), msg.decode(c.value()).trim().to_string()))
            .filter(|(_, v)| !v.is_empty());
        if let Some((first_span, first)) = ipc_uids.next() {
            if let Some((other, _)) = ipc_uids.find(|(_, v)| *v != first) {
                order.warnings.push(Warning::ConflictingStudyUid {
                    first: first_span,
                    other,
                });
            }
        }
        // Some sites carry the study UID in OBR-3 (Filler Order Number). If a
        // fallback is ever needed it belongs here, gated behind an explicit
        // option so the source stays auditable. Not implemented.
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

    /// The path that supplied `field`, when it was found.
    pub fn source_path(&self, field: OrderField) -> Option<&'static str> {
        self.sources
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, p)| *p)
    }
}
