//! DICOM loading, tag listing and pixel decoding. Host-testable; no `web-sys`.

pub mod color;
pub mod load;
pub mod pixels;
pub mod series;
pub mod sr;
pub mod study;
pub mod tags;
pub mod transfer_syntax;

pub use load::load;
pub use pixels::{decode_frame, fallback_window, Frame, Pixels};
pub use series::{FileEntry, Series, StudySet};
pub use study::Study;
pub use tags::{tag_rows, TagRow};

#[cfg(test)]
mod tests {
    //! End-to-end on the host: build a Part 10 file in memory, then run the
    //! same load / study / tags / decode path the browser uses.

    use super::load::load_header;
    use super::testutil::Synthetic;
    use super::*;
    use crate::error::AppError;

    fn synthetic(photometric: &str, with_window: bool) -> Vec<u8> {
        Synthetic {
            photometric: photometric.into(),
            with_window,
            ..Synthetic::default()
        }
        .build()
    }

    fn synthetic_ts(ts: &str, photometric: &str, with_window: bool) -> Vec<u8> {
        Synthetic {
            ts: ts.into(),
            photometric: photometric.into(),
            with_window,
            ..Synthetic::default()
        }
        .build()
    }

    fn synthetic_full(
        ts: &str,
        photometric: &str,
        with_window: bool,
        rgb: Option<Vec<u8>>,
    ) -> Vec<u8> {
        Synthetic {
            ts: ts.into(),
            photometric: photometric.into(),
            with_window,
            rgb,
            ..Synthetic::default()
        }
        .build()
    }

    #[test]
    fn load_study_tags_and_decode() {
        let bytes = synthetic("MONOCHROME2", true);
        assert_eq!(&bytes[128..132], b"DICM");
        let obj = load(&bytes).unwrap();

        let study = Study::from_object(&obj);
        assert_eq!(study.patient_id.as_deref(), Some("4MR1"), "padding trimmed");
        assert_eq!(study.accession_number.as_deref(), Some("ACC-2026-0001"));
        assert_eq!(
            study.study_uid.as_deref(),
            Some("1.3.6.1.4.1.5962.1.2.4.20040826185059.5457")
        );
        assert_eq!(study.transfer_syntax, "1.2.840.10008.1.2.1");
        assert_eq!((study.rows, study.cols), (2, 3));
        assert_eq!(study.modality.as_deref(), Some("CT"));

        let rows = tag_rows(&obj);
        assert!(rows
            .iter()
            .any(|r| r.tag == "(0002,0010)" && r.value.starts_with("1.2.840.10008.1.2.1")));
        let pid = rows.iter().find(|r| r.keyword == "PatientID").unwrap();
        assert_eq!(
            (pid.vr.as_str(), pid.value.as_str(), pid.depth),
            ("LO", "4MR1", 0)
        );
        let px = rows.iter().find(|r| r.keyword == "PixelData").unwrap();
        assert_eq!(px.value, "<12 bytes>");

        let frame = decode_frame(&obj, 0).unwrap();
        assert_eq!((frame.width, frame.height), (3, 2));
        assert_eq!(
            frame.pixels,
            Pixels::Gray(vec![-1024.0, 0.0, 1024.0, 2048.0, 3072.0, 64311.0])
        );
        assert_eq!(frame.value_range, (-1024.0, 64311.0));
        assert_eq!(
            frame.default_window,
            Some((40.0, 400.0)),
            "first of multi-valued"
        );
        assert_eq!(frame.rescale, (1.0, -1024.0));
        assert!(!frame.inverted);
        assert_eq!((frame.frame_index, frame.frame_count), (0, 1));
        assert_eq!(frame.bits_stored, 16);
    }

    #[test]
    fn monochrome1_sets_flag_without_inverting_buffer() {
        let obj = load(&synthetic("MONOCHROME1", false)).unwrap();
        let frame = decode_frame(&obj, 0).unwrap();
        assert!(frame.inverted);
        assert!(matches!(&frame.pixels, Pixels::Gray(d) if d[0] == -1024.0));
        assert_eq!(frame.default_window, None);
        assert_eq!(
            fallback_window(frame.value_range),
            ((-1024.0 + 64311.0) / 2.0, 64311.0 + 1024.0)
        );
    }

    #[test]
    fn multi_frame_decodes_the_requested_frame() {
        let obj = load(
            &Synthetic {
                frames: 3,
                ..Synthetic::default()
            }
            .build(),
        )
        .unwrap();
        let f0 = decode_frame(&obj, 0).unwrap();
        let f2 = decode_frame(&obj, 2).unwrap();
        assert_eq!((f0.frame_index, f0.frame_count), (0, 3));
        assert_eq!((f2.frame_index, f2.frame_count), (2, 3));
        assert!(matches!(&f0.pixels, Pixels::Gray(d) if d[1] == 0.0));
        assert!(matches!(&f2.pixels, Pixels::Gray(d) if d[1] == 200.0));
        assert!(decode_frame(&obj, 3).is_err());
        let header = load_header(
            &Synthetic {
                frames: 3,
                ..Synthetic::default()
            }
            .build(),
        )
        .unwrap();
        assert!(header.get(dicom_dictionary_std::tags::PIXEL_DATA).is_none());
        assert_eq!(Study::from_object(&header).rows, 2);
    }

