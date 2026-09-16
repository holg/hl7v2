//! Measurements and nerve traces saved next to a study, keyed by SOP
//! Instance UID and frame so they survive re-scans, renamed files and a
//! different file order. Plain JSON; a DICOM Presentation State export is
//! the interoperable next step.

use crate::measure::Measurement;
use crate::nerve::NerveTrace;
use std::collections::BTreeMap;

/// Everything drawn on one image.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImageAnnotations {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<Measurement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nerves: Vec<NerveTrace>,
}

impl ImageAnnotations {
    pub fn is_empty(&self) -> bool {
        self.measurements.is_empty() && self.nerves.is_empty()
    }
}

/// The file: one entry per `<sop instance uid>#<frame>`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Annotations {
    pub version: u32,
    pub images: BTreeMap<String, ImageAnnotations>,
}

pub const VERSION: u32 = 1;

/// The key for one image.
pub fn key(sop_instance_uid: &str, frame: u32) -> String {
    format!("{}#{frame}", sop_instance_uid.trim())
}

impl Annotations {
    pub fn to_json(&self) -> String {
        let mut clean = self.clone();
        clean.version = VERSION;
        clean.images.retain(|_, a| !a.is_empty());
        serde_json::to_string_pretty(&clean).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Annotations, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_traces_and_drops_empty_images() {
        let mut a = Annotations::default();
        a.images
            .entry(key("1.2.3", 0))
            .or_default()
            .nerves
            .push(NerveTrace::new(vec![(1.0, 2.0), (3.0, 4.0)]));
        a.images
            .entry(key("1.2.3", 0))
            .or_default()
            .measurements
            .push(Measurement::Length {
                a: (0.0, 0.0),
                b: (3.0, 4.0),
            });
        a.images.entry(key("9.9", 2)).or_default();
        let json = a.to_json();
        assert!(json.contains("\"1.2.3#0\""));
        assert!(!json.contains("9.9#2"), "empty images are not written");
        let back = Annotations::from_json(&json).unwrap();
        assert_eq!(back.version, VERSION);
        assert_eq!(back.images.len(), 1);
        assert_eq!(back.images["1.2.3#0"].nerves[0].points.len(), 2);
        assert!(Annotations::from_json("nope").is_err());
    }
}
