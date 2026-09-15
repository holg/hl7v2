//! dicomscope on the desktop: a winit window, a native wgpu surface, and the
//! same `dicomscope-core` code the browser runs.
//!
//! Milestone 1: open a folder, zip or file from the command line, render the
//! first series, scroll it, window it, zoom and pan. The panels come next.
//!
//! Keys: wheel or Up/Down scroll slices, PageUp/PageDown switch series,
//! Ctrl+wheel or +/- zoom, drag pans, right-drag windows, 0 fits, 1 is 1:1,
//! r/R rotate, h/v flip, i toggles interpolation, w resets the window,
//! Escape quits.

#![forbid(unsafe_code)]
#![warn(clippy::unwrap_used, clippy::expect_used)]

use dicomscope_core::dicom::pixels::FrameInfo;
use dicomscope_core::dicom::{self, StudySet};
use dicomscope_core::render::{Renderer, Uniforms};
use dicomscope_core::view::Viewport;
use dicomscope_core::{fs, AppError};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: dicomscope-desktop <folder | study.zip | file.dcm>");
            std::process::exit(2);
        }
    };
    let set = match fs::collect_inputs(&path).map(StudySet::scan) {
        Ok(set) if !set.is_empty() => set,
        Ok(_) => {
            eprintln!("{path}: no displayable DICOM image found");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "scanned {path}: {} series, {} slices, {} skipped",
        set.series.len(),
        set.slice_count(),
        set.skipped.len()
    );
    let event_loop = match EventLoop::new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot create an event loop: {e}");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(set);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop failed: {e}");
        std::process::exit(1);
    }
}

/// Everything the window needs. Created on `resumed`, as winit requires.
struct Gpu {
    window: Arc<Window>,
    renderer: Renderer,
}

struct App {
    set: StudySet,
    gpu: Option<Gpu>,
    /// (series, slice) on screen.
    current: (usize, usize),
    frame: Option<FrameInfo>,
    view: Viewport,
    window: (f32, f32),
    error: Option<String>,
    cursor: PhysicalPosition<f64>,
    drag: Option<Drag>,
    ctrl: bool,
}

#[derive(Clone, Copy)]
enum Drag {
    Pan,
    Window {
        start: PhysicalPosition<f64>,
        from: (f32, f32),
    },
}

impl App {
    fn new(set: StudySet) -> App {
        App {
            set,
            gpu: None,
            current: (0, 0),
            frame: None,
            view: Viewport::default(),
            window: (0.0, 1.0),
            error: None,
            cursor: PhysicalPosition::new(0.0, 0.0),
            drag: None,
            ctrl: false,
        }
    }

    fn canvas(&self) -> (u32, u32) {
        match &self.gpu {
            Some(g) => {
                let s = g.window.inner_size();
                (s.width.max(1), s.height.max(1))
            }
            None => (1, 1),
        }
    }

    fn image(&self) -> Option<(u32, u32)> {
        self.frame.as_ref().map(|f| (f.width, f.height))
    }

