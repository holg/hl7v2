//! Root component and top-level state.
//!
//! Pixel data never enters a signal: a decoded `Frame` goes straight to the
//! renderer and is dropped. Signals hold metadata, the window and the view.
//! The loaded study set (archives and directly read files) lives in an `Arc`
//! inside a signal; slices and documents are decoded from it on demand.

use crate::dicom::pixels::FrameInfo;
use crate::dicom::sr::{self, DocumentKind};
use crate::dicom::{self, FileEntry, Study, StudySet, TagRow};
use crate::error::AppError;
use crate::link::{self, Linkage};
use crate::measure::Measurement;
use crate::render::{Renderer, Uniforms};
use crate::thumbnail::{thumbnail, Thumbnail};
use crate::ui::document_view::DocumentContent;
use crate::ui::file_drop::FilesResult;
use crate::ui::viewer::{sync_backing_size, Tool, ViewControls, Viewport};
use crate::ui::{DocumentView, FileDrop, Hl7View, LinkPanel, SeriesPanel, TagTree, WindowControls};
use hl7kit::order::{Order, OrderField};
use hl7kit::{Message, Span};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos::web_sys::{KeyboardEvent, MouseEvent, WheelEvent};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// The instance currently on screen.
#[derive(Clone, PartialEq)]
struct LoadedDicom {
    name: String,
    study: Study,
    tags: Arc<Vec<TagRow>>,
    /// `Err` when the header parsed but the pixels did not.
    frame: Result<FrameInfo, String>,
}

#[derive(Clone, PartialEq)]
struct LoadedHl7 {
    name: String,
    raw: String,
    order: Order,
    warnings: Vec<String>,
    summary: String,
}

#[derive(Clone, PartialEq)]
enum GpuState {
    Starting,
    Ready,
    Failed(String),
}

type RendererHandle = Rc<RefCell<Option<Renderer>>>;
type PendingFrame = Rc<RefCell<Option<dicom::Frame>>>;
type CineHandle = Rc<RefCell<Option<IntervalHandle>>>;
/// Measurements per (file, frame), kept while scrolling.
type Marks = HashMap<(usize, u32), Vec<Measurement>>;

const THUMB_SIZE: u32 = 96;

