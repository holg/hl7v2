//! The egui panels. Pure presentation over [`Session`]; file dialogs and
//! GPU work are requested through [`Actions`] and done by the window loop.

use crate::session::{DocumentContent, FhirText, Session};
use dicomscope_core::dicom::TagRow;
use dicomscope_core::link::{ChainFinding, KeyConflict, LinkPath, Pair};
use dicomscope_core::measure::{Measurement, Point};
use dicomscope_core::view::Viewport;
use egui::{Color32, Key, Pos2, RichText, TextureHandle, Ui};
use hl7kit::order::OrderField;
use mwlkit::StudyUidOrigin;
use std::time::Duration;

/// What the panels asked the window loop to do this frame.
#[derive(Default)]
pub struct Actions {
    pub open_study_folder: bool,
    pub open_study_file: bool,
    pub open_hl7: bool,
    pub save_worklist: bool,
    pub save_fhir: bool,
    pub open_pdf: bool,
    pub save_pdf: bool,
    /// Re-scan the platform's document folder (iOS).
    pub reload: bool,
    /// The image area in physical pixels, if a study is shown.
    pub image_rect: Option<(u32, u32, u32, u32)>,
}

/// What the left mouse button does in the image area.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pan,
    Length,
    Angle,
}

/// UI-only state that outlives a frame.
#[derive(Default)]
pub struct UiState {
    pub tag_filter: String,
    pub worklist_filter: String,
    pub fhir_tab: FhirTab,
    pub thumb_textures: Vec<Option<TextureHandle>>,
    pub scroll_accum: f32,
    pub status: Option<String>,
    pub tool: Tool,
    /// Points of the measurement being drawn, in source pixels.
    pub draft: Vec<Point>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum FhirTab {
    #[default]
    Bundle,
    Patient,
    ServiceRequest,
    ImagingStudy,
}

impl FhirTab {
    const ALL: [FhirTab; 4] = [
        FhirTab::Bundle,
        FhirTab::Patient,
        FhirTab::ServiceRequest,
        FhirTab::ImagingStudy,
    ];
    fn label(self) -> &'static str {
        match self {
            FhirTab::Bundle => "Bundle",
            FhirTab::Patient => "Patient",
            FhirTab::ServiceRequest => "ServiceRequest",
            FhirTab::ImagingStudy => "ImagingStudy",
        }
    }
    fn text(self, t: &FhirText) -> &str {
        match self {
            FhirTab::Bundle => &t.bundle,
            FhirTab::Patient => &t.patient,
            FhirTab::ServiceRequest => &t.service_request,
            FhirTab::ImagingStudy => &t.imaging_study,
        }
    }
}

const OK: Color32 = Color32::from_rgb(0x1b, 0x6e, 0x3a);
const DANGER: Color32 = Color32::from_rgb(0xb3, 0x26, 0x1e);
const WARN: Color32 = Color32::from_rgb(0x9a, 0x6a, 0x00);