    #[test]
    fn file_without_preamble_loads() {
        let bytes = synthetic("MONOCHROME2", true);
        let obj = load(&bytes[128..]).unwrap();
        assert_eq!(Study::from_object(&obj).patient_id.as_deref(), Some("4MR1"));
    }

    #[test]
    fn rgb_and_ybr_decode_to_rgba() {
        let samples = vec![
            255, 0, 0, 0, 255, 0, 0, 0, 255, 10, 20, 30, 0, 0, 0, 255, 255, 255,
        ];
        let obj = load(&synthetic_full(
            "1.2.840.10008.1.2.1",
            "RGB",
            false,
            Some(samples.clone()),
        ))
        .unwrap();
        let frame = decode_frame(&obj, 0).unwrap();
        assert_eq!((frame.width, frame.height, frame.bits_stored), (3, 2, 8));
        let Pixels::Rgba(rgba) = &frame.pixels else {
            panic!("expected colour")
        };
        assert_eq!(&rgba[..8], &[255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(&rgba[20..24], &[255, 255, 255, 255]);
        assert_eq!(frame.default_window, None);

        let obj = load(&synthetic_full(
            "1.2.840.10008.1.2.1",
            "YBR_FULL",
            false,
            Some(vec![128u8; 18]),
        ))
        .unwrap();
        let Pixels::Rgba(rgba) = decode_frame(&obj, 0).unwrap().pixels else {
            panic!()
        };
        assert_eq!(&rgba[..4], &[128, 128, 128, 255]);
    }

    #[test]
    fn palette_colour_is_looked_up() {
        // Indices 0,1024,2048,3072,4096,65535 with first mapped 1024 and 3
        // entries: 0 and 1024 hit entry 0, everything from 1026 up clamps to
        // entry 2. Entry 1 (red) is only reachable by index 1025.
        let obj = load(&synthetic("PALETTE COLOR", false)).unwrap();
        let frame = decode_frame(&obj, 0).unwrap();
        let Pixels::Rgba(rgba) = &frame.pixels else {
            panic!("expected colour")
        };
        assert_eq!(&rgba[0..4], &[0, 0, 128, 255]);
        assert_eq!(&rgba[4..8], &[0, 0, 128, 255]);
        assert_eq!(&rgba[8..12], &[0, 255, 0, 255]);
        assert_eq!(&rgba[20..24], &[0, 255, 0, 255]);
        assert_eq!(frame.value_range, (0.0, 255.0));
        assert!(!frame.inverted);
    }

    #[test]
    fn unknown_photometric_is_reported_not_rendered() {
        let obj = load(&synthetic("HSV", false)).unwrap();
        match decode_frame(&obj, 0) {
            Err(AppError::UnsupportedPhotometric { photometric }) => {
                assert!(photometric.contains("HSV"), "{photometric}")
            }
            other => panic!("expected UnsupportedPhotometric, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_transfer_syntax_names_reason() {
        // Native pixel data under a JPEG 2000 transfer syntax is not a valid
        // file, but it is exactly what the error path must handle gracefully.
        let obj = load(&synthetic_ts(
            "1.2.840.10008.1.2.4.90",
            "MONOCHROME2",
            false,
        ))
        .unwrap();
        match decode_frame(&obj, 0) {
            Err(AppError::UnsupportedTransferSyntax { uid, name, reason }) => {
                assert_eq!(uid, "1.2.840.10008.1.2.4.90");
                assert_eq!(name, "JPEG 2000 Lossless");
                assert!(reason.contains("openjp2"));
            }
            other => panic!("expected UnsupportedTransferSyntax, got {other:?}"),
        }
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Build Part 10 files in memory so the whole DICOM path runs on the host.

    use dicom_core::{DataElement, PrimitiveValue, VR};
    use dicom_dictionary_std::tags;
    use dicom_object::meta::FileMetaTableBuilder;
    use dicom_object::FileDicomObject;

    /// A 3x2 image. Greyscale unless `rgb` is given.
    pub struct Synthetic {
        pub ts: String,
        pub photometric: String,
        pub with_window: bool,
        /// Interleaved 8-bit samples for a 3x2 RGB-type image.
        pub rgb: Option<Vec<u8>>,
        pub frames: u32,
        pub series_uid: String,
        pub series_number: Option<i32>,
        pub instance_number: Option<i32>,
        pub position: Option<[f64; 3]>,
        pub orientation: Option<[f64; 6]>,
    }

    impl Default for Synthetic {
        fn default() -> Self {
            Synthetic {
                ts: "1.2.840.10008.1.2.1".into(),
                photometric: "MONOCHROME2".into(),
                with_window: false,
                rgb: None,
                frames: 1,
                series_uid: "1.2.3.4.5".into(),
                series_number: None,
                instance_number: None,
                position: None,
                orientation: None,
            }
        }
    }

    impl Synthetic {
        pub fn build(self) -> Vec<u8> {
            let meta = FileMetaTableBuilder::new()
                .transfer_syntax(&self.ts)
                .media_storage_sop_class_uid("1.2.840.10008.5.1.4.1.1.2")
                .media_storage_sop_instance_uid("1.2.3.4.5.6")
                .build()
                .unwrap();
            let mut obj = FileDicomObject::new_empty_with_meta(meta);
            let mut put = |tag, vr, v: PrimitiveValue| {
                obj.put(DataElement::new(tag, vr, v));
            };
            let ds = |v: &[f64]| -> PrimitiveValue {
                PrimitiveValue::Strs(v.iter().map(|x| x.to_string()).collect::<Vec<_>>().into())
            };
            put(tags::SOP_INSTANCE_UID, VR::UI, "1.2.3.4.5.6".into());
            put(tags::PATIENT_ID, VR::LO, "4MR1 ".into());
            put(tags::ACCESSION_NUMBER, VR::SH, "ACC-2026-0001".into());
            put(
                tags::STUDY_INSTANCE_UID,
                VR::UI,
                "1.3.6.1.4.1.5962.1.2.4.20040826185059.5457".into(),
            );
            put(
                tags::SERIES_INSTANCE_UID,
                VR::UI,
                self.series_uid.as_str().into(),
            );
            put(tags::SERIES_DESCRIPTION, VR::LO, "Axial".into());
            if let Some(n) = self.series_number {
                put(tags::SERIES_NUMBER, VR::IS, n.to_string().into());
            }
            if let Some(n) = self.instance_number {
                put(tags::INSTANCE_NUMBER, VR::IS, n.to_string().into());
            }
            if let Some(p) = self.position {
                put(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&p));
            }
            if let Some(o) = self.orientation {
                put(tags::IMAGE_ORIENTATION_PATIENT, VR::DS, ds(&o));
            }
            put(tags::MODALITY, VR::CS, "CT".into());
            put(tags::ROWS, VR::US, 2u16.into());
            put(tags::COLUMNS, VR::US, 3u16.into());
            let bits: u16 = if self.rgb.is_some() { 8 } else { 16 };
            put(tags::BITS_ALLOCATED, VR::US, bits.into());
            put(tags::BITS_STORED, VR::US, bits.into());
            put(tags::HIGH_BIT, VR::US, (bits - 1).into());
            put(tags::PIXEL_REPRESENTATION, VR::US, 0u16.into());
            put(
                tags::SAMPLES_PER_PIXEL,
                VR::US,
                if self.rgb.is_some() { 3u16 } else { 1u16 }.into(),
            );
            if self.rgb.is_some() {
                put(tags::PLANAR_CONFIGURATION, VR::US, 0u16.into());
            }
            put(
                tags::PHOTOMETRIC_INTERPRETATION,
                VR::CS,
                self.photometric.as_str().into(),
            );
            if self.photometric == "PALETTE COLOR" {
                // 3 entries of 16 bits, first mapped value 1024.
                for (desc, data, values) in [
                    (
                        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
                        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                        [0x0000u16, 0xFF00, 0x0000],
                    ),
                    (
                        tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
                        tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                        [0x0000u16, 0x0000, 0xFF00],
                    ),
                    (
                        tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
                        tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                        [0x8000u16, 0x0000, 0x0000],
                    ),
                ] {
                    put(
                        desc,
                        VR::US,
                        PrimitiveValue::U16(vec![3u16, 1024, 16].into()),
                    );
                    put(data, VR::OW, PrimitiveValue::U16(values.to_vec().into()));
                }
            }
            put(tags::RESCALE_SLOPE, VR::DS, "1".into());
            put(tags::RESCALE_INTERCEPT, VR::DS, "-1024".into());
            if self.with_window {
                put(tags::WINDOW_CENTER, VR::DS, "40\\80".into());
                put(tags::WINDOW_WIDTH, VR::DS, "400\\800".into());
            }
            if self.frames > 1 {
                put(
                    tags::NUMBER_OF_FRAMES,
                    VR::IS,
                    self.frames.to_string().into(),
                );
            }
            match self.rgb {
                Some(samples) => put(tags::PIXEL_DATA, VR::OB, PrimitiveValue::U8(samples.into())),
                None => {
                    // Frame k holds the base pattern plus k*100.
                    let base = [0u16, 1024, 2048, 3072, 4096, 65335];
                    let pixels: Vec<u16> = (0..self.frames)
                        .flat_map(|k| base.iter().map(move |&v| v + (k as u16) * 100))
                        .collect();
                    put(tags::PIXEL_DATA, VR::OW, PrimitiveValue::U16(pixels.into()));
                }
            }
            let mut out = Vec::new();
            obj.write_all(&mut out).unwrap();
            out
        }
    }
}
