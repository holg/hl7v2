//! What the mapping reads and the policies the caller chooses.

use hl7kit::order::Order;
use hl7kit::Message;

/// The order and the message it was extracted from. The item needs more
/// than the four linkage fields, so the mapping reads the rest from the
/// message itself through `hl7kit` paths; `Order` stays about linkage.
pub struct Input<'m> {
    /// The parsed message.
    pub message: &'m Message,
    /// The identifiers `hl7kit` extracted from it.
    pub order: &'m Order,
}

/// Decisions that belong to the caller, never to the mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// What to do when the order carries no Study Instance UID.
    pub uid_policy: UidPolicy,
    /// What to do with SH, LO and PN values that exceed their VR length.
    /// AE and UI are never truncated regardless of this setting.
    pub length_policy: LengthPolicy,
    /// Scheduled Station AE Title (0040,0001) when the message carries none
    /// in IPC-9. It is Type 1, so without a value the item is refused;
    /// supplying one is recorded as [`crate::Warning::StationAeDefaulted`].
    pub default_station_ae: Option<String>,
    /// (0008,0005) Specific Character Set to declare. `None` reads MSH-18
    /// and falls back to `ISO_IR 192` (UTF-8) with a warning.
    pub character_set: Option<String>,
}

impl Default for Options {
    /// dcm4che-style UID generation, no truncation, no station default,
    /// character set from the message.
    fn default() -> Self {
        Options {
            uid_policy: UidPolicy::Dcm4cheStyle,
            length_policy: LengthPolicy::Refuse,
            default_station_ae: None,
            character_set: None,
        }
    }
}

/// How a missing Study Instance UID is handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UidPolicy {
    /// Derive deterministically from the Requested Procedure ID, else from
    /// the Accession Number, else fail with [`crate::MwlError::NoStudyUid`].
    /// This mirrors dcm4che's HL7-to-MWL behaviour, which derives a
    /// name-based UID from those identifiers. The same order always yields
    /// the same UID. It is dcm4che-style, not dcm4che-identical: dcm4che
    /// uses UUID version 5, this crate a version 8 hash.
    Dcm4cheStyle,
    /// Use the 128 bits the caller supplies (from a real random source) as
    /// a `2.25.` UUID-derived UID. The crate has no random source of its
    /// own and will not pretend to.
    Random(u128),
    /// Never generate; fail with [`crate::MwlError::NoStudyUid`].
    Refuse,
}

/// What to do with SH, LO and PN values longer than their VR allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthPolicy {
    /// Fail with [`crate::MwlError::TooLong`]. A cut accession number is a
    /// different key from the one the RIS holds, so this is the default.
    Refuse,
    /// Cut to the limit and record [`crate::Warning::Truncated`]. Use it to
    /// see what a lenient interface would have produced.
    TruncateAndWarn,
}