pub fn draw(root: &mut Ui, session: &mut Session, state: &mut UiState) -> Actions {
    let mut actions = Actions::default();

    // Files dropped onto the window, from winit through egui.
    let dropped: Vec<String> = root.ctx().input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().display().to_string())
            .filter(|p| !p.is_empty())
            .collect()
    });
    if !dropped.is_empty() {
        session.open_paths(&dropped);
        state.thumb_textures.clear();
    }

    egui::Panel::top("top").show(root, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.heading("dicomscope");
            if ui.button("Open study folder").clicked() {
                actions.open_study_folder = true;
            }
            if ui.button("Open zip or file").clicked() {
                actions.open_study_file = true;
            }
            if ui.button("Open HL7 order").clicked() {
                actions.open_hl7 = true;
            }
            if cfg!(target_os = "ios") && ui.button("Reload").clicked() {
                actions.reload = true;
            }
            ui.separator();
            for (t, label) in [
                (Tool::Pan, "Pan"),
                (Tool::Length, "Length"),
                (Tool::Angle, "Angle"),
            ] {
                if ui.selectable_label(state.tool == t, label).clicked() {
                    state.tool = t;
                    state.draft.clear();
                }
            }
            if ui.button("Undo").clicked() {
                session.remove_last_measurement();
            }
            if ui.button("Clear").clicked() {
                session.clear_measurements();
            }
            ui.separator();
            let now = ui.input(|i| i.time);
            if ui
                .button(if session.playing { "Pause" } else { "Play" })
                .on_hover_text("Space. Cine through the series at the file's frame rate.")
                .clicked()
            {
                session.toggle_cine(now);
            }
            if !session.source.is_empty() {
                ui.label(RichText::new(&session.source).weak());
            }
            if let Some(h) = &session.hl7 {
                ui.label(RichText::new(&h.name).weak());
            }
        });
        if let Some(e) = &session.error {
            banner(ui, DANGER, e);
        }
        if let Some(s) = &state.status {
            ui.label(RichText::new(s).weak());
        }
    });

    egui::Panel::right("panels")
        .resizable(true)
        .default_size(460.0)
        .min_size(280.0)
        .max_size(640.0)
        .show(root, |ui| {
            // Long UIDs must wrap or scroll inside the panel, never widen it.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::CollapsingHeader::new("Series")
                    .default_open(true)
                    .show(ui, |ui| series_panel(ui, session, state));
                egui::CollapsingHeader::new("Window")
                    .default_open(true)
                    .show(ui, |ui| window_panel(ui, session));
                egui::CollapsingHeader::new("Tags").show(ui, |ui| {
                    if session.tags.is_empty() {
                        ui.weak("No DICOM file loaded.");
                    } else {
                        tag_table(ui, "tags", &mut state.tag_filter, &session.tags, 320.0);
                    }
                });
                egui::CollapsingHeader::new("HL7 order")
                    .default_open(true)
                    .show(ui, |ui| hl7_panel(ui, session));
                egui::CollapsingHeader::new("Linkage")
                    .default_open(true)
                    .show(ui, |ui| link_panel(ui, session));
                egui::CollapsingHeader::new("Modality Worklist")
                    .default_open(true)
                    .show(ui, |ui| worklist_panel(ui, session, state, &mut actions));
                egui::CollapsingHeader::new("FHIR R4")
                    .show(ui, |ui| fhir_panel(ui, session, state, &mut actions));
            });
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(root, |ui| image_area(ui, session, state, &mut actions));

    actions
}

fn banner(ui: &mut Ui, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .stroke(egui::Stroke::new(1.0, color))
        .inner_margin(6.0)
        .corner_radius(4.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(text);
        });
}

// ---------------------------------------------------------------------------
// Image area: rendering happens in the window loop; here is only the input.
// ---------------------------------------------------------------------------