    /// Decode and upload one slice. `reset` refits the view and reloads the
    /// window from the file; scrolling within a series keeps both.
    fn show_slice(&mut self, series: usize, slice: usize, reset: bool) {
        let Some((_, s)) = self.set.slice(series, slice) else {
            return;
        };
        let (file, frame_index) = (s.file, s.frame);
        let name = self.set.files[file].name.clone();
        let frame = self
            .set
            .bytes(file)
            .map_err(AppError::FileRead)
            .and_then(|bytes| dicom::load(&bytes))
            .and_then(|obj| dicom::decode_frame(&obj, frame_index));
        match frame {
            Ok(frame) => {
                let info = FrameInfo::from(&frame);
                if let Some(g) = &mut self.gpu {
                    g.renderer.upload(&frame);
                }
                let size_changed = self.image() != Some((info.width, info.height));
                if reset || size_changed {
                    self.view = Viewport::default().fit(self.canvas(), (info.width, info.height));
                }
                if reset {
                    self.window = info
                        .default_window
                        .unwrap_or_else(|| dicom::fallback_window(info.value_range));
                }
                self.frame = Some(info);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("{name}: {e}"));
                eprintln!("{name}: {e}");
            }
        }
        self.current = (series, slice);
        self.update_title();
        self.redraw();
    }

    fn update_title(&self) {
        let Some(g) = &self.gpu else { return };
        let (si, sl) = self.current;
        let title = match self.set.series.get(si) {
            Some(series) => format!(
                "dicomscope  {}  slice {}/{}  W {:.0} L {:.0}  zoom {:.0}%",
                series.label(),
                sl + 1,
                series.slices.len(),
                self.window.1,
                self.window.0,
                self.view.scale * 100.0
            ),
            None => "dicomscope".to_string(),
        };
        g.window.set_title(&title);
    }

    fn redraw(&self) {
        if let Some(g) = &self.gpu {
            g.window.request_redraw();
        }
    }

    fn draw(&mut self) {
        let Some(frame) = &self.frame else { return };
        let (c, w) = self.window;
        let v = self.view;
        let uniforms = Uniforms {
            center: c,
            width: w,
            invert: u32::from(frame.inverted),
            interp: u32::from(v.smooth),
            scale: v.scale,
            tx: v.tx,
            ty: v.ty,
            color: u32::from(frame.color),
            rot: u32::from(v.rotation),
            flip: u32::from(v.flip_h) | (u32::from(v.flip_v) << 1),
            _pad0: 0,
            _pad1: 0,
        };
        if let Some(g) = &mut self.gpu {
            g.renderer.set_uniforms(uniforms);
            if let Err(e) = g.renderer.draw() {
                eprintln!("draw failed: {e}");
            }
        }
    }

    fn scroll_slices(&mut self, delta: i32) {
        let (si, sl) = self.current;
        let Some(series) = self.set.series.get(si) else {
            return;
        };
        let n = series.slices.len() as i32;
        let next = (sl as i32 + delta).clamp(0, n - 1) as usize;
        if next != sl {
            self.show_slice(si, next, false);
        }
    }

    fn switch_series(&mut self, delta: i32) {
        let n = self.set.series.len() as i32;
        let next = (self.current.0 as i32 + delta).clamp(0, n - 1) as usize;
        if next != self.current.0 {
            self.show_slice(next, 0, true);
        }
    }

    fn zoom(&mut self, factor: f32) {
        let (x, y) = (self.cursor.x as f32, self.cursor.y as f32);
        self.view = self.view.zoom_about(factor, x, y);
        self.update_title();
        self.redraw();
    }

    fn key(&mut self, event: KeyEvent, event_loop: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }
        let image = self.image();
        match event.logical_key.as_ref() {
            Key::Named(NamedKey::Escape) => event_loop.exit(),
            Key::Named(NamedKey::ArrowUp) => self.scroll_slices(-1),
            Key::Named(NamedKey::ArrowDown) => self.scroll_slices(1),
            Key::Named(NamedKey::Home) => self.show_slice(self.current.0, 0, false),
            Key::Named(NamedKey::End) => {
                let last = self.set.series[self.current.0]
                    .slices
                    .len()
                    .saturating_sub(1);
                self.show_slice(self.current.0, last, false);
            }
            Key::Named(NamedKey::PageUp) => self.switch_series(-1),
            Key::Named(NamedKey::PageDown) => self.switch_series(1),
            Key::Character("+") | Key::Character("=") => self.zoom(1.25),
            Key::Character("-") => self.zoom(0.8),
            Key::Character("0") => {
                if let Some(img) = image {
                    self.view = self.view.fit(self.canvas(), img);
                }
            }
            Key::Character("1") => {
                if let Some(img) = image {
                    self.view = self.view.one_to_one(self.canvas(), img);
                }
            }
            Key::Character("r") => {
                if let Some(img) = image {
                    self.view = self.view.rotate(1, img);
                }
            }
            Key::Character("R") => {
                if let Some(img) = image {
                    self.view = self.view.rotate(-1, img);
                }
            }
            Key::Character("h") => self.view = self.view.flip_horizontal(),
            Key::Character("v") => self.view = self.view.flip_vertical(),
            Key::Character("i") => self.view.smooth = !self.view.smooth,
            Key::Character("w") => {
                if let Some(f) = &self.frame {
                    self.window = f
                        .default_window
                        .unwrap_or_else(|| dicom::fallback_window(f.value_range));
                }
            }
            _ => return,
        }
        self.update_title();
        self.redraw();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("dicomscope")
            .with_inner_size(winit::dpi::LogicalSize::new(1024.0, 768.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("cannot create a window: {e}");
                event_loop.exit();
                return;
            }
        };
        // The display handle lets Vulkan and GL pick the right surface
        // extension on Wayland and X11. WGPU_BACKEND in the environment
        // overrides the backend choice, which is the Linux escape hatch.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
                event_loop.owned_display_handle(),
            ))
        });
        let surface = match instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cannot create a surface for the window: {e}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        let renderer =
            match pollster::block_on(Renderer::new(&instance, surface, size.width, size.height)) {
                Ok(r) => r,
                Err(AppError::NoWebGpu(e)) => {
                    eprintln!("no GPU adapter (Vulkan, Metal, DX12 or OpenGL): {e}");
                    event_loop.exit();
                    return;
                }
                Err(e) => {
                    eprintln!("{e}");
                    event_loop.exit();
                    return;
                }
            };
        self.gpu = Some(Gpu { window, renderer });
        self.show_slice(0, 0, true);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gpu {
                    g.renderer.resize(size.width, size.height);
                }
                if let Some(img) = self.image() {
                    self.view = self.view.fit(self.canvas(), img);
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::ModifiersChanged(m) => {
                self.ctrl = m.state().control_key() || m.state().super_key();
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(event, event_loop),
            WindowEvent::CursorMoved { position, .. } => {
                let previous = self.cursor;
                self.cursor = position;
                match self.drag {
                    Some(Drag::Pan) => {
                        let dx = (position.x - previous.x) as f32;
                        let dy = (position.y - previous.y) as f32;
                        self.view = self.view.pan(dx, dy);
                        self.redraw();
                    }
                    Some(Drag::Window { start, from }) => {
                        // Radiology convention: horizontal drag changes the
                        // width, vertical the centre; both scaled to the
                        // value range so a screen-width drag spans it.
                        let Some(f) = &self.frame else { return };
                        let span = (f.value_range.1 - f.value_range.0).max(1.0);
                        let (cw, ch) = self.canvas();
                        let dx = (position.x - start.x) as f32 / cw as f32 * span;
                        let dy = (position.y - start.y) as f32 / ch as f32 * span;
                        self.window = (
                            (from.0 + dy).clamp(f.value_range.0, f.value_range.1),
                            (from.1 + dx).max(1.0),
                        );
                        self.update_title();
                        self.redraw();
                    }
                    None => {}
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.drag = match (state, button) {
                    (ElementState::Pressed, MouseButton::Left) => Some(Drag::Pan),
                    (ElementState::Pressed, MouseButton::Right) => Some(Drag::Window {
                        start: self.cursor,
                        from: self.window,
                    }),
                    _ => None,
                };
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
                };
                if steps == 0.0 {
                    return;
                }
                if self.ctrl {
                    self.zoom(if steps > 0.0 { 1.1 } else { 1.0 / 1.1 });
                } else {
                    self.scroll_slices(if steps > 0.0 { -1 } else { 1 });
                }
            }
            _ => {}
        }
    }
}
