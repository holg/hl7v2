//! The desktop app's state and every operation on it, free of egui and
//! winit so it can be tested on the host like the core it drives.

use dicomscope_core::annotations::{self, Annotations, ImageAnnotations};
use dicomscope_core::dicom::pixels::FrameInfo;
use dicomscope_core::dicom::sr;
use dicomscope_core::dicom::{self, Frame, Study, StudySet, TagRow};
use dicomscope_core::fhir::{self, FhirInput};
use dicomscope_core::link::{self, Chain, Linkage, WorklistKeys};
use dicomscope_core::measure::{Measurement, Spacing};
use dicomscope_core::nerve::NerveTrace;
use dicomscope_core::nervefind;
use dicomscope_core::thumbnail::{thumbnail, Thumbnail};
use dicomscope_core::view::Viewport;
use dicomscope_core::worklist::{self, WorklistOutput};
use dicomscope_core::{fs, AppError};
use hl7kit::order::{Order, OrderField};
use hl7kit::{Message, Span};
use std::collections::HashMap;
use std::path::PathBuf;

/// Longest side of a series thumbnail, in pixels.
pub const THUMB_SIZE: u32 = 96;

/// A parsed HL7 order message.
pub struct LoadedHl7 {
    pub name: String,
    pub raw: String,
    pub order: Order,
    pub spans: Vec<(Span, OrderField)>,
    pub warnings: Vec<String>,
    pub summary: String,
    /// PID-3.4, for the linkage note.
    pub patient_authority: Option<String>,
}

/// Pretty-printed FHIR JSON per resource.
pub struct FhirText {
    pub patient: String,
    pub service_request: String,
    pub imaging_study: String,
    pub bundle: String,
}

/// A report or PDF instance, decoded for display.
pub enum DocumentContent {
    Report {
        title: String,
        text: String,
    },
    Pdf {
        title: String,
        bytes: Vec<u8>,
        mime: String,
    },
    Failed(String),
}

/// Cine playback rate when the file carries no Frame Time.
pub const DEFAULT_FRAME_MS: f32 = 100.0;

#[derive(Default)]
pub struct Session {
    pub set: Option<StudySet>,
    pub source: String,
    /// (series, slice) on screen.
    pub current: (usize, usize),
    pub frame: Option<FrameInfo>,
    pub study: Option<Study>,
    pub file_name: String,
    pub tags: Vec<TagRow>,
    pub thumbs: Vec<Option<Thumbnail>>,
    pub view: Viewport,
    pub window: (f32, f32),
    pub hl7: Option<LoadedHl7>,
    pub linkage: Option<Linkage>,
    pub worklist: Option<Result<WorklistOutput, String>>,
    pub chain: Option<Chain>,
    pub fhir: Option<FhirText>,
    pub error: Option<String>,
    /// The document on screen instead of the image, if one was selected.
    pub document: Option<(usize, DocumentContent)>,
    /// Measurements per (file, frame), kept while scrolling.
    marks: HashMap<(usize, u32), Vec<Measurement>>,
    /// Nerve traces per (file, frame).
    nerves: HashMap<(usize, u32), Vec<NerveTrace>>,
    /// Where measurements and traces are saved, next to the study.
    annotations_path: Option<PathBuf>,
    /// Local contrast enhancement in the shader.
    pub enhance: bool,
    pub enhance_amount: f32,
    pub enhance_radius_mm: f32,
    /// (file, frame) of the slice on screen.
    key: (usize, u32),
    pub playing: bool,
    /// Time of the last cine step, in the UI clock's seconds.
    last_tick: f64,
    /// Decoded but not yet uploaded to the GPU; the window loop takes it.
    pending: Option<Frame>,
    /// The greyscale pixels on screen, kept for image analysis (canal
    /// finding); `None` for colour images.
    gray: Option<std::sync::Arc<Vec<f32>>>,
    /// Size of the image area in device pixels, kept for refits.
    canvas: (u32, u32),
}

impl Session {
    pub fn new() -> Session {
        Session {
            enhance_amount: 1.0,
            enhance_radius_mm: 3.0,
            ..Session::default()
        }
    }

    /// Enhancement radius in source pixels: from the pixel spacing, else a
    /// guess that suits a panoramic image.
    pub fn enhance_radius_px(&self) -> f32 {
        match self.frame.as_ref().and_then(|f| f.spacing) {
            Some(s) => (self.enhance_radius_mm / s.col_mm).clamp(2.0, 200.0),
            None => 30.0,
        }
    }