fn image_area(ui: &mut Ui, session: &mut Session, state: &mut UiState, actions: &mut Actions) {
    let rect = ui.available_rect_before_wrap();
    let ppp = ui.ctx().pixels_per_point();
    let px = |v: f32| (v * ppp).round().max(0.0) as u32;
    let region = (
        px(rect.min.x),
        px(rect.min.y),
        px(rect.width()),
        px(rect.height()),
    );
    session.set_canvas((region.2, region.3));

    if session.set.is_none() {
        ui.put(
            rect,
            egui::Label::new(
                RichText::new("Open a study folder, a zip or a DICOM file, or drop them here.\nThen open its HL7 order to see the linkage, the worklist item and the FHIR output.")
                    .color(Color32::GRAY),
            ),
        );
        drop_hint(ui, rect);
        return;
    }
    if session.document.is_some() {
        document_view(ui, session, actions);
        return;
    }
    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    actions.image_rect = Some(region);

    // Source pixel <-> screen point, through the view geometry in device pixels.
    let image = session.image().unwrap_or((1, 1));
    let view = session.view;
    let to_source = |p: Pos2| -> Point {
        let c = ((p.x - rect.min.x) * ppp, (p.y - rect.min.y) * ppp);
        view.canvas_to_source(c, image)
    };
    let to_screen = |s: Point| -> Pos2 {
        let c = view.source_to_canvas(s, image);
        Pos2::new(rect.min.x + c.0 / ppp, rect.min.y + c.1 / ppp)
    };
    let pointer = ui
        .input(|i| i.pointer.hover_pos())
        .or_else(|| response.interact_pointer_pos());

    // Left button: pan, or draw a measurement.
    match state.tool {
        Tool::Pan => {
            if response.dragged_by(egui::PointerButton::Primary) {
                let d = response.drag_delta() * ppp;
                session.view = session.view.pan(d.x, d.y);
            }
        }
        Tool::Length => {
            if let Some(p) = pointer.map(to_source) {
                if response.drag_started_by(egui::PointerButton::Primary) {
                    state.draft = vec![p, p];
                } else if response.dragged_by(egui::PointerButton::Primary)
                    && state.draft.len() == 2
                {
                    state.draft[1] = p;
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) && state.draft.len() == 2
                {
                    let a = state.draft[0];
                    if dicomscope_core::measure::length_px(a, p) >= 1.0 {
                        session.push_measurement(Measurement::Length { a, b: p });
                    }
                    state.draft.clear();
                }
            }
        }
        Tool::Angle => {
            if let Some(p) = pointer.map(to_source) {
                if response.clicked_by(egui::PointerButton::Primary) {
                    // The last draft point follows the pointer; a click fixes it.
                    if let Some(last) = state.draft.last_mut() {
                        *last = p;
                    }
                    if state.draft.len() >= 3 {
                        session.push_measurement(Measurement::Angle {
                            a: state.draft[0],
                            vertex: state.draft[1],
                            c: state.draft[2],
                        });
                        state.draft.clear();
                    } else {
                        state.draft.push(p);
                    }
                } else if let Some(last) = state.draft.last_mut() {
                    *last = p;
                }
            }
        }
    }
    // Right button windows: horizontal = width, vertical = centre.
    if response.dragged_by(egui::PointerButton::Secondary) {
        if let Some(f) = &session.frame {
            let span = (f.value_range.1 - f.value_range.0).max(1.0);
            let d = response.drag_delta();
            let (c, w) = session.window;
            let dc = d.y / rect.height() * span;
            let dw = d.x / rect.width() * span;
            session.window = (
                (c + dc).clamp(f.value_range.0, f.value_range.1),
                (w + dw).max(1.0),
            );
        }
    }
    // Wheel: slices. Zoom is a trackpad pinch or Option/Alt+wheel, never
    // Ctrl+wheel: on macOS that is the system accessibility zoom and does
    // not reach the application.
    if response.hovered() {
        let (scroll, zoom, modifiers) =
            ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
        let p = pointer.unwrap_or(rect.center()) - rect.min;
        if zoom != 1.0 {
            session.view = session.view.zoom_about(zoom, p.x * ppp, p.y * ppp);
        } else if scroll.y != 0.0 {
            if modifiers.alt {
                let factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                session.view = session.view.zoom_about(factor, p.x * ppp, p.y * ppp);
            } else {
                state.scroll_accum += scroll.y;
                while state.scroll_accum >= 30.0 {
                    session.scroll_slices(-1);
                    state.scroll_accum -= 30.0;
                }
                while state.scroll_accum <= -30.0 {
                    session.scroll_slices(1);
                    state.scroll_accum += 30.0;
                }
            }
        }
    }
    // Keys, unless a text field has focus.
    if !ui.ctx().egui_wants_keyboard_input() {
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
        let shift = ui.input(|i| i.modifiers.shift);
        let now = ui.input(|i| i.time);
        if pressed(Key::ArrowUp) {
            session.scroll_slices(-1);
        }
        if pressed(Key::ArrowDown) {
            session.scroll_slices(1);
        }
        if pressed(Key::PageUp) {
            session.switch_series(-1);
        }
        if pressed(Key::PageDown) {
            session.switch_series(1);
        }
        if pressed(Key::Home) {
            session.show_slice(session.current.0, 0, false);
        }
        if pressed(Key::End) {
            if let Some(n) = session
                .set
                .as_ref()
                .and_then(|s| s.series.get(session.current.0))
                .map(|s| s.slices.len())
            {
                session.show_slice(session.current.0, n.saturating_sub(1), false);
            }
        }
        if pressed(Key::Space) {
            session.toggle_cine(now);
        }
        if pressed(Key::Escape) {
            state.draft.clear();
            state.tool = Tool::Pan;
        }
        if pressed(Key::Delete) || pressed(Key::Backspace) {
            if state.draft.is_empty() {
                session.remove_last_measurement();
            } else {
                state.draft.clear();
            }
        }
        if pressed(Key::Num0) {
            session.fit();
        }
        if pressed(Key::Num1) {
            session.one_to_one();
        }
        if pressed(Key::Plus) || pressed(Key::Equals) {
            let c = rect.center() - rect.min;
            session.view = session.view.zoom_about(1.25, c.x * ppp, c.y * ppp);
        }
        if pressed(Key::Minus) {
            let c = rect.center() - rect.min;
            session.view = session.view.zoom_about(0.8, c.x * ppp, c.y * ppp);
        }
        if pressed(Key::R) {
            session.rotate(if shift { -1 } else { 1 });
        }
        if pressed(Key::H) {
            session.view = session.view.flip_horizontal();
        }
        if pressed(Key::V) {
            session.view = session.view.flip_vertical();
        }
        if pressed(Key::I) {
            session.view.smooth = !session.view.smooth;
        }
        if pressed(Key::W) {
            session.reset_window();
        }
    }
    // Cine: step on the UI clock and keep frames coming.
    if session.playing {
        let now = ui.input(|i| i.time);
        session.cine_tick(now);
        let half = (session.frame_ms() / 2.0).max(8.0) as u64;
        ui.ctx().request_repaint_after(Duration::from_millis(half));
    }

    // Measurements and the draft, drawn in screen space over the image.
    let spacing = session.frame.as_ref().and_then(|f| f.spacing);
    let painter = ui.painter_at(rect);
    let stroke = egui::Stroke::new(1.5, Color32::from_rgb(0xff, 0xd7, 0x4d));
    let label = |painter: &egui::Painter, at: Pos2, text: String| {
        painter.text(
            at + egui::vec2(6.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            text,
            egui::FontId::proportional(13.0),
            Color32::from_rgb(0xff, 0xd7, 0x4d),
        );
    };
    for m in session.measurements() {
        let pts: Vec<Pos2> = m.points().iter().map(|&p| to_screen(p)).collect();
        painter.add(egui::Shape::line(pts.clone(), stroke));
        for p in &pts {
            painter.circle_filled(*p, 3.0, stroke.color);
        }
        let at = match m {
            Measurement::Length { .. } => {
                Pos2::new((pts[0].x + pts[1].x) / 2.0, (pts[0].y + pts[1].y) / 2.0)
            }
            Measurement::Angle { .. } => pts[1],
        };
        label(&painter, at, m.label(spacing));
    }
    if state.draft.len() >= 2 {
        let pts: Vec<Pos2> = state.draft.iter().map(|&p| to_screen(p)).collect();
        painter.add(egui::Shape::line(
            pts.clone(),
            egui::Stroke::new(1.0, Color32::from_rgb(0xff, 0xf0, 0xa0)),
        ));
        let preview = match state.tool {
            Tool::Length => Some(Measurement::Length {
                a: state.draft[0],
                b: state.draft[1],
            }),
            Tool::Angle if state.draft.len() == 3 => Some(Measurement::Angle {
                a: state.draft[0],
                vertex: state.draft[1],
                c: state.draft[2],
            }),
            _ => None,
        };
        if let Some(m) = preview {
            label(&painter, pts[pts.len() - 1], m.label(spacing));
        }
    }

    // Overlay, top-left and bottom-left of the image area.
    if let Some(f) = &session.frame {
        let (si, sl) = session.current;
        let series_len = session
            .set
            .as_ref()
            .and_then(|s| s.series.get(si))
            .map(|s| s.slices.len())
            .unwrap_or(0);
        let v: Viewport = session.view;
        let text = format!(
            "{}  {}x{}  {}-bit  slice {}/{}  W {:.0} L {:.0}  zoom {:.0}%{}{}{}{}",
            session
                .study
                .as_ref()
                .and_then(|s| s.modality.clone())
                .unwrap_or_else(|| "?".into()),
            f.width,
            f.height,
            f.bits_stored,
            sl + 1,
            series_len,
            session.window.1,
            session.window.0,
            v.scale / ppp * 100.0,
            if v.rotation != 0 {
                format!("  rot {}", v.rotation as u32 * 90)
            } else {
                String::new()
            },
            if v.smooth { "" } else { "  nearest" },
            if session.playing {
                format!("  playing {:.0} ms/frame", session.frame_ms())
            } else {
                String::new()
            },
            match spacing {
                Some(s) if s.at_detector => "  *spacing at detector",
                Some(_) => "",
                None => "  no pixel spacing: lengths in px",
            },
        );
        painter.text(
            rect.min + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::monospace(12.0),
            Color32::from_rgb(0xdd, 0xdd, 0xdd),
        );
        let hint = match state.tool {
            Tool::Pan => "wheel slices · pinch or Option/Alt+wheel zoom · drag pans · right-drag windows · space plays · 0 fit · 1 1:1 · r/R rotate · h/v flip · i interpolation · w reset window",
            Tool::Length => "Length: drag from one point to the other · Delete removes the last · Escape back to Pan",
            Tool::Angle => "Angle: click the first ray end, the vertex, then the second ray end · Escape back to Pan",
        };
        painter.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            hint,
            egui::FontId::proportional(11.0),
            Color32::from_rgb(0x88, 0x88, 0x88),
        );
    }
    drop_hint(ui, rect);
}

