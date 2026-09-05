//! A bag of files (folder, multi-select or zip) turned into series of
//! sortable slices.
//!
//! Scanning is header-only and, for zip entries, streaming: each entry is
//! inflated only as far as the header reader consumes it, a few kilobytes
//! rather than the whole file. Archive bytes are kept as they are and an
//! entry is inflated again when its slice is shown, so memory is the input
//! plus one frame. Pixel data is decoded on demand by
//! [`crate::dicom::pixels::decode_frame`].

use crate::dicom::load::load_header_from;
use crate::dicom::sr::{document_kind, document_title, DocumentKind};
use crate::dicom::study::{number, string};
use dicom_core::Tag;
use dicom_dictionary_std::tags;
use dicom_object::DefaultDicomObject;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{Cursor, Read};

const DICOMDIR_SOP_CLASS: &str = "1.2.840.10008.1.3.10";
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// Where a file's bytes live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Read directly (a picked or dropped file).
    Bytes(Vec<u8>),
    /// Entry `index` of `StudySet::archives[archive]`, inflated on demand.
    Archived { archive: usize, index: usize },
}

/// One input file, as read from the browser or found inside a zip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub source: Source,
}

impl FileEntry {
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> FileEntry {
        FileEntry {
            name: name.into(),
            source: Source::Bytes(bytes),
        }
    }
}

/// A file that was not turned into slices, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

/// One displayable frame: a file and a frame index within it.
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    pub file: usize,
    pub frame: u32,
    pub instance_number: Option<i32>,
    /// Image Position (Patient) projected onto the slice normal, when known.
    pub position: Option<f64>,
    pub sop_instance_uid: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub uid: String,
    pub number: Option<i32>,
    pub description: String,
    pub modality: String,
    pub rows: u32,
    pub cols: u32,
    pub slices: Vec<Slice>,
}