#[component]
pub fn App() -> impl IntoView {
    let study_set = RwSignal::new(None::<Arc<StudySet>>);
    let thumbs = RwSignal::new(None::<Arc<Vec<Option<Arc<Thumbnail>>>>>);
    let current = RwSignal::new((0usize, 0usize));
    let current_document = RwSignal::new(None::<usize>);
    let document = RwSignal::new(None::<DocumentContent>);
    let dicom_state = RwSignal::new(None::<Arc<LoadedDicom>>);
    let hl7_state = RwSignal::new(None::<Arc<LoadedHl7>>);
    let window = RwSignal::new((0.0f32, 1.0f32));
    let view = RwSignal::new(Viewport::default());
    let error = RwSignal::new(None::<String>);
    let gpu = RwSignal::new(GpuState::Starting);
    let playing = RwSignal::new(false);
    let marks = RwSignal::new(Marks::new());
    // Non-Send GPU objects live in local storage, reachable from Send closures
    // through a Copy handle.
    let renderer: StoredValue<RendererHandle, LocalStorage> = StoredValue::new_local(Rc::default());
    let pending: StoredValue<PendingFrame, LocalStorage> = StoredValue::new_local(Rc::default());
    let cine: StoredValue<CineHandle, LocalStorage> = StoredValue::new_local(Rc::default());
    let canvas_ref = NodeRef::<html::Canvas>::new();
    let viewer_ref = NodeRef::<html::Div>::new();
    let started = StoredValue::new(false);

    let frame_info = move || dicom_state.get().and_then(|d| d.frame.clone().ok());
    let image_size = Signal::derive(move || frame_info().map(|f| (f.width, f.height)));
    let controls = ViewControls::new(view, canvas_ref, image_size);

    // Draw with the current window and view. The canvas backing store is
    // synced to its CSS box first, so the surface is never stale.
    let redraw = move || {
        let size = controls.canvas_size();
        let (c, w) = window.get_untracked();
        let v = view.get_untracked();
        let (inverted, color) = dicom_state.with_untracked(|d| {
            d.as_ref()
                .and_then(|d| d.frame.as_ref().ok())
                .map(|f| (f.inverted, f.color))
                .unwrap_or((false, false))
        });
        renderer.with_value(|h| {
            if let Some(r) = h.borrow_mut().as_mut() {
                r.resize(size.0, size.1);
                r.set_uniforms(Uniforms {
                    center: c,
                    width: w,
                    invert: u32::from(inverted),
                    interp: u32::from(v.smooth),
                    scale: v.scale,
                    tx: v.tx,
                    ty: v.ty,
                    color: u32::from(color),
                    rot: u32::from(v.rotation),
                    flip: u32::from(v.flip_h) | (u32::from(v.flip_v) << 1),
                    _pad0: 0,
                    _pad1: 0,
                });
                if let Err(e) = r.draw() {
                    error.set(Some(e.to_string()));
                }
            }
        });
    };

    window_event_listener(leptos::ev::resize, move |_| {
        controls.fit();
        redraw();
    });

    // GPU init, once the canvas is in the DOM.
    Effect::new(move |_| {
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        if started.get_value() {
            return;
        }
        started.set_value(true);
        sync_backing_size(&canvas);
        spawn_local(async move {
            match Renderer::new(canvas).await {
                Ok(mut r) => {
                    if let Some(frame) = pending.with_value(|p| p.borrow_mut().take()) {
                        r.upload(&frame);
                    }
                    renderer.with_value(|h| *h.borrow_mut() = Some(r));
                    gpu.set(GpuState::Ready);
                    controls.fit();
                    redraw();
                }
                Err(e) => gpu.set(GpuState::Failed(e.to_string())),
            }
        });
    });

    // Window or view changes rewrite a 48-byte uniform and redraw. Nothing else.
    Effect::new(move |_| {
        window.track();
        view.track();
        redraw();
    });

    // Measurements belong to the slice they were drawn on.
    let current_key = move || {
        let set = study_set.get_untracked()?;
        let (s, i) = current.get_untracked();
        let (_, slice) = set.slice(s, i)?;
        Some((slice.file, slice.frame))
    };
    let stash_marks = move || {
        if let Some(key) = current_key() {
            let ms = controls.measurements.get_untracked();
            marks.update(|m| {
                if ms.is_empty() {
                    m.remove(&key);
                } else {
                    m.insert(key, ms);
                }
            });
        }
    };

    // Decode and show one slice. `reset` re-fits the view and takes the
    // window from the file; scrolling within a series keeps both.
    let show_slice = move |series: usize, slice: usize, reset: bool| {
        let Some(set) = study_set.get_untracked() else {
            return;
        };
        let Some((_, s)) = set.slice(series, slice) else {
            return;
        };
        stash_marks();
        let file = &set.files[s.file];
        let bytes = match set.bytes(s.file) {
            Ok(b) => b,
            Err(e) => return error.set(Some(e)),
        };
        let obj = match dicom::load(&bytes) {
            Ok(obj) => obj,
            Err(e) => return error.set(Some(format!("{}: {e}", file.name))),
        };
        let study = Study::from_object(&obj);
        let tags = Arc::new(dicom::tag_rows(&obj));
        let frame = dicom::decode_frame(&obj, s.frame);
        drop(obj);
        let previous_size = image_size.get_untracked();
        let info = match &frame {
            Ok(f) => {
                error.set(None);
                Ok(FrameInfo::from(f))
            }
            Err(e) => {
                error.set(Some(format!("{}: {e}", file.name)));
                Err(e.to_string())
            }
        };
        current.set((series, slice));
        controls.draft.set(None);
        controls.measurements.set(
            marks
                .with_untracked(|m| m.get(&(s.file, s.frame)).cloned())
                .unwrap_or_default(),
        );
        dicom_state.set(Some(Arc::new(LoadedDicom {
            name: file.name.clone(),
            study,
            tags,
            frame: info,
        })));
        if let Ok(frame) = frame {
            let size_changed = previous_size != Some((frame.width, frame.height));
            if reset || window.get_untracked() == (0.0, 1.0) {
                window.set(
                    frame
                        .default_window
                        .unwrap_or_else(|| dicom::fallback_window(frame.value_range)),
                );
            }
            let uploaded = renderer.with_value(|h| match h.borrow_mut().as_mut() {
                Some(r) => {
                    r.upload(&frame);
                    true
                }
                None => false,
            });
            if uploaded {
                if reset || size_changed {
                    controls.fit();
                }
                redraw();
            } else {
                pending.with_value(|p| *p.borrow_mut() = Some(frame));
            }
        }
    };

    // One thumbnail per series from its middle slice. Decoding one frame per
    // series is cheap next to the scan itself.
    let build_thumbs = move |set: &StudySet| -> Vec<Option<Arc<Thumbnail>>> {
        set.series
            .iter()
            .map(|series| {
                let s = series.slices.get(series.slices.len() / 2)?;
                let bytes = set.bytes(s.file).ok()?;
                let obj = dicom::load(&bytes).ok()?;
                let frame = dicom::decode_frame(&obj, s.frame).ok()?;
                Some(Arc::new(thumbnail(&frame, THUMB_SIZE)))
            })
            .collect()
    };

    let stop_cine = move || {
        cine.with_value(|h| {
            if let Some(handle) = h.borrow_mut().take() {
                handle.clear();
            }
        });
        playing.set(false);
    };

    let show_document = move |index: usize| {
        let Some(set) = study_set.get_untracked() else {
            return;
        };
        let Some(doc) = set.documents.get(index) else {
            return;
        };
        let name = set.files[doc.file].name.clone();
        let content = match set
            .bytes(doc.file)
            .and_then(|b| dicom::load(&b).map_err(|e| e.to_string()))
        {
            Err(e) => DocumentContent::Failed(format!("{name}: {e}")),
            Ok(obj) => match doc.kind {
                DocumentKind::EncapsulatedPdf => match sr::encapsulated_document(&obj) {
                    Some((bytes, mime)) => DocumentContent::Pdf {
                        title: doc.title.clone(),
                        bytes: Arc::new(bytes),
                        mime,
                    },
                    None => DocumentContent::Failed(format!(
                        "{name}: no Encapsulated Document (0042,0011)"
                    )),
                },
                DocumentKind::StructuredReport | DocumentKind::KeyObjectSelection => {
                    DocumentContent::Report {
                        title: doc.title.clone(),
                        lines: Arc::new(sr::render_sr(&obj)),
                    }
                }
            },
        };
        current_document.set(Some(index));
        document.set(Some(content));
    };

    let on_dicom = Callback::new(move |result: FilesResult| {
        let files = match result {
            Ok(v) => v,
            Err(e) => return error.set(Some(e.to_string())),
        };
        let entries: Vec<FileEntry> = files
            .into_iter()
            .map(|(name, bytes)| FileEntry::new(name, bytes))
            .collect();
        let set = StudySet::scan(entries);
        if set.is_empty() {
            let reasons: Vec<String> = set
                .skipped
                .iter()
                .take(5)
                .map(|s| format!("{}: {}", s.name, s.reason))
                .collect();
            error.set(Some(format!(
                "No displayable DICOM image or document among {} file(s). {}",
                set.skipped.len(),
                reasons.join(" · ")
            )));
            return;
        }
        stop_cine();
        marks.set(Marks::new());
        controls.clear_measurements();
        window.set((0.0, 1.0));
        thumbs.set(Some(Arc::new(build_thumbs(&set))));
        let has_images = !set.series.is_empty();
        let has_documents = !set.documents.is_empty();
        study_set.set(Some(Arc::new(set)));
        current_document.set(None);
        document.set(None);
        if has_images {
            show_slice(0, 0, true);
        } else if has_documents {
            error.set(None);
            show_document(0);
        }
    });

    let on_select = Callback::new(move |(series, slice): (usize, usize)| {
        let reset = current.get_untracked().0 != series;
        if reset {
            stop_cine();
        }
        show_slice(series, slice, reset);
    });
    let on_document = Callback::new(move |index: usize| show_document(index));

    let slice_count = move || {
        let (s, _) = current.get_untracked();
        study_set
            .get_untracked()
            .and_then(|st| st.series.get(s).map(|x| x.slices.len()))
            .unwrap_or(0)
    };
    // Step within the current series; clamps at both ends, or wraps for cine.
    let step_slice = move |delta: i64, wrap: bool| {
        let (s, i) = current.get_untracked();
        let count = slice_count() as i64;
        if count < 2 {
            return;
        }
        let target = if wrap {
            (i as i64 + delta).rem_euclid(count)
        } else {
            (i as i64 + delta).clamp(0, count - 1)
        } as usize;
        if target != i {
            show_slice(s, target, false);
        }
    };

    let toggle_cine = move || {
        if playing.get_untracked() {
            stop_cine();
            return;
        }
        if slice_count() < 2 {
            return;
        }
        let ms = frame_info()
            .and_then(|f| f.frame_time_ms)
            .unwrap_or(100.0)
            .clamp(20.0, 2000.0);
        match set_interval_with_handle(
            move || step_slice(1, true),
            Duration::from_millis(ms as u64),
        ) {
            Ok(handle) => {
                cine.with_value(|h| *h.borrow_mut() = Some(handle));
                playing.set(true);
            }
            Err(_) => error.set(Some("the browser refused to start a timer for cine".into())),
        }
    };

    let on_hl7 = Callback::new(move |result: FilesResult| {
        let (name, bytes) = match result {
            Ok(mut v) if !v.is_empty() => v.remove(0),
            Ok(_) => return,
            Err(e) => return error.set(Some(e.to_string())),
        };
        // Latin-1 order messages are common; keep going and say so.
        let msg = match Message::parse_bytes(&bytes) {
            Ok(m) => m,
            Err(hl7kit::ParseError::InvalidUtf8 { .. }) => match Message::parse_lossy(&bytes) {
                Ok(m) => m,
                Err(e) => return error.set(Some(AppError::from(e).to_string())),
            },
            Err(e) => return error.set(Some(AppError::from(e).to_string())),
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
        error.set(None);
        hl7_state.set(Some(Arc::new(LoadedHl7 {
            name,
            raw: msg.raw().to_string(),
            order,
            warnings,
            summary,
        })));
    });

    let linkage = Memo::new(move |_| -> Option<Linkage> {
        let d = dicom_state.get()?;
        let h = hl7_state.get()?;
        Some(link::resolve(&d.study, &h.order))
    });

    let range = Signal::derive(move || frame_info().map(|f| f.value_range).unwrap_or((0.0, 1.0)));
    let default_window = Signal::derive(move || frame_info().and_then(|f| f.default_window));
    let is_color = Signal::derive(move || frame_info().map(|f| f.color).unwrap_or(false));
    let overlay = move || {
        let d = dicom_state.get()?;
        let f = d.frame.clone().ok()?;
        let (c, w) = window.get();
        let v = view.get();
        let zoom = v.scale / crate::ui::viewer::device_pixel_ratio() * 100.0;
        let mut s = format!(
            "{} {}x{} {}-bit {} zoom {:.0}%{}",
            d.study.modality.clone().unwrap_or_else(|| "?".into()),
            f.width,
            f.height,
            f.bits_stored,
            if f.color {
                "RGB".to_string()
            } else {
                format!(" C {c:.0} / W {w:.0} ")
            },
            zoom,
            if v.smooth { "" } else { " nearest" }
        );
        if v.rotation != 0 {
            s.push_str(&format!("  rot {}°", u32::from(v.rotation) * 90));
        }
        if v.flip_h {
            s.push_str("  flip H");
        }
        if v.flip_v {
            s.push_str("  flip V");
        }
        if f.inverted {
            s.push_str("  MONOCHROME1");
        }
        let (series, slice) = current.get();
        if let Some(set) = study_set.get() {
            if let Some(sr) = set.series.get(series) {
                if sr.slices.len() > 1 {
                    s.push_str(&format!("  slice {}/{}", slice + 1, sr.slices.len()));
                }
                if f.frame_count > 1 {
                    s.push_str(&format!("  frame {}/{}", f.frame_index + 1, f.frame_count));
                }
            }
        }
        if playing.get() {
            s.push_str("  ▶");
        }
        match f.spacing {
            Some(sp) => s.push_str(&format!("  {:.3}x{:.3} mm/px", sp.col_mm, sp.row_mm)),
            None => s.push_str("  no pixel spacing"),
        }
        Some(s)
    };
    let loaded_name = Signal::derive(move || {
        let set = study_set.get()?;
        Some(if set.files.len() == 1 {
            set.files[0].name.clone()
        } else {
            format!(
                "{} files, {} series, {:.0} MB resident",
                set.files.len(),
                set.series.len(),
                set.resident_bytes() as f64 / 1e6
            )
        })
    });

    // Measurement overlay: source points mapped through the same transform
    // the shader uses, expressed in CSS pixels on an SVG over the canvas.
    let marks_svg = move || {
        let img = image_size.get()?;
        let v = view.get();
        let dpr = crate::ui::viewer::device_pixel_ratio();
        let spacing = frame_info().and_then(|f| f.spacing);
        let to_css = move |p: (f32, f32)| {
            let c = v.source_to_canvas(p, img);
            (c.0 / dpr, c.1 / dpr)
        };
        let mut items: Vec<AnyView> = Vec::new();
        let draw = |items: &mut Vec<AnyView>,
                    points: &[(f32, f32)],
                    label: Option<String>,
                    draft: bool| {
            let css: Vec<_> = points.iter().map(|&p| to_css(p)).collect();
            for pair in css.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                items.push(
                    view! {
                        <line x1=a.0 y1=a.1 x2=b.0 y2=b.1 class:draft=draft />
                    }
                    .into_any(),
                );
            }
            for p in &css {
                items.push(view! { <circle cx=p.0 cy=p.1 r="3" /> }.into_any());
            }
            if let (Some(text), Some(p)) = (label, css.last()) {
                items.push(view! { <text x=p.0 + 6.0 y=p.1 - 6.0>{text}</text> }.into_any());
            }
        };
        for m in controls.measurements.get() {
            draw(&mut items, &m.points(), Some(m.label(spacing)), false);
        }
        if let Some(d) = controls.draft.get() {
            let label = match (d.tool, d.points.as_slice()) {
                (Tool::Length, [a, b]) => Some(Measurement::Length { a: *a, b: *b }.label(spacing)),
                (Tool::Angle, [a, vtx, c]) => Some(
                    Measurement::Angle {
                        a: *a,
                        vertex: *vtx,
                        c: *c,
                    }
                    .label(spacing),
                ),
                _ => None,
            };
            draw(&mut items, &d.points, label, true);
        }
        (!items.is_empty()).then(|| view! { <svg class="marks">{items}</svg> })
    };

    let focus_viewer = move || {
        if let Some(div) = viewer_ref.get_untracked() {
            let _ = div.focus();
        }
    };
    // Wheel scrolls slices when there are several; Ctrl/Cmd+wheel, or wheel
    // on a single image, zooms.
    let on_wheel = move |ev: WheelEvent| {
        if slice_count() > 1 && !ev.ctrl_key() && !ev.meta_key() {
            ev.prevent_default();
            step_slice(if ev.delta_y() > 0.0 { 1 } else { -1 }, false);
        } else if let Some(canvas) = canvas_ref.get_untracked() {
            controls.on_wheel(&canvas, &ev);
        }
    };
    let with_canvas = move |f: &dyn Fn(&leptos::web_sys::HtmlCanvasElement)| {
        if let Some(canvas) = canvas_ref.get_untracked() {
            f(&canvas);
        }
    };
    let on_mouse_down = move |ev: MouseEvent| {
        focus_viewer();
        with_canvas(&|c| controls.on_mouse_down(c, &ev));
    };
    let on_key = move |ev: KeyboardEvent| {
        let handled = match ev.key().as_str() {
            "PageDown" => {
                step_slice(1, false);
                true
            }
            "PageUp" => {
                step_slice(-1, false);
                true
            }
            "Home" => {
                step_slice(i64::MIN / 2, false);
                true
            }
            "End" => {
                step_slice(i64::MAX / 2, false);
                true
            }
            " " => {
                toggle_cine();
                true
            }
            _ => false,
        };
        if handled {
            ev.prevent_default();
        } else {
            controls.on_key(&ev);
        }
    };
    let tool_button = move |tool: Tool| {
        view! {
            <button type="button" class:active=move || controls.tool.get() == tool
                on:click=move |_| controls.set_tool(tool)>{tool.label()}</button>
        }
    };

    view! {
        <main>
            <h1>"dicomscope"</h1>
            <p class="sub">
                "Browser-only DICOM viewer with HL7 v2 order linkage. Demo for the "
                <code>"hl7kit"</code> " crate. Nothing leaves this page: no network requests after load."
            </p>

            {move || error.get().map(|e| view! { <p class="banner danger">{e}</p> })}
            {move || match gpu.get() {
                GpuState::Failed(e) => Some(view! { <p class="banner danger">{e}</p> }),
                _ => None,
            }}

            <div class="loadbar">
                <FileDrop label="DICOM: files, a folder or a zip" accept=".dcm,.zip,application/dicom,application/zip"
                    multiple=true loaded=loaded_name on_files=on_dicom />
                <FileDrop label="HL7 v2 order (ORM^O01 / OMI^O23)" accept=".hl7,.txt,x-application/hl7-v2+er7"
                    loaded=Signal::derive(move || hl7_state.get().map(|h| h.name.clone()))
                    on_files=on_hl7 />
            </div>

            <div class="cols">
                <section>
                    <h2>"Image"</h2>
                    <div class="viewer" node_ref=viewer_ref tabindex="0" on:keydown=on_key>
                        <canvas node_ref=canvas_ref width="1" height="1"
                            class:tool=move || controls.tool.get() != Tool::Pan
                            on:wheel=on_wheel
                            on:mousedown=on_mouse_down
                            on:mousemove=move |ev: MouseEvent| with_canvas(&|c| controls.on_mouse_move(c, &ev))
                            on:mouseup=move |ev: MouseEvent| with_canvas(&|c| controls.on_mouse_up(c, &ev))
                            on:mouseleave=move |_| controls.on_mouse_leave()
                            on:dblclick=move |_| { if controls.tool.get_untracked() == Tool::Pan { controls.fit() } } />
                        {marks_svg}
                        {move || match (frame_info(), gpu.get()) {
                            (_, GpuState::Failed(_)) => Some(view! { <p class="placeholder">"WebGPU unavailable; see the message above."</p> }.into_any()),
                            (_, GpuState::Starting) => Some(view! { <p class="placeholder">"Starting WebGPU…"</p> }.into_any()),
                            (None, _) => Some(view! { <p class="placeholder">{move || match dicom_state.get() {
                                Some(_) => "The header loaded but the pixel data could not be decoded; see the message above.",
                                None => "No DICOM image loaded.",
                            }}</p> }.into_any()),
                            (Some(_), GpuState::Ready) => None,
                        }}
                        <div class="overlay">{overlay}</div>
                    </div>
                    <div class="viewbar">
                        <button type="button" on:click=move |_| controls.fit()>"Fit"</button>
                        <button type="button" on:click=move |_| controls.one_to_one()>"1:1"</button>
                        <button type="button" on:click=move |_| controls.zoom_in()>"+"</button>
                        <button type="button" on:click=move |_| controls.zoom_out()>"−"</button>
                        <button type="button" title="Rotate clockwise (r)" on:click=move |_| controls.rotate(1)>"⟳"</button>
                        <button type="button" title="Rotate counter-clockwise (R)" on:click=move |_| controls.rotate(-1)>"⟲"</button>
                        <button type="button" title="Flip horizontal (h)" on:click=move |_| controls.flip_horizontal()>"⇋"</button>
                        <button type="button" title="Flip vertical (v)" on:click=move |_| controls.flip_vertical()>"⇅"</button>
                        <button type="button" title="Upright and fit (o)" on:click=move |_| controls.reset_orientation()>"Reset"</button>
                        <button type="button" on:click=move |_| controls.toggle_smooth()>
                            {move || if view.get().smooth { "Smooth: on" } else { "Smooth: off" }}
                        </button>
                        <button type="button" class:active=move || playing.get()
                            disabled=move || { current.track(); study_set.track(); slice_count() < 2 }
                            on:click=move |_| toggle_cine()>
                            {move || if playing.get() { "⏸ Pause" } else { "▶ Cine" }}
                        </button>
                    </div>
                    <div class="viewbar">
                        <span>"Tool:"</span>
                        {tool_button(Tool::Pan)}
                        {tool_button(Tool::Length)}
                        {tool_button(Tool::Angle)}
                        <button type="button" on:click=move |_| controls.remove_last_measurement()>"Undo"</button>
                        <button type="button" on:click=move |_| controls.clear_measurements()>"Clear"</button>
                        <small class="hint">"Length: drag. Angle: click three points. Delete removes the last, Escape returns to Pan. \
                            Wheel scrolls slices (Ctrl/Cmd+wheel zooms) · drag pans · arrows move · PageUp/PageDown, Home/End · space plays · 0 fit · 1 is 1:1 · r/R rotate · h/v flip · i interpolation"</small>
                    </div>
                    <SeriesPanel set=Signal::derive(move || study_set.get())
                        thumbs=Signal::derive(move || thumbs.get())
                        current=Signal::derive(move || current.get())
                        current_document=Signal::derive(move || current_document.get())
                        on_select=on_select on_document=on_document />
                    <h2>"Window"</h2>
                    {move || if is_color.get() {
                        view! { <p><small>"Colour image: shown as stored, windowing does not apply."</small></p> }.into_any()
                    } else {
                        view! { <WindowControls window=window range=range default_window=default_window /> }.into_any()
                    }}
                    <h2>"Linkage"</h2>
                    <LinkPanel linkage=Signal::derive(move || linkage.get()) />
                </section>
                <section>
                    <h2>"HL7 order"</h2>
                    {move || match hl7_state.get() {
                        None => view! { <p class="placeholder">"No HL7 message loaded."</p> }.into_any(),
                        Some(h) => {
                            let raw = h.raw.clone();
                            let spans: Vec<(Span, OrderField)> = h.order.spans.clone();
                            let warnings = h.warnings.clone();
                            let summary = h.summary.clone();
                            view! {
                                <Hl7View raw=Signal::derive(move || raw.clone())
                                    spans=Signal::derive(move || spans.clone())
                                    warnings=Signal::derive(move || warnings.clone())
                                    summary=Signal::derive(move || summary.clone()) />
                            }.into_any()
                        }
                    }}
                    {move || document.get().is_some().then(|| view! {
                        <h2>"Document"</h2>
                        <DocumentView content=Signal::derive(move || document.get()) />
                    })}
                    <h2>{move || match dicom_state.get() {
                        Some(d) => format!("DICOM tags: {}", d.name),
                        None => "DICOM tags".to_string(),
                    }}</h2>
                    <TagTree rows=Signal::derive(move || dicom_state.get().map(|d| d.tags.clone())) />
                </section>
            </div>
            <footer>
                "Transfer syntax: " {move || dicom_state.get().map(|d| {
                    format!("{} ({})", d.study.transfer_syntax, dicom::transfer_syntax::name(&d.study.transfer_syntax))
                }).unwrap_or_else(|| "–".into())}
            </footer>
        </main>
    }
}