/// "Drop to open" while files hover over the window.
fn drop_hint(ui: &Ui, rect: egui::Rect) {
    let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
    if hovering {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(160));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop a study folder, zip, DICOM files or an HL7 order",
            egui::FontId::proportional(20.0),
            Color32::WHITE,
        );
    }
}

/// A report as text, or a PDF handed to the system viewer.
fn document_view(ui: &mut Ui, session: &mut Session, actions: &mut Actions) {
    let mut back = false;
    egui::Frame::new().inner_margin(12.0).show(ui, |ui| {
        back = ui.button("Back to images").clicked();
    });
    if back {
        session.close_document();
        return;
    }
    let Some((_, content)) = &session.document else {
        return;
    };
    egui::Frame::new()
        .inner_margin(12.0)
        .show(ui, |ui| match content {
            DocumentContent::Failed(e) => banner(ui, DANGER, e),
            DocumentContent::Report { title, text } => {
                ui.heading(title);
                egui::ScrollArea::both().id_salt("report").show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).monospace()).selectable(true));
                });
            }
            DocumentContent::Pdf { title, bytes, mime } => {
                ui.heading(title);
                ui.label(format!(
                    "{mime}, {} bytes. No PDF renderer is built in; the system viewer shows it.",
                    bytes.len()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Open in system viewer").clicked() {
                        actions.open_pdf = true;
                    }
                    if ui.button("Save PDF").clicked() {
                        actions.save_pdf = true;
                    }
                });
            }
        });
}