impl Series {
    /// `3: CT  Axial 2mm (140)`
    pub fn label(&self) -> String {
        let mut s = String::new();
        if let Some(n) = self.number {
            s.push_str(&format!("{n}: "));
        }
        s.push_str(&self.modality);
        if !self.description.is_empty() {
            s.push_str("  ");
            s.push_str(&self.description);
        }
        s.push_str(&format!(" ({})", self.slices.len()));
        s
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StudySet {
    /// Zip archives as given, never expanded in full.
    pub archives: Vec<Vec<u8>>,
    /// Every file that was looked at, including skipped ones.
    pub files: Vec<FileEntry>,
    /// Sorted by series number, then UID.
    pub series: Vec<Series>,
    pub skipped: Vec<Skipped>,
    /// Structured Reports, Key Object Selections and Encapsulated PDFs found
    /// alongside the images, in scan order.
    pub documents: Vec<Document>,
    pub study_uid: Option<String>,
    /// Operating-system metadata entries that were ignored without parsing
    /// (`__MACOSX/`, `._*`, `.DS_Store`, `Thumbs.db`).
    pub ignored_metadata: usize,
}

impl StudySet {
    /// Read every header, group and sort. Zips are scanned entry by entry.
    pub fn scan(inputs: Vec<FileEntry>) -> StudySet {
        let mut set = StudySet::default();
        let mut groups: BTreeMap<String, Series> = BTreeMap::new();

        for input in inputs {
            if is_metadata_name(&input.name) {
                set.ignored_metadata += 1;
                continue;
            }
            match input.source {
                Source::Bytes(bytes) if is_zip(&bytes) => {
                    let archive = set.archives.len();
                    set.archives.push(bytes);
                    set.scan_archive(&input.name, archive, &mut groups);
                }
                Source::Bytes(bytes) => {
                    let outcome = classify(&input.name, Cursor::new(&bytes));
                    let file = set.files.len();
                    set.files.push(FileEntry::new(input.name.clone(), bytes));
                    set.record(file, &input.name, outcome, &mut groups);
                }
                Source::Archived { .. } => {
                    // Callers hand in fresh bytes; archived entries are only
                    // ever produced here.
                    set.skipped.push(Skipped {
                        name: input.name,
                        reason: "archived entries cannot be re-scanned".into(),
                    });
                }
            }
        }

        let mut series: Vec<Series> = groups.into_values().collect();
        for s in &mut series {
            sort_slices(&mut s.slices, &set.files);
        }
        series.sort_by(|a, b| a.number.cmp(&b.number).then_with(|| a.uid.cmp(&b.uid)));
        set.series = series;
        set
    }

    fn scan_archive(
        &mut self,
        zip_name: &str,
        archive: usize,
        groups: &mut BTreeMap<String, Series>,
    ) {
        let bytes = &self.archives[archive];
        let mut zip = match zip::ZipArchive::new(Cursor::new(bytes.as_slice())) {
            Ok(z) => z,
            Err(e) => {
                self.skipped.push(Skipped {
                    name: zip_name.to_string(),
                    reason: format!("not a readable zip: {e}"),
                });
                return;
            }
        };
        let mut found = Vec::new();
        for index in 0..zip.len() {
            let entry = match zip.by_index(index) {
                Ok(e) => e,
                Err(e) => {
                    self.skipped.push(Skipped {
                        name: format!("{zip_name}#{index}"),
                        reason: format!("unreadable zip entry: {e}"),
                    });
                    continue;
                }
            };
            if entry.is_dir() {
                continue;
            }
            let name = format!("{zip_name}/{}", entry.name());
            if is_metadata_name(entry.name()) {
                self.ignored_metadata += 1;
                continue;
            }
            let outcome = if entry.name().ends_with(".zip") || entry.name().ends_with(".ZIP") {
                Err("nested zip archives are not expanded".to_string())
            } else {
                // `entry` is a streaming reader: only the header is inflated.
                classify(&name, entry)
            };
            found.push((name, index, outcome));
        }
        for (name, index, outcome) in found {
            let file = self.files.len();
            self.files.push(FileEntry {
                name: name.clone(),
                source: Source::Archived { archive, index },
            });
            self.record(file, &name, outcome, groups);
        }
    }

    fn record(
        &mut self,
        file: usize,
        name: &str,
        outcome: Result<Classified, String>,
        groups: &mut BTreeMap<String, Series>,
    ) {
        match outcome {
            Err(reason) => self.skipped.push(Skipped {
                name: name.to_string(),
                reason,
            }),
            Ok(Classified::Document {
                kind,
                title,
                series_number,
                study_uid,
            }) => {
                if self.study_uid.is_none() {
                    self.study_uid = study_uid;
                }
                self.documents.push(Document {
                    file,
                    kind,
                    title,
                    series_number,
                });
            }
            Ok(Classified::Image(meta, frames)) => {
                if self.study_uid.is_none() {
                    self.study_uid = meta.study_uid.clone();
                }
                let series = groups
                    .entry(meta.series_uid.clone())
                    .or_insert_with(|| Series {
                        uid: meta.series_uid.clone(),
                        number: meta.series_number,
                        description: meta.series_description.clone(),
                        modality: meta.modality.clone(),
                        rows: meta.rows,
                        cols: meta.cols,
                        slices: Vec::new(),
                    });
                for frame in 0..frames {
                    series.slices.push(Slice {
                        file,
                        frame,
                        instance_number: meta.instance_number,
                        position: meta.position,
                        sop_instance_uid: meta.sop_instance_uid.clone(),
                    });
                }
            }
        }
    }

    /// The full bytes of a file, inflating a zip entry if needed.
    pub fn bytes(&self, file: usize) -> Result<Cow<'_, [u8]>, String> {
        let entry = self.files.get(file).ok_or("no such file")?;
        match &entry.source {
            Source::Bytes(b) => Ok(Cow::Borrowed(b.as_slice())),
            Source::Archived { archive, index } => {
                let bytes = self.archives.get(*archive).ok_or("no such archive")?;
                let mut zip = zip::ZipArchive::new(Cursor::new(bytes.as_slice()))
                    .map_err(|e| format!("{}: {e}", entry.name))?;
                let mut z = zip
                    .by_index(*index)
                    .map_err(|e| format!("{}: {e}", entry.name))?;
                let mut out = Vec::with_capacity(z.size() as usize);
                z.read_to_end(&mut out)
                    .map_err(|e| format!("{}: {e}", entry.name))?;
                Ok(Cow::Owned(out))
            }
        }
    }

    pub fn slice_count(&self) -> usize {
        self.series.iter().map(|s| s.slices.len()).sum()
    }

    /// True when there is nothing to show: no image series and no documents.
    pub fn is_empty(&self) -> bool {
        self.series.is_empty() && self.documents.is_empty()
    }

    pub fn slice(&self, series: usize, slice: usize) -> Option<(&Series, &Slice)> {
        let s = self.series.get(series)?;
        Some((s, s.slices.get(slice)?))
    }

    /// Bytes held in memory: archives plus directly read files.
    pub fn resident_bytes(&self) -> usize {
        self.archives.iter().map(Vec::len).sum::<usize>()
            + self
                .files
                .iter()
                .map(|f| match &f.source {
                    Source::Bytes(b) => b.len(),
                    Source::Archived { .. } => 0,
                })
                .sum::<usize>()
    }
}

/// `__MACOSX/` resource forks, `._*` AppleDouble files, `.DS_Store`,
/// `Thumbs.db`: never DICOM, never worth a parse attempt.
pub fn is_metadata_name(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    name.split(['/', '\\']).any(|part| part == "__MACOSX")
        || base.starts_with("._")
        || base == ".DS_Store"
        || base.eq_ignore_ascii_case("Thumbs.db")
        || base == "desktop.ini"
}

/// Slices sort by position along the normal when every slice has one, else
/// by instance number, then frame, then file name. This is what makes a
/// folder of arbitrarily named files scroll in anatomical order.
fn sort_slices(slices: &mut [Slice], files: &[FileEntry]) {
    let all_positioned = slices.iter().all(|s| s.position.is_some());
    slices.sort_by(|a, b| {
        let by_position = if all_positioned {
            a.position
                .unwrap_or(0.0)
                .total_cmp(&b.position.unwrap_or(0.0))
        } else {
            std::cmp::Ordering::Equal
        };
        by_position
            .then_with(|| a.instance_number.cmp(&b.instance_number))
            .then_with(|| a.frame.cmp(&b.frame))
            .then_with(|| {
                let name = |s: &Slice| files.get(s.file).map(|f| f.name.as_str()).unwrap_or("");
                name(a).cmp(name(b))
            })
    });
}

/// A non-image instance worth showing: report or PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub file: usize,
    pub kind: DocumentKind,
    pub title: String,
    pub series_number: Option<i32>,
}

