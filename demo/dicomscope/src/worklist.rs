//! The Modality Worklist item the order would have produced, built with
//! `mwlkit`. Host-testable: no browser types.
//!
//! The demo opts into truncation so that a too-long accession number
//! becomes a visible broken link in the chain view instead of an error;
//! `mwlkit`'s own default refuses.

use crate::dicom::tags::{dataset_rows, TagRow};
use hl7kit::order::Order;
use hl7kit::Message;
use mwlkit::{to_bytes, worklist_item, Input, LengthPolicy, MwlError, Options, WorklistItem};

/// The AE title written when the order carries none (IPC-9). Reported by
/// `mwlkit` as a warning, so the panel shows it was assumed.
pub const DEMO_STATION_AE: &str = "DICOMSCOPE";

/// The item, its data set as display rows, and the Part 10 bytes.
#[derive(Debug, Clone)]
pub struct WorklistOutput {
    pub item: WorklistItem,
    pub rows: Vec<TagRow>,
    pub bytes: Vec<u8>,
}

impl PartialEq for WorklistOutput {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

/// What the demo asks of `mwlkit`.
pub fn demo_options() -> Options {
    Options {
        default_station_ae: Some(DEMO_STATION_AE.to_string()),
        length_policy: LengthPolicy::TruncateAndWarn,
        ..Options::default()
    }
}

pub fn build(message: &Message, order: &Order) -> Result<WorklistOutput, MwlError> {
    let item = worklist_item(&Input { message, order }, &demo_options())?;
    let bytes = to_bytes(&item)?;
    let rows = dataset_rows(&item.dataset);
    Ok(WorklistOutput { item, rows, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_dictionary_std::tags;
    use mwlkit::{StudyUidOrigin, Warning};

    const ORM: &str = include_str!("../../../samples/order.hl7");

    #[test]
    fn sample_order_becomes_an_item_with_the_demo_station() {
        let msg = Message::parse(ORM).unwrap();
        let order = Order::extract(&msg);
        let out = build(&msg, &order).unwrap();
        assert!(matches!(
            out.item.study_uid.origin,
            StudyUidOrigin::FromOrder(_)
        ));
        assert!(out
            .item
            .warnings
            .contains(&Warning::StationAeDefaulted(DEMO_STATION_AE.into())));
        assert!(out
            .rows
            .iter()
            .any(|r| r.keyword == "ScheduledStationAETitle" && r.value.trim() == DEMO_STATION_AE));
        assert!(out.rows.iter().any(|r| r.keyword == "StudyInstanceUID"));
        assert_eq!(&out.bytes[128..132], b"DICM");
        assert!(out.item.dataset.get(tags::ACCESSION_NUMBER).is_some());
    }

    #[test]
    fn long_accession_is_truncated_not_refused() {
        let text = ORM.replace("|ACC-2026-0001|", "|ACC-2026-00000000001|");
        let msg = Message::parse(&text).unwrap();
        let order = Order::extract(&msg);
        let out = build(&msg, &order).unwrap();
        assert!(out.item.warnings.iter().any(
            |w| matches!(w, Warning::Truncated { tag, .. } if *tag == tags::ACCESSION_NUMBER)
        ));
    }
}