// ---------------------------------------------------------------------------
// Panels
// ---------------------------------------------------------------------------

fn series_panel(ui: &mut Ui, session: &mut Session, state: &mut UiState) {
    let Some(set) = &session.set else {
        ui.weak("No study loaded.");
        return;
    };
    // Thumbnails become egui textures once, lazily.
    if state.thumb_textures.len() != session.thumbs.len() {
        state.thumb_textures = (0..session.thumbs.len()).map(|_| None).collect();
    }
    let n = set.series.len();
    ui.weak(format!(
        "{} series, {} slices{}",
        n,
        set.slice_count(),
        if set.skipped.is_empty() {
            String::new()
        } else {
            format!(", {} skipped", set.skipped.len())
        }
    ));
    let mut select = None;
    for (i, series) in set.series.iter().enumerate() {
        let selected = session.current.0 == i;
        let response = ui
            .horizontal(|ui| {
                if state.thumb_textures[i].is_none() {
                    if let Some(t) = &session.thumbs[i] {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [t.width as usize, t.height as usize],
                            &t.rgba,
                        );
                        state.thumb_textures[i] = Some(ui.ctx().load_texture(
                            format!("thumb{i}"),
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                if let Some(tex) = &state.thumb_textures[i] {
                    ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(48.0, 48.0)));
                }
                ui.selectable_label(
                    selected,
                    format!(
                        "{}\n{} {}×{}, {} slices",
                        series.label(),
                        series.modality,
                        series.cols,
                        series.rows,
                        series.slices.len()
                    ),
                )
            })
            .inner;
        if response.clicked() && !selected {
            select = Some(i);
        }
    }
    if let Some(i) = select {
        session.show_slice(i, 0, true);
    }
    // Slice slider: the touch and trackpad way through a series.
    let (si, sl) = session.current;
    let n = session
        .set
        .as_ref()
        .and_then(|s| s.series.get(si))
        .map(|s| s.slices.len())
        .unwrap_or(0);
    if n > 1 {
        let mut pos = sl + 1;
        if ui
            .add(egui::Slider::new(&mut pos, 1..=n).text("slice"))
            .changed()
            && pos - 1 != sl
        {
            session.show_slice(si, pos - 1, false);
        }
    }
    // Reports and PDFs found next to the images.
    let docs: Vec<(usize, String)> = session
        .set
        .as_ref()
        .map(|s| {
            s.documents
                .iter()
                .enumerate()
                .map(|(i, d)| (i, d.label()))
                .collect()
        })
        .unwrap_or_default();
    if !docs.is_empty() {
        ui.separator();
        ui.weak(format!("{} document(s)", docs.len()));
        let open = session.document.as_ref().map(|(i, _)| *i);
        let mut pick = None;
        for (i, label) in docs {
            if ui.selectable_label(open == Some(i), label).clicked() {
                pick = Some(i);
            }
        }
        if let Some(i) = pick {
            session.open_document(i);
        }
    }
}

fn window_panel(ui: &mut Ui, session: &mut Session) {
    let Some(f) = session.frame else {
        ui.weak("No image.");
        return;
    };
    if f.color {
        ui.weak("Colour image: shown as stored, windowing does not apply.");
        return;
    }
    let (lo, hi) = f.value_range;
    let span = (hi - lo).max(1.0);
    let (mut c, mut w) = session.window;
    ui.add(egui::Slider::new(&mut c, lo..=hi).text("centre"));
    ui.add(egui::Slider::new(&mut w, 1.0..=span * 2.0).text("width"));
    session.window = (c, w);
    ui.horizontal(|ui| {
        if ui.button("File default").clicked() {
            session.reset_window();
        }
        if ui.button("Full range").clicked() {
            session.window = dicomscope_core::dicom::fallback_window(f.value_range);
        }
        if let Some((dc, dw)) = f.default_window {
            ui.weak(format!("file: W {dw:.0} L {dc:.0}"));
        }
    });
}

fn tag_table(ui: &mut Ui, id: &str, filter: &mut String, rows: &[TagRow], height: f32) {
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("Filter by tag, keyword or value")
            .desired_width(f32::INFINITY),
    );
    let shown: Vec<&TagRow> = rows.iter().filter(|r| r.matches(filter)).collect();
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(height)
        .show_rows(ui, row_height, shown.len(), |ui, range| {
            egui::Grid::new(format!("{id}-grid"))
                .striped(true)
                .show(ui, |ui| {
                    for r in &shown[range] {
                        let indent = "  ".repeat(r.depth.min(3));
                        ui.monospace(format!("{indent}{}", r.tag));
                        ui.monospace(&r.keyword);
                        ui.monospace(&r.vr);
                        ui.monospace(&r.value);
                        ui.end_row();
                    }
                });
        });
}

