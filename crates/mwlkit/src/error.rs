//! Why an order could not become a worklist item.

use dicom_core::Tag;
use std::fmt;

/// A Type 1 attribute that the mapping refuses to invent, or a value that
/// would be wrong if cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MwlError {
    /// PID-3.1 is absent or empty; there is no patient to schedule.
    MissingPatientId,
    /// Neither IPC-5 nor a translatable OBR-24 gave a modality. `found` is
    /// what OBR-24 carried, if anything.
    MissingModality {
        /// The untranslatable OBR-24 value.
        found: Option<String>,
    },
    /// No IPC-9 and no default station AE title in the options.
    MissingStationAe,
    /// Neither OBR-19, IPC-2 nor a placer order number (ORC-2 / OBR-2)
    /// gives a Requested Procedure ID, which is Type 1.
    MissingRequestedProcedureId,
    /// Neither OBR-27 nor TQ1-7 carries a start date and time.
    MissingStartDateTime,
    /// The order carries no Study Instance UID and none was generated: the
    /// policy is `Refuse`, or it is `Dcm4cheStyle` and the order has neither a
    /// Requested Procedure ID nor an Accession Number to derive one from.
    NoStudyUid,
    /// A value exceeds its VR length and the policy (or the VR) forbids
    /// truncation: AE and UI are never cut, SH/LO/PN only under
    /// `LengthPolicy::TruncateAndWarn`.
    TooLong {
        /// The attribute.
        tag: Tag,
        /// Characters allowed.
        max: usize,
        /// Characters found.
        actual: usize,
    },
    /// The data set could not be encoded.
    Write(String),
}

impl fmt::Display for MwlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MwlError::MissingPatientId => write!(f, "PID-3.1 (Patient ID) is absent; it is Type 1 in the worklist"),
            MwlError::MissingModality { found: Some(v) } => write!(
                f,
                "no modality: IPC-5 absent and OBR-24 {v:?} is not a DICOM modality or a translatable HL7 table 0074 code"
            ),
            MwlError::MissingModality { found: None } => {
                write!(f, "no modality: neither IPC-5 nor OBR-24 is present")
            }
            MwlError::MissingStationAe => write!(
                f,
                "no Scheduled Station AE Title: IPC-9 absent and no default supplied"
            ),
            MwlError::MissingRequestedProcedureId => write!(
                f,
                "no Requested Procedure ID: OBR-19, IPC-2 and the placer order number (ORC-2, OBR-2) are all absent"
            ),
            MwlError::MissingStartDateTime => write!(
                f,
                "no scheduled start: neither OBR-27.4 nor TQ1-7 carries a date and time"
            ),
            MwlError::NoStudyUid => write!(
                f,
                "the order carries no Study Instance UID (IPC-3, ZDS-1 or OBX) and none was generated: either the policy refuses, or there is no Requested Procedure ID and no Accession Number to derive one from"
            ),
            MwlError::TooLong { tag, max, actual } => write!(
                f,
                "{tag} is {actual} characters, the VR allows {max}; not truncated because the result would be a different identifier"
            ),
            MwlError::Write(e) => write!(f, "cannot encode the worklist item: {e}"),
        }
    }
}

impl std::error::Error for MwlError {}
