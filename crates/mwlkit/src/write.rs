//! Encode a worklist item as a DICOM Part 10 file.
//!
//! A Modality Worklist item is a C-FIND response, not a stored object, so
//! the standard defines no file format for it and no SOP Instance UID.
//! Writing one to a file is a convention of this crate, useful for
//! inspection, for tests and for feeding a worklist SCP that reads files
//! (dcm4che's `wlmscpfs` and Orthanc's worklist plugin both do). The file
//! meta group declares:
//!
//! * Media Storage SOP Class UID: `1.2.840.10008.5.1.4.31`, the Modality
//!   Worklist Information Model - FIND SOP Class. It is the only UID the
//!   standard associates with this data set.
//! * Media Storage SOP Instance UID: a name-based `2.25.` UID derived from
//!   the Study Instance UID and the Scheduled Procedure Step IDs, so the
//!   same item always writes the same file identity.
//! * Transfer Syntax: Explicit VR Little Endian, `1.2.840.10008.1.2.1`.

use crate::uid::name_based_uid;
use crate::{MwlError, WorklistItem};
use dicom_core::header::HasLength;
use dicom_dictionary_std::{tags, uids};
use dicom_object::meta::FileMetaTableBuilder;
use dicom_object::{FileDicomObject, InMemDicomObject};

/// Namespace for the file's SOP Instance UID.
const FILE_NAMESPACE: &str = "mwlkit.file-instance-uid";

/// The item as a Part 10 file: 128-byte preamble, `DICM`, file meta group,
/// then the data set in Explicit VR Little Endian.
pub fn to_bytes(item: &WorklistItem) -> Result<Vec<u8>, MwlError> {
    let file = to_file(item)?;
    let mut out = Vec::new();
    file.write_all(&mut out)
        .map_err(|e| MwlError::Write(e.to_string()))?;
    Ok(out)
}

/// The item wrapped in a file meta group, for callers that want the
/// `dicom-object` type rather than bytes.
pub fn to_file(item: &WorklistItem) -> Result<FileDicomObject<InMemDicomObject>, MwlError> {
    let meta = FileMetaTableBuilder::new()
        .media_storage_sop_class_uid(uids::MODALITY_WORKLIST_INFORMATION_MODEL_FIND)
        .media_storage_sop_instance_uid(instance_uid(item))
        .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN);
    item.dataset
        .clone()
        .with_meta(meta)
        .map_err(|e| MwlError::Write(e.to_string()))
}

/// Deterministic file identity: Study Instance UID plus every SPS ID.
fn instance_uid(item: &WorklistItem) -> String {
    let mut name = item.study_uid.value.clone();
    if let Some(seq) = item
        .dataset
        .get(tags::SCHEDULED_PROCEDURE_STEP_SEQUENCE)
        .and_then(|e| e.items())
    {
        for sps in seq {
            if let Some(id) = sps.get(tags::SCHEDULED_PROCEDURE_STEP_ID) {
                if !id.is_empty() {
                    name.push('|');
                    name.push_str(id.value().to_str().unwrap_or_default().trim());
                }
            }
        }
    }
    name_based_uid(FILE_NAMESPACE, &name)
}