fn hl7_panel(ui: &mut Ui, session: &Session) {
    let Some(h) = &session.hl7 else {
        ui.weak("No HL7 message loaded.");
        return;
    };
    ui.label(&h.summary);
    for w in &h.warnings {
        banner(ui, WARN, &format!("Parse warning: {w}"));
    }
    // The raw message with the four order fields highlighted, as in the
    // browser: spans are byte offsets into the raw text.
    let mut job = egui::text::LayoutJob::default();
    let mono = egui::FontId::monospace(12.0);
    let plain = egui::TextFormat::simple(mono.clone(), Color32::from_gray(0xcc));
    let mut spans: Vec<(usize, usize, OrderField)> =
        h.spans.iter().map(|(s, f)| (s.start, s.end, *f)).collect();
    spans.sort_by_key(|s| s.0);
    let mut pos = 0;
    let raw = h.raw.replace('\r', "\n");
    for (start, end, field) in spans {
        if start < pos || end > raw.len() || start > end {
            continue;
        }
        job.append(&raw[pos..start], 0.0, plain.clone());
        let color = match field {
            OrderField::StudyUid => Color32::from_rgb(0x7c, 0xb3, 0xff),
            OrderField::Accession => Color32::from_rgb(0xff, 0xc1, 0x5e),
            OrderField::PatientId => Color32::from_rgb(0x8f, 0xe3, 0x9a),
            OrderField::ProcedureId => Color32::from_rgb(0xe3, 0x9a, 0xff),
        };
        let mut fmt = egui::TextFormat::simple(mono.clone(), Color32::BLACK);
        fmt.background = color;
        job.append(&raw[start..end], 0.0, fmt);
        pos = end;
    }
    job.append(&raw[pos..], 0.0, plain);
    egui::ScrollArea::both()
        .id_salt("hl7")
        .max_height(220.0)
        .show(ui, |ui| {
            ui.label(job);
        });
    ui.horizontal_wrapped(|ui| {
        ui.weak("highlighted:");
        for (f, name) in [
            (OrderField::StudyUid, "Study UID"),
            (OrderField::Accession, "Accession"),
            (OrderField::PatientId, "Patient ID"),
            (OrderField::ProcedureId, "Procedure ID"),
        ] {
            let path = h
                .order
                .source_path(f)
                .map(str::to_string)
                .unwrap_or_else(|| "absent".into());
            ui.weak(format!("{name} {path}"));
        }
    });
    for w in &h.order.warnings {
        banner(ui, DANGER, &format!("Order warning: {w}"));
    }
}

