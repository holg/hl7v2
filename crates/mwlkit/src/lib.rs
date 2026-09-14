//! Turn an HL7 v2 imaging order into a DICOM Modality Worklist item.
//!
//! A modality queries the Modality Worklist SCP with C-FIND before an
//! examination and receives one data set per scheduled procedure step,
//! defined by the Modality Worklist Information Model (DICOM PS3.4 Annex K,
//! Table K.6-1). It carries patient identification, the imaging service
//! request (order numbers, accession), the requested procedure (procedure
//! ID, Study Instance UID) and one or more Scheduled Procedure Steps. The
//! modality copies Patient ID, Accession Number, Study Instance UID,
//! Requested Procedure ID and SPS ID verbatim into every image it produces,
//! which makes this data set the hinge of RIS/PACS linkage.
//!
//! This crate maps an [`hl7kit::Message`] and the [`hl7kit::order::Order`]
//! extracted from it to that data set. The mapping follows IHE Radiology
//! Technical Framework Volume 2, transaction RAD-4 "Procedure Scheduled",
//! with dcm4che's HL7-to-MWL behaviour noted where it differs. Every choice
//! that comes from a document says which one in a comment in [`map`];
//! every choice that does not says so too.
//!
//! Nothing is invented silently. When the item needs a value the order does
//! not carry, it is omitted (Type 2 and 3 attributes), refused
//! ([`MwlError`]), or generated **and reported** as a [`Warning`]. The only
//! generated value is the Study Instance UID, governed by [`UidPolicy`].
//!
//! ```
//! use hl7kit::{order::Order, Message};
//! use mwlkit::{worklist_item, Input, Options};
//!
//! let text = "MSH|^~\\&|RIS|HOSP|PACS|HOSP|20260905121500||OMI^O23|1|P|2.5.1||||||UNICODE UTF-8\r\
//!             PID|1||4MR1^^^HOSP^MR||Doe^Jane||19700101|F\r\
//!             ORC|NW|ORD-1|FIL-1||SC||||20260905113000\r\
//!             TQ1|1||||||20260905113000\r\
//!             OBR|1|ORD-1|FIL-1|MR-HEAD^MRI head^L\r\
//!             IPC|ACC-1^HOSP|RP-1^HOSP|1.2.3^HOSP|SPS-1^HOSP|MR|||^|MR1AE\r";
//! let msg = Message::parse(text).unwrap();
//! let order = Order::extract(&msg);
//! let item = worklist_item(&Input { message: &msg, order: &order }, &Options::default()).unwrap();
//! assert_eq!(item.study_uid.value, "1.2.3");
//! assert!(item.warnings.is_empty());
//! ```
//!
//! What this crate does not do: MLLP, C-FIND, or any network code. It maps
//! and writes; serving the item to a modality is a different component.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod error;
mod input;
pub mod limits;
pub mod map;
pub mod pn;
pub mod uid;
pub mod write;

pub use error::MwlError;
pub use input::{Input, LengthPolicy, Options, UidPolicy};
pub use map::worklist_item;
pub use uid::{GeneratedFrom, StudyUidOrigin};
pub use write::to_bytes;

use dicom_core::Tag;
use dicom_object::InMemDicomObject;

/// A worklist item: the data set, where its Study Instance UID came from,
/// and everything the mapping had to decide on its own.
#[derive(Debug, Clone)]
pub struct WorklistItem {
    /// The Modality Worklist data set (PS3.4 Table K.6-1), without file meta.
    pub dataset: InMemDicomObject,
    /// The Study Instance UID and its provenance.
    pub study_uid: StudyUid,
    /// What was truncated, substituted, defaulted or generated. Returned,
    /// not logged: the caller shows it.
    pub warnings: Vec<Warning>,
}

/// The Study Instance UID of the item with its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudyUid {
    /// The UID as written to (0020,000D).
    pub value: String,
    /// Where it came from.
    pub origin: StudyUidOrigin,
}

/// A decision the mapping made that a reviewer should see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A value exceeded its VR length and was cut to `max` characters.
    Truncated {
        /// The attribute.
        tag: Tag,
        /// Characters allowed.
        max: usize,
        /// Characters in the message.
        actual: usize,
    },
    /// A Type 1 attribute was absent; `substituted` is what was used instead,
    /// `None` when it stayed absent because the standard allows it.
    MissingType1 {
        /// The attribute.
        tag: Tag,
        /// The substitute value, and where it came from.
        substituted: Option<String>,
    },
    /// PID-8 carried a value with no DICOM equivalent; (0010,0040) is empty.
    SexMappedToOther {
        /// The HL7 value.
        hl7: String,
    },
    /// The name carried a prefix or suffix, which sit in different positions
    /// in HL7 XPN and DICOM PN; they were moved.
    NameComponentsReordered,
    /// The order carried no Study Instance UID; one was generated.
    StudyUidGenerated(GeneratedFrom),
    /// The message carried no Scheduled Station AE Title; the caller's
    /// default was written.
    StationAeDefaulted(String),
    /// MSH-18 was absent or unknown; the character set was assumed.
    CharacterSetAssumed {
        /// MSH-18 as found.
        msh18: Option<String>,
        /// The DICOM term written to (0008,0005).
        used: String,
    },
    /// OBR-24 carried an HL7 table 0074 code that was translated to a DICOM
    /// modality.
    ModalityTranslated {
        /// The HL7 value.
        from: String,
        /// The DICOM value written.
        to: String,
    },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::Truncated { tag, max, actual } => write!(
                f,
                "{tag} truncated from {actual} to {max} characters (VR length limit)"
            ),
            Warning::MissingType1 {
                tag,
                substituted: Some(s),
            } => write!(f, "{tag} is Type 1 but absent from the order; {s} was used"),
            Warning::MissingType1 {
                tag,
                substituted: None,
            } => write!(f, "{tag} is Type 1 but absent from the order"),
            Warning::SexMappedToOther { hl7 } => write!(
                f,
                "PID-8 {hl7:?} has no DICOM equivalent; Patient's Sex left empty"
            ),
            Warning::NameComponentsReordered => write!(
                f,
                "name prefix and suffix moved: HL7 XPN and DICOM PN order them differently"
            ),
            Warning::StudyUidGenerated(from) => write!(
                f,
                "the order carried no Study Instance UID; one was generated {from}. \
                 The entry goes to the modality with a UID the RIS does not know."
            ),
            Warning::StationAeDefaulted(ae) => write!(
                f,
                "no Scheduled Station AE Title in the message (IPC-9); {ae:?} was assumed"
            ),
            Warning::CharacterSetAssumed { msh18, used } => match msh18 {
                Some(m) => write!(
                    f,
                    "MSH-18 {m:?} is not a known character set; {used} assumed"
                ),
                None => write!(f, "MSH-18 absent; {used} assumed"),
            },
            Warning::ModalityTranslated { from, to } => write!(
                f,
                "OBR-24 {from:?} (HL7 table 0074) translated to modality {to}"
            ),
        }
    }
}