impl Document {
    /// `SR: CT report` or `PDF: Discharge letter`.
    pub fn label(&self) -> String {
        format!("{}: {}", self.kind.label(), self.title)
    }
}

/// What a scanned file turned out to be.
enum Classified {
    Image(Meta, u32),
    Document {
        kind: DocumentKind,
        title: String,
        series_number: Option<i32>,
        study_uid: Option<String>,
    },
}

struct Meta {
    study_uid: Option<String>,
    series_uid: String,
    series_number: Option<i32>,
    series_description: String,
    modality: String,
    instance_number: Option<i32>,
    position: Option<f64>,
    sop_instance_uid: String,
    rows: u32,
    cols: u32,
}

/// Header-only look at one file. `Err` is the reason it is not a displayable
/// image instance.
fn classify<R: Read>(name: &str, reader: R) -> Result<Classified, String> {
    if name
        .rsplit(['/', '\\'])
        .next()
        .map(|n| n.eq_ignore_ascii_case("DICOMDIR"))
        == Some(true)
    {
        return Err("DICOMDIR index, not an image".into());
    }
    let obj = load_header_from(reader).map_err(|e| e.to_string())?;
    if obj
        .meta()
        .media_storage_sop_class_uid()
        .trim_end_matches('\0')
        == DICOMDIR_SOP_CLASS
    {
        return Err("DICOMDIR index, not an image".into());
    }
    if let Some(kind) = document_kind(&obj) {
        return Ok(Classified::Document {
            kind,
            title: document_title(&obj),
            series_number: int(&obj, tags::SERIES_NUMBER),
            study_uid: string(&obj, tags::STUDY_INSTANCE_UID),
        });
    }
    let rows = number(&obj, tags::ROWS).unwrap_or(0);
    let cols = number(&obj, tags::COLUMNS).unwrap_or(0);
    if rows == 0 || cols == 0 {
        return Err("no image (Rows/Columns absent)".into());
    }
    let frames = obj
        .get(tags::NUMBER_OF_FRAMES)
        .and_then(|e| e.to_str().ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    let meta = Meta {
        study_uid: string(&obj, tags::STUDY_INSTANCE_UID),
        series_uid: string(&obj, tags::SERIES_INSTANCE_UID)
            .unwrap_or_else(|| "(no Series Instance UID)".into()),
        series_number: int(&obj, tags::SERIES_NUMBER),
        series_description: string(&obj, tags::SERIES_DESCRIPTION).unwrap_or_default(),
        modality: string(&obj, tags::MODALITY).unwrap_or_else(|| "?".into()),
        instance_number: int(&obj, tags::INSTANCE_NUMBER),
        position: position_key(&obj),
        sop_instance_uid: string(&obj, tags::SOP_INSTANCE_UID).unwrap_or_default(),
        rows,
        cols,
    };
    Ok(Classified::Image(meta, frames))
}

fn int(obj: &DefaultDicomObject, tag: Tag) -> Option<i32> {
    let e = obj.get(tag)?;
    e.to_int::<i32>()
        .ok()
        .or_else(|| e.to_str().ok()?.trim().parse().ok())
}

fn floats(obj: &DefaultDicomObject, tag: Tag) -> Option<Vec<f64>> {
    let e = obj.get(tag)?;
    let values = e.to_multi_str().ok()?;
    values
        .iter()
        .map(|s| s.trim().parse::<f64>().ok())
        .collect()
}

/// Image Position (Patient) projected onto the normal of Image Orientation
/// (Patient); the z coordinate when orientation is absent.
fn position_key(obj: &DefaultDicomObject) -> Option<f64> {
    let pos = floats(obj, tags::IMAGE_POSITION_PATIENT)?;
    if pos.len() != 3 {
        return None;
    }
    match floats(obj, tags::IMAGE_ORIENTATION_PATIENT) {
        Some(o) if o.len() == 6 => {
            let n = [
                o[1] * o[5] - o[2] * o[4],
                o[2] * o[3] - o[0] * o[5],
                o[0] * o[4] - o[1] * o[3],
            ];
            Some(pos[0] * n[0] + pos[1] * n[1] + pos[2] * n[2])
        }
        _ => Some(pos[2]),
    }
}

pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(ZIP_MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dicom::testutil::Synthetic;
    use std::io::Write;

    fn entry(name: &str, bytes: Vec<u8>) -> FileEntry {
        FileEntry::new(name, bytes)
    }

    fn slice_file(series: &str, series_number: i32, instance: i32, z: f64) -> Vec<u8> {
        Synthetic {
            series_uid: series.into(),
            series_number: Some(series_number),
            instance_number: Some(instance),
            position: Some([0.0, 0.0, z]),
            orientation: Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            ..Synthetic::default()
        }
        .build()
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, bytes) in entries {
                if name.ends_with('/') {
                    w.add_directory(*name, opts).unwrap();
                } else {
                    w.start_file(*name, opts).unwrap();
                    w.write_all(bytes).unwrap();
                }
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn groups_and_sorts_by_position() {
        let files = vec![
            entry("c.dcm", slice_file("1.2.3.A", 2, 3, 30.0)),
            entry("a.dcm", slice_file("1.2.3.A", 2, 1, 10.0)),
            entry("b.dcm", slice_file("1.2.3.A", 2, 2, 20.0)),
            entry("x.dcm", slice_file("1.2.3.B", 1, 1, 0.0)),
            entry("README.txt", b"not dicom at all, just text".to_vec()),
        ];
        let set = StudySet::scan(files);
        assert_eq!(set.series.len(), 2);
        assert_eq!(set.series[0].number, Some(1));
        assert_eq!(set.series[1].number, Some(2));
        let order: Vec<_> = set.series[1]
            .slices
            .iter()
            .map(|s| s.instance_number)
            .collect();
        assert_eq!(order, [Some(1), Some(2), Some(3)]);
        assert_eq!(set.slice_count(), 4);
        assert_eq!(set.skipped.len(), 1);
        assert!(
            set.skipped[0].reason.contains("Not a DICOM file"),
            "{}",
            set.skipped[0].reason
        );
        assert!(set.series[1].label().starts_with("2: CT"));
        assert_eq!(
            set.study_uid.as_deref(),
            Some("1.3.6.1.4.1.5962.1.2.4.20040826185059.5457")
        );
        assert_eq!(
            set.bytes(0).unwrap().len(),
            slice_file("1.2.3.A", 2, 3, 30.0).len()
        );
        assert_eq!(
            set.resident_bytes(),
            set.files
                .iter()
                .map(|f| match &f.source {
                    Source::Bytes(b) => b.len(),
                    Source::Archived { .. } => 0,
                })
                .sum::<usize>()
        );
    }

    #[test]
    fn position_beats_instance_number_and_reversed_normal_flips_order() {
        let files = vec![
            entry("1.dcm", slice_file("S", 1, 1, 50.0)),
            entry("2.dcm", slice_file("S", 1, 2, 40.0)),
        ];
        let set = StudySet::scan(files);
        let order: Vec<_> = set.series[0]
            .slices
            .iter()
            .map(|s| s.instance_number)
            .collect();
        assert_eq!(order, [Some(2), Some(1)]);
    }

    #[test]
    fn multi_frame_becomes_slices_and_dicomdir_is_skipped() {
        let mf = Synthetic {
            frames: 3,
            ..Synthetic::default()
        }
        .build();
        let files = vec![entry("mf.dcm", mf), entry("DICOMDIR", b"junk".to_vec())];
        let set = StudySet::scan(files);
        assert_eq!(set.series.len(), 1);
        let frames: Vec<_> = set.series[0].slices.iter().map(|s| s.frame).collect();
        assert_eq!(frames, [0, 1, 2]);
        assert_eq!(set.skipped[0].reason, "DICOMDIR index, not an image");
    }

    #[test]
    fn zip_is_scanned_lazily_and_entries_inflate_on_demand() {
        let a = slice_file("S", 1, 1, 0.0);
        let b = slice_file("S", 1, 2, 1.0);
        let zipped = zip_of(&[
            ("study/", b""),
            ("study/a.dcm", &a),
            ("study/b.dcm", &b),
            ("study/notes.txt", b"hello"),
            ("__MACOSX/study/._a.dcm", b"\x00\x05\x16\x07"),
            ("study/.DS_Store", b"\x00"),
        ]);
        assert!(is_zip(&zipped));
        let set = StudySet::scan(vec![entry("study.zip", zipped.clone())]);
        assert_eq!(set.series.len(), 1);
        assert_eq!(set.series[0].slices.len(), 2);
        assert_eq!(set.files[0].name, "study.zip/study/a.dcm");
        assert!(matches!(
            set.files[0].source,
            Source::Archived {
                archive: 0,
                index: 1
            }
        ));
        assert_eq!(set.skipped.len(), 1);
        assert_eq!(set.skipped[0].name, "study.zip/study/notes.txt");
        assert_eq!(set.ignored_metadata, 2);
        // Memory is the archive, not the archive plus its contents.
        assert_eq!(set.resident_bytes(), zipped.len());
        // On demand inflation gives back the original file.
        assert_eq!(set.bytes(0).unwrap().as_ref(), a.as_slice());
        assert_eq!(set.bytes(1).unwrap().as_ref(), b.as_slice());

        let bad = StudySet::scan(vec![entry("bad.zip", b"PK\x03\x04garbage".to_vec())]);
        assert!(bad.is_empty());
        assert!(bad.skipped[0].reason.contains("zip"));
    }

    #[test]
    fn structured_reports_become_documents() {
        let path = format!(
            "{}/../../samples/dicom/test-SR.dcm",
            env!("CARGO_MANIFEST_DIR")
        );
        let Ok(sr) = std::fs::read(&path) else {
            eprintln!("{path} not fetched; skipping");
            return;
        };
        let set = StudySet::scan(vec![
            entry("report.dcm", sr),
            entry("a.dcm", slice_file("S", 1, 1, 0.0)),
        ]);
        assert_eq!(set.series.len(), 1);
        assert_eq!(set.documents.len(), 1);
        assert_eq!(set.documents[0].kind, DocumentKind::StructuredReport);
        assert_eq!(set.documents[0].file, 0);
        assert!(set.documents[0].label().starts_with("SR: "));
        assert!(set.skipped.is_empty(), "{:?}", set.skipped);
        assert!(!set.is_empty());
        let only_docs = StudySet::scan(vec![entry("report.dcm", std::fs::read(&path).unwrap())]);
        assert!(
            !only_docs.is_empty(),
            "a study with only a report is still a study"
        );
    }

    #[test]
    fn metadata_names() {
        assert!(is_metadata_name("__MACOSX/x/._a.dcm"));
        assert!(is_metadata_name("study/._a.dcm"));
        assert!(is_metadata_name(".DS_Store"));
        assert!(is_metadata_name("a\\b\\Thumbs.db"));
        assert!(!is_metadata_name("study/a.dcm"));
        assert!(!is_metadata_name("_underscore.dcm"));
    }
}