    pub fn spacing(&self) -> Option<Spacing> {
        self.frame.as_ref().and_then(|f| f.spacing)
    }

    /// The window loop tells the session how big the image area is.
    pub fn set_canvas(&mut self, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if size != self.canvas {
            let refit = self.canvas == (0, 0);
            self.canvas = size;
            if refit {
                self.fit();
            }
        }
    }

    pub fn canvas(&self) -> (u32, u32) {
        if self.canvas == (0, 0) {
            (1, 1)
        } else {
            self.canvas
        }
    }

    pub fn image(&self) -> Option<(u32, u32)> {
        self.frame.as_ref().map(|f| (f.width, f.height))
    }

    /// The frame decoded by the last `show_slice`, once.
    pub fn take_pending_frame(&mut self) -> Option<Frame> {
        self.pending.take()
    }

    /// Open a folder, zip or file. Replaces the study; keeps the order.
    pub fn open_study(&mut self, path: &str) {
        self.open_study_paths(&[path.to_string()]);
    }

    /// Open several folders, zips or files as one study, as dropped onto
    /// the window.
    pub fn open_study_paths(&mut self, paths: &[String]) {
        let path = paths.join(", ");
        let inputs = paths.iter().try_fold(Vec::new(), |mut acc, p| {
            acc.extend(fs::collect_inputs(p)?);
            Ok::<_, String>(acc)
        });
        self.marks.clear();
        self.nerves.clear();
        self.playing = false;
        self.document = None;
        self.annotations_path = paths.first().map(|p| annotations_path_for(p));
        match inputs.map(StudySet::scan) {
            Ok(set) if !set.is_empty() => {
                self.thumbs = (0..set.series.len()).map(|_| None).collect();
                self.set = Some(set);
                self.source = path.clone();
                self.error = None;
                self.make_thumbnails();
                self.load_annotations();
                self.show_slice(0, 0, true);
            }
            Ok(set) => {
                let why = set
                    .skipped
                    .first()
                    .map(|s| format!(" ({}: {})", s.name, s.reason))
                    .unwrap_or_default();
                self.error = Some(format!("{path}: no displayable DICOM image found{why}"));
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// One thumbnail per series from its first slice.
    fn make_thumbnails(&mut self) {
        let Some(set) = &self.set else { return };
        let mut thumbs = Vec::with_capacity(set.series.len());
        for series in &set.series {
            let thumb = series.slices.first().and_then(|s| {
                let bytes = set.bytes(s.file).ok()?;
                let obj = dicom::load(&bytes).ok()?;
                let frame = dicom::decode_frame(&obj, s.frame).ok()?;
                Some(thumbnail(&frame, THUMB_SIZE))
            });
            thumbs.push(thumb);
        }
        self.thumbs = thumbs;
    }

    /// Decode one slice. `reset` refits the view and reloads the window
    /// from the file; scrolling within a series keeps both.
    pub fn show_slice(&mut self, series: usize, slice: usize, reset: bool) {
        let Some(set) = &self.set else { return };
        let Some((_, s)) = set.slice(series, slice) else {
            return;
        };
        let (file, frame_index) = (s.file, s.frame);
        let name = set.files[file].name.clone();
        self.key = (file, frame_index);
        self.document = None;
        let loaded = set
            .bytes(file)
            .map_err(AppError::FileRead)
            .and_then(|bytes| dicom::load(&bytes))
            .map(|obj| {
                let study = Study::from_object(&obj);
                let tags = dicom::tag_rows(&obj);
                let frame = dicom::decode_frame(&obj, frame_index);
                (study, tags, frame)
            });
        match loaded {
            Ok((study, tags, Ok(frame))) => {
                let info = FrameInfo::from(&frame);
                let size_changed = self.image() != Some((info.width, info.height));
                self.frame = Some(info);
                if reset || size_changed {
                    self.fit();
                }
                if reset {
                    self.reset_window();
                }
                self.study = Some(study);
                self.tags = tags;
                self.gray = match &frame.pixels {
                    dicomscope_core::dicom::Pixels::Gray(g) => Some(std::sync::Arc::new(g.clone())),
                    dicomscope_core::dicom::Pixels::Rgba(_) => None,
                };
                self.pending = Some(frame);
                self.error = None;
            }
            Ok((study, tags, Err(e))) => {
                self.study = Some(study);
                self.tags = tags;
                self.frame = None;
                self.error = Some(format!("{name}: {e}"));
            }
            Err(e) => self.error = Some(format!("{name}: {e}")),
        }
        self.file_name = name;
        self.current = (series, slice);
        self.derive();
    }

    pub fn fit(&mut self) {
        if let Some(img) = self.image() {
            self.view = Viewport::default().fit(self.canvas(), img);
        }
    }

    pub fn one_to_one(&mut self) {
        if let Some(img) = self.image() {
            self.view = self.view.one_to_one(self.canvas(), img);
        }
    }

    pub fn rotate(&mut self, quarter_turns: i8) {
        if let Some(img) = self.image() {
            self.view = self.view.rotate(quarter_turns, img);
        }
    }

    pub fn reset_window(&mut self) {
        if let Some(f) = &self.frame {
            self.window = f
                .default_window
                .unwrap_or_else(|| dicom::fallback_window(f.value_range));
        }
    }

    pub fn scroll_slices(&mut self, delta: i32) {
        let (si, sl) = self.current;
        let Some(n) = self
            .set
            .as_ref()
            .and_then(|s| s.series.get(si))
            .map(|s| s.slices.len())
        else {
            return;
        };
        let next = (sl as i32 + delta).clamp(0, n as i32 - 1) as usize;
        if next != sl {
            self.show_slice(si, next, false);
        }
    }

    pub fn switch_series(&mut self, delta: i32) {
        let Some(n) = self.set.as_ref().map(|s| s.series.len()) else {
            return;
        };
        let next = (self.current.0 as i32 + delta).clamp(0, n as i32 - 1) as usize;
        if next != self.current.0 {
            self.show_slice(next, 0, true);
        }
    }

    /// Parse an HL7 file. Latin-1 messages are common; keep going and say so.
    pub fn open_hl7(&mut self, path: &str) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => return self.error = Some(format!("{path}: {e}")),
        };
        let msg = match Message::parse_bytes(&bytes) {
            Ok(m) => m,
            Err(hl7kit::ParseError::InvalidUtf8 { .. }) => match Message::parse_lossy(&bytes) {
                Ok(m) => m,
                Err(e) => return self.error = Some(format!("{path}: {e}")),
            },
            Err(e) => return self.error = Some(format!("{path}: {e}")),
        };
        let order = Order::extract(&msg);
        let mut warnings: Vec<String> = msg.warnings().iter().map(|w| w.to_string()).collect();
        if std::str::from_utf8(&bytes).is_err() {
            warnings.push("input was not valid UTF-8; undecodable bytes replaced".to_string());
        }
        let summary = match msg.message_type() {
            Some(t) => format!(
                "{}^{} from {} ({} segments, HL7 v{}, control ID {})",
                t.code,
                t.trigger,
                msg.get("MSH-3").unwrap_or("?"),
                msg.segment_count(),
                msg.version().unwrap_or("?"),
                msg.control_id().unwrap_or("?")
            ),
            None => format!("{} segments, no MSH-9", msg.segment_count()),
        };
        let patient_authority = msg
            .get("PID-3.4")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self.hl7 = Some(LoadedHl7 {
            name: path.to_string(),
            raw: msg.raw().to_string(),
            spans: order.spans.clone(),
            order,
            warnings,
            summary,
            patient_authority,
        });
        self.error = None;
        self.derive();
    }

    /// Everything that depends on the study, the order, or both.
    fn derive(&mut self) {
        self.linkage = None;
        self.worklist = None;
        self.chain = None;
        self.fhir = None;
        let Some(h) = &self.hl7 else { return };
        let Ok(msg) = Message::parse(&h.raw) else {
            return;
        };
        let worklist = worklist::build(&msg, &h.order).map_err(|e| e.to_string());
        let keys = worklist
            .as_ref()
            .ok()
            .map(|o| WorklistKeys::from_item(&o.item));
        if let Some(study) = &self.study {
            let linkage = link::resolve(study, &h.order);
            self.chain = Some(link::resolve_chain(&h.order, keys.as_ref(), study));
            let series: &[dicom::Series] = self
                .set
                .as_ref()
                .map(|s| s.series.as_slice())
                .unwrap_or(&[]);
            let input = FhirInput {
                study,
                series,
                message: &msg,
                order: &h.order,
                linkage: &linkage,
            };
            let r = fhir::resources(&input);
            let bundle = fhir::bundle(&input);
            let pretty =
                |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_default();
            self.fhir = Some(FhirText {
                patient: pretty(&r.patient),
                service_request: pretty(&r.service_request),
                imaging_study: pretty(&r.imaging_study),
                bundle: pretty(&bundle),
            });
            self.linkage = Some(linkage);
        }
        self.worklist = Some(worklist);
    }

    /// Files dropped onto the window: `.hl7` and `.txt` are orders, the
    /// rest is one study.
    pub fn open_paths(&mut self, paths: &[String]) {
        let is_order = |p: &String| {
            let lower = p.to_ascii_lowercase();
            lower.ends_with(".hl7") || lower.ends_with(".txt")
        };
        let (orders, studies): (Vec<String>, Vec<String>) =
            paths.iter().cloned().partition(is_order);
        if !studies.is_empty() {
            self.open_study_paths(&studies);
        }
        if let Some(order) = orders.last() {
            self.open_hl7(order);
        }
    }

    // --- measurements ---

    pub fn measurements(&self) -> &[Measurement] {
        self.marks.get(&self.key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn push_measurement(&mut self, m: Measurement) {
        self.marks.entry(self.key).or_default().push(m);
        self.save_annotations();
    }

    pub fn remove_last_measurement(&mut self) {
        if let Some(v) = self.marks.get_mut(&self.key) {
            v.pop();
        }
        self.save_annotations();
    }

    pub fn clear_measurements(&mut self) {
        self.marks.remove(&self.key);
        self.nerves.remove(&self.key);
        self.save_annotations();
    }

    // --- nerve traces ---

    pub fn nerves(&self) -> &[NerveTrace] {
        self.nerves.get(&self.key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn push_nerve(&mut self, t: NerveTrace) {
        self.nerves.entry(self.key).or_default().push(t);
        self.save_annotations();
    }

    pub fn remove_last_nerve(&mut self) {
        if let Some(v) = self.nerves.get_mut(&self.key) {
            v.pop();
        }
        self.save_annotations();
    }

    /// Move one control point of one trace; saved when the drag ends.
    pub fn move_nerve_point(&mut self, trace: usize, point: usize, to: (f32, f32)) {
        if let Some(p) = self
            .nerves
            .get_mut(&self.key)
            .and_then(|v| v.get_mut(trace))
            .and_then(|t| t.points.get_mut(point))
        {
            *p = to;
        }
    }

    pub fn set_nerve_diameter(&mut self, trace: usize, diameter_mm: f32) {
        if let Some(t) = self
            .nerves
            .get_mut(&self.key)
            .and_then(|v| v.get_mut(trace))
        {
            t.diameter_mm = diameter_mm.clamp(0.5, 10.0);
        }
        self.save_annotations();
    }

    /// Find the canal between two points on the current image and add it
    /// as a trace. The expected canal width comes from the pixel spacing
    /// (3.5 mm), else a guess for a panoramic image.
    pub fn detect_nerve(&mut self, a: (f32, f32), b: (f32, f32)) -> Result<f32, String> {
        let (Some(gray), Some(f)) = (&self.gray, &self.frame) else {
            return Err("no greyscale image on screen".into());
        };
        let canal_px = match f.spacing {
            Some(s) => 3.5 / s.col_mm,
            None => (f.width as f32 / 80.0).max(6.0),
        };
        let found =
            nervefind::find_canal(gray, f.width as usize, f.height as usize, a, b, canal_px)?;
        let confidence = found.confidence;
        self.push_nerve(found.trace);
        Ok(confidence)
    }

    // --- persistence ---

    /// SOP Instance UID for a (file, frame) key, from the scanned series.
    fn sop_uid(&self, key: (usize, u32)) -> Option<String> {
        let set = self.set.as_ref()?;
        set.series
            .iter()
            .flat_map(|s| s.slices.iter())
            .find(|s| (s.file, s.frame) == key)
            .map(|s| s.sop_instance_uid.clone())
    }

    fn annotations(&self) -> Annotations {
        let mut a = Annotations::default();
        let keys: std::collections::BTreeSet<(usize, u32)> = self
            .marks
            .keys()
            .chain(self.nerves.keys())
            .copied()
            .collect();
        for k in keys {
            let Some(uid) = self.sop_uid(k) else { continue };
            let entry = a.images.entry(annotations::key(&uid, k.1)).or_default();
            entry.measurements = self.marks.get(&k).cloned().unwrap_or_default();
            entry.nerves = self.nerves.get(&k).cloned().unwrap_or_default();
        }
        a
    }

    /// Write everything drawn to the JSON next to the study. Silent when
    /// the study has no path or nothing is drawn yet and no file exists.
    pub fn save_annotations(&self) {
        let Some(path) = &self.annotations_path else {
            return;
        };
        let a = self.annotations();
        if a.images.values().all(ImageAnnotations::is_empty) && !path.exists() {
            return;
        }
        if let Err(e) = std::fs::write(path, a.to_json()) {
            eprintln!("annotations {}: {e}", path.display());
        }
    }

    fn load_annotations(&mut self) {
        let Some(path) = &self.annotations_path else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let a = match Annotations::from_json(&text) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("annotations {}: {e}", path.display());
                return;
            }
        };
        let Some(set) = &self.set else { return };
        for slice in set.series.iter().flat_map(|s| s.slices.iter()) {
            let k = annotations::key(&slice.sop_instance_uid, slice.frame);
            if let Some(img) = a.images.get(&k) {
                let key = (slice.file, slice.frame);
                if !img.measurements.is_empty() {
                    self.marks.insert(key, img.measurements.clone());
                }
                if !img.nerves.is_empty() {
                    self.nerves.insert(key, img.nerves.clone());
                }
            }
        }
    }

    // --- cine ---

    /// Milliseconds per frame: the file's Frame Time or Cine Rate, else a
    /// default.
    pub fn frame_ms(&self) -> f32 {
        self.frame
            .as_ref()
            .and_then(|f| f.frame_time_ms)
            .filter(|ms| *ms > 0.0)
            .unwrap_or(DEFAULT_FRAME_MS)
    }

    pub fn toggle_cine(&mut self, now: f64) {
        let has_slices = self
            .set
            .as_ref()
            .and_then(|s| s.series.get(self.current.0))
            .is_some_and(|s| s.slices.len() > 1);
        self.playing = !self.playing && has_slices;
        self.last_tick = now;
    }

    /// Advance when a frame period has passed; wraps at the end of the
    /// series. `now` is the UI clock in seconds.
    pub fn cine_tick(&mut self, now: f64) {
        if !self.playing {
            return;
        }
        let period = f64::from(self.frame_ms()) / 1000.0;
        if now - self.last_tick < period {
            return;
        }
        self.last_tick = now;
        let (si, sl) = self.current;
        let Some(n) = self
            .set
            .as_ref()
            .and_then(|s| s.series.get(si))
            .map(|s| s.slices.len())
        else {
            return;
        };
        self.show_slice(si, (sl + 1) % n.max(1), false);
    }

    // --- documents ---

    /// Decode a report or PDF from the study's document list and show it
    /// instead of the image.
    pub fn open_document(&mut self, index: usize) {
        let Some(set) = &self.set else { return };
        let Some(doc) = set.documents.get(index) else {
            return;
        };
        let title = doc.title.clone();
        let content = match set
            .bytes(doc.file)
            .map_err(AppError::FileRead)
            .and_then(|b| dicom::load(&b))
        {
            Err(e) => DocumentContent::Failed(format!("{}: {e}", set.files[doc.file].name)),
            Ok(obj) => match sr::document_kind(&obj) {
                Some(sr::DocumentKind::EncapsulatedPdf) => match sr::encapsulated_document(&obj) {
                    Some((bytes, mime)) => DocumentContent::Pdf { title, bytes, mime },
                    None => DocumentContent::Failed(
                        "Encapsulated PDF without an Encapsulated Document element".into(),
                    ),
                },
                _ => DocumentContent::Report {
                    title,
                    text: sr::sr_to_text(&sr::render_sr(&obj)),
                },
            },
        };
        self.playing = false;
        self.document = Some((index, content));
    }

    /// Back from a document to the image.
    pub fn close_document(&mut self) {
        self.document = None;
    }
}

/// Where a study's annotations live: inside a folder, or next to a zip or
/// file with `.annotations.json` appended.
fn annotations_path_for(study_path: &str) -> PathBuf {
    let p = PathBuf::from(study_path);
    if p.is_dir() {
        p.join("dicomscope-annotations.json")
    } else {
        let mut s = p.into_os_string();
        s.push(".annotations.json");
        PathBuf::from(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dicomscope_core::dicom::testutil::Synthetic;

    const SAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples");

    /// A one-file study on disk, built in memory: the DICOM samples are not
    /// committed, the HL7 orders are.
    fn synthetic_study(name: &str) -> String {
        let dir =
            std::env::temp_dir().join(format!("dicomscope-desktop-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("1.dcm");
        std::fs::write(&path, Synthetic::default().build()).unwrap();
        path.display().to_string()
    }

    #[test]
    fn study_and_order_derive_linkage_worklist_and_fhir() {
        let mut s = Session::new();
        s.set_canvas((800, 600));
        s.open_study(&synthetic_study("session"));
        assert!(s.error.is_none(), "{:?}", s.error);
        assert!(s.take_pending_frame().is_some());
        assert_eq!(s.image(), Some((3, 2)));
        assert!(!s.tags.is_empty());
        assert_eq!(s.thumbs.len(), 1);
        s.open_hl7(&format!("{SAMPLES}/order.hl7"));
        assert!(s.error.is_none(), "{:?}", s.error);
        assert!(s.linkage.is_some());
        assert!(matches!(s.worklist, Some(Ok(_))));
        assert!(s.chain.is_some());
        assert!(s
            .fhir
            .as_ref()
            .is_some_and(|f| f.bundle.contains("\"Bundle\"")));
    }

    #[test]
    fn cine_steps_on_the_clock_and_measurements_stay_with_their_slice() {
        let dir =
            std::env::temp_dir().join(format!("dicomscope-desktop-cine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mf.dcm");
        std::fs::write(
            &path,
            Synthetic {
                frames: 3,
                ..Synthetic::default()
            }
            .build(),
        )
        .unwrap();
        let mut s = Session::new();
        s.set_canvas((100, 100));
        s.open_study(&path.display().to_string());
        assert!(s.error.is_none(), "{:?}", s.error);
        assert_eq!(s.set.as_ref().unwrap().series[0].slices.len(), 3);

        s.push_measurement(Measurement::Length {
            a: (0.0, 0.0),
            b: (3.0, 4.0),
        });
        assert_eq!(s.measurements().len(), 1);
        assert_eq!(s.measurements()[0].label(None), "5 px");

        s.toggle_cine(0.0);
        assert!(s.playing);
        s.cine_tick(0.05);
        assert_eq!(s.current, (0, 0), "no step before a frame period");
        s.cine_tick(0.2);
        assert_eq!(s.current, (0, 1));
        assert!(
            s.measurements().is_empty(),
            "marks belong to the slice they were drawn on"
        );
        s.cine_tick(0.4);
        s.cine_tick(0.6);
        assert_eq!(s.current, (0, 0), "wraps at the end of the series");
        assert_eq!(s.measurements().len(), 1);
        s.remove_last_measurement();
        assert!(s.measurements().is_empty());
        s.toggle_cine(0.7);
        assert!(!s.playing);

        // Dropping an order file next to the study loads both.
        s.open_paths(&[format!("{SAMPLES}/order.hl7")]);
        assert!(s.hl7.is_some() && s.linkage.is_some());
    }

    #[test]
    fn annotations_are_saved_next_to_the_study_and_reloaded() {
        let path = synthetic_study("annot");
        let mut s = Session::new();
        s.set_canvas((100, 100));
        s.open_study(&path);
        assert!(s.nerves().is_empty());
        s.push_nerve(NerveTrace::new(vec![(0.0, 0.0), (2.0, 1.0)]));
        s.push_measurement(Measurement::Length {
            a: (0.0, 0.0),
            b: (3.0, 4.0),
        });
        let json = format!("{path}.annotations.json");
        assert!(
            std::path::Path::new(&json).exists(),
            "written next to the file"
        );
        let mut again = Session::new();
        again.set_canvas((100, 100));
        again.open_study(&path);
        assert_eq!(again.nerves().len(), 1);
        assert_eq!(again.measurements().len(), 1);
    }

    #[test]
    fn order_alone_builds_the_worklist_item() {
        let mut s = Session::new();
        s.open_hl7(&format!("{SAMPLES}/order-omi.hl7"));
        assert!(s.linkage.is_none());
        assert!(matches!(s.worklist, Some(Ok(_))));
        assert!(s.fhir.is_none());
    }
}