fn link_panel(ui: &mut Ui, session: &Session) {
    let Some(l) = &session.linkage else {
        ui.weak("Load both a DICOM study and an HL7 order to compare their identifiers.");
        return;
    };
    if l.linked_with_patient_mismatch() {
        banner(
            ui,
            DANGER,
            &format!(
                "Patient mismatch on a linked study. The study links to this order by {}, but Patient ID (0010,0020) and PID-3.1 disagree. The images are filed under an order for a different patient.",
                l.path.label()
            ),
        );
    } else if l.path == LinkPath::None {
        banner(ui, WARN, "No link. Neither the Study Instance UID nor the Accession Number matches. The FHIR ImagingStudy carries no basedOn.");
    } else {
        let patient = match l.patient_match {
            Some(true) => "Patient identifiers agree.",
            Some(false) => "",
            None => "Patient identifier is absent on one side, so it could not be compared.",
        };
        banner(ui, OK, &format!("Linked by {}. {patient}", l.path.label()));
    }
    if let Some(k) = l.key_conflict() {
        let title = match k {
            KeyConflict::AccessionDiffers => {
                "Keys disagree: accession number differs on a UID-linked study. "
            }
            KeyConflict::StudyUidDiffers => {
                "Keys disagree: study UID differs on an accession-linked study. "
            }
        };
        banner(ui, DANGER, &format!("{title}{}", k.explanation()));
    }
    let authority = match session
        .hl7
        .as_ref()
        .and_then(|h| h.patient_authority.clone())
    {
        Some(a) => format!("assigning authority {a} from PID-3.4"),
        None => "no assigning authority in message".to_string(),
    };
    let path_of = |f: OrderField| -> String {
        session
            .hl7
            .as_ref()
            .and_then(|h| h.order.source_path(f))
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} (absent)", f.paths().join(" / ")))
    };
    // UIDs stay on one line and the table scrolls sideways if it must.
    egui::ScrollArea::horizontal()
        .id_salt("link-scroll")
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            grid_link(ui, l, &authority, &path_of);
        });
}

fn grid_link(
    ui: &mut Ui,
    l: &dicomscope_core::link::Linkage,
    authority: &str,
    path_of: &dyn Fn(OrderField) -> String,
) {
    egui::Grid::new("link")
        .striped(true)
        .num_columns(5)
        .show(ui, |ui| {
            for (name, tag, field, pair, note) in [
                (
                    "Study Instance UID",
                    "(0020,000D)",
                    OrderField::StudyUid,
                    &l.study_uid,
                    String::new(),
                ),
                (
                    "Accession Number",
                    "(0008,0050)",
                    OrderField::Accession,
                    &l.accession,
                    String::new(),
                ),
                (
                    "Patient ID",
                    "(0010,0020)",
                    OrderField::PatientId,
                    &l.patient_id,
                    authority.to_string(),
                ),
                (
                    "Requested Procedure ID",
                    "(0040,1001)",
                    OrderField::ProcedureId,
                    &l.procedure_id,
                    String::new(),
                ),
            ] {
                ui.label(name);
                ui.label(RichText::new(format!("{}\n{tag}", path_of(field))).small());
                pair_cells(ui, pair);
                ui.label(RichText::new(note).small());
                ui.end_row();
            }
        });
}

fn pair_cells(ui: &mut Ui, pair: &Pair) {
    let absent = || RichText::new("absent").weak();
    match &pair.dicom {
        Some(v) => ui.monospace(v),
        None => ui.label(absent()),
    };
    let (text, color) = match pair.matches() {
        Some(true) => ("=", OK),
        Some(false) => ("≠", DANGER),
        None => ("–", Color32::GRAY),
    };
    ui.label(RichText::new(text).color(color).strong());
    match &pair.hl7 {
        Some(v) => ui.monospace(v),
        None => ui.label(absent()),
    };
}

fn worklist_panel(ui: &mut Ui, session: &Session, state: &mut UiState, actions: &mut Actions) {
    match &session.worklist {
        None => {
            ui.weak("Load an HL7 order; the worklist item is built from it alone. Load a study as well to follow the chain order → worklist → image.");
        }
        Some(Err(e)) => {
            banner(ui, DANGER, &format!("Refused: {e} A worklist SCP that filled this in with a placeholder would hand the modality an entry the RIS cannot reconcile; mwlkit refuses instead."));
        }
        Some(Ok(out)) => {
            match out.item.study_uid.origin {
                StudyUidOrigin::FromOrder(src) => banner(
                    ui,
                    OK,
                    &format!("Study Instance UID {} taken from the order ({}). The RIS knows this UID.", out.item.study_uid.value, src.path()),
                ),
                StudyUidOrigin::Generated(from) => banner(
                    ui,
                    WARN,
                    &format!(
                        "Study Instance UID {} generated {from}: the order carried none. The modality will copy this UID into every image, and the RIS has never seen it.",
                        out.item.study_uid.value
                    ),
                ),
            }
            for w in &out.item.warnings {
                banner(ui, WARN, &format!("Worklist note: {w}"));
            }
            if let Some(chain) = &session.chain {
                for f in &chain.findings {
                    let (color, title) = match f {
                        ChainFinding::Intact => (OK, "Chain intact. "),
                        ChainFinding::GeneratedUidReachedImage => {
                            (DANGER, "Generated UID reached the image. ")
                        }
                        ChainFinding::GeneratedUidNotInImage => {
                            (DANGER, "Generated UID is not in the image. ")
                        }
                        ChainFinding::OrderUidReplacedDownstream => {
                            (DANGER, "Order UID replaced downstream. ")
                        }
                        ChainFinding::TruncatedAccessionBreaksLink => {
                            (DANGER, "Truncated accession number breaks the RIS link. ")
                        }
                        ChainFinding::TruncatedAccessionAndImageDiffers => (
                            DANGER,
                            "Truncated accession number, and the image differs. ",
                        ),
                    };
                    banner(ui, color, &format!("{title}{}", f.explanation()));
                }
                egui::ScrollArea::horizontal()
                    .id_salt("chain-scroll")
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        grid_chain(ui, chain);
                    });
            }
            ui.horizontal(|ui| {
                if ui.button("Save order.mwl.dcm").clicked() {
                    actions.save_worklist = true;
                }
                ui.weak(format!(
                    "{} bytes, Explicit VR Little Endian",
                    out.bytes.len()
                ));
            });
            tag_table(
                ui,
                "worklist-tags",
                &mut state.worklist_filter,
                &out.rows,
                240.0,
            );
        }
    }
}

fn grid_chain(ui: &mut Ui, chain: &dicomscope_core::link::Chain) {
    egui::Grid::new("chain")
        .striped(true)
        .num_columns(6)
        .show(ui, |ui| {
            ui.strong("Identifier");
            ui.strong("Order");
            ui.label("");
            ui.strong("Worklist");
            ui.label("");
            ui.strong("Image");
            ui.end_row();
            for r in &chain.rows {
                let arrow = |m: Option<bool>| match m {
                    Some(true) => RichText::new("→").color(OK).strong(),
                    Some(false) => RichText::new("≠").color(DANGER).strong(),
                    None => RichText::new("–").color(Color32::GRAY),
                };
                let cell = |v: &Option<String>| match v {
                    Some(v) => RichText::new(v).monospace(),
                    None => RichText::new("absent").weak(),
                };
                ui.label(r.name);
                ui.label(cell(&r.order));
                ui.label(arrow(r.order_to_worklist()));
                ui.label(cell(&r.worklist));
                ui.label(arrow(r.worklist_to_image()));
                ui.label(cell(&r.image));
                ui.end_row();
            }
        });
}

fn fhir_panel(ui: &mut Ui, session: &Session, state: &mut UiState, actions: &mut Actions) {
    let Some(f) = &session.fhir else {
        ui.weak("Load both a DICOM study and an HL7 order; the FHIR R4 bundle is generated from the pair.");
        return;
    };
    ui.horizontal(|ui| {
        for t in FhirTab::ALL {
            ui.selectable_value(&mut state.fhir_tab, t, t.label());
        }
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(state.fhir_tab.text(f).to_string());
            state.status = Some(format!(
                "Copied {} characters.",
                state.fhir_tab.text(f).chars().count()
            ));
        }
        if ui.button("Save bundle.json").clicked() {
            actions.save_fhir = true;
        }
    });
    egui::ScrollArea::both()
        .id_salt("fhir")
        .max_height(360.0)
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(state.fhir_tab.text(f)).monospace().size(11.0))
                    .selectable(true),
            );
        });
}
