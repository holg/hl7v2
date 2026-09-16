//! The window loop: winit events in, egui and the core renderer out.

use crate::platform;
use crate::session;
use crate::ui;
use dicomscope_core::render::{Renderer, Uniforms};
use dicomscope_core::AppError;
use session::Session;
use std::sync::Arc;
use ui::{Actions, UiState};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Run the application until its window closes. `paths` are opened first:
/// studies (folders, zips, DICOM files) and orders (`.hl7`, `.txt`) in any
/// order.
pub fn run(paths: Vec<String>) {
    let mut session = Session::new();
    if !paths.is_empty() {
        session.open_paths(&paths);
        if let Some(e) = &session.error {
            eprintln!("{e}");
        }
    }
    let event_loop = match EventLoop::new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot create an event loop: {e}");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let ui = UiState {
        status: platform::picker_note().map(str::to_string),
        ..UiState::default()
    };
    let mut app = App {
        session,
        gpu: None,
        ui,
        // Everything in the folder now is known; only later arrivals count.
        known: paths
            .into_iter()
            .chain(platform::folder_entries().into_iter().map(|e| e.path))
            .collect(),
        next_repaint: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop failed: {e}");
        std::process::exit(1);
    }
}

/// Everything that exists only while the window does.
struct Gpu {
    window: Arc<Window>,
    renderer: Renderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

struct App {
    session: Session,
    gpu: Option<Gpu>,
    ui: UiState,
    /// Paths already opened from the platform's document folders, so a
    /// rescan on foreground opens only what is new.
    known: std::collections::HashSet<String>,
    /// When egui asked to be repainted next (cine, animations).
    next_repaint: Option<std::time::Instant>,
}

impl App {
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<Gpu, String> {
        let attributes = Window::default_attributes()
            .with_title("dicomscope")
            .with_inner_size(winit::dpi::LogicalSize::new(1400.0, 900.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| format!("cannot create a window: {e}"))?,
        );
        // The display handle lets Vulkan and GL pick the right surface
        // extension on Wayland and X11. WGPU_BACKEND in the environment
        // overrides the backend choice, which is the Linux escape hatch.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
                event_loop.owned_display_handle(),
            ))
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("cannot create a surface for the window: {e}"))?;
        let size = window.inner_size();
        let renderer =
            pollster::block_on(Renderer::new(&instance, surface, size.width, size.height))
                .map_err(|e| match e {
                    AppError::NoWebGpu(e) => {
                        format!("no GPU adapter (Vulkan, Metal, DX12 or OpenGL): {e}")
                    }
                    e => e.to_string(),
                })?;

        let egui_ctx = egui::Context::default();
        // Touch screens are read at arm's length: larger text and targets.
        if cfg!(target_os = "ios") {
            egui_ctx.set_zoom_factor(1.2);
        }
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(renderer.device().limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            renderer.device(),
            renderer.format(),
            egui_wgpu::RendererOptions::default(),
        );
        Ok(Gpu {
            window,
            renderer,
            egui_ctx,
            egui_state,
            egui_renderer,
        })
    }

    fn frame(&mut self) {
        let Some(gpu) = &mut self.gpu else { return };
        let raw_input = gpu.egui_state.take_egui_input(&gpu.window);
        let mut actions = Actions::default();
        let output = gpu.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.session, &mut self.ui);
        });
        gpu.egui_state
            .handle_platform_output(&gpu.window, output.platform_output);

        // A newly decoded slice goes to the GPU before this frame is drawn.
        if let Some(frame) = self.session.take_pending_frame() {
            gpu.renderer.upload(&frame);
        }

        let (w, h) = gpu.renderer.size();
        let ppp = output.pixels_per_point;
        let primitives = gpu.egui_ctx.tessellate(output.shapes, ppp);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point: ppp,
        };
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                gpu.egui_renderer.update_texture(
                    gpu.renderer.device(),
                    gpu.renderer.queue(),
                    *id,
                    delta,
                );
            }
        }
        let Some(target) = gpu.renderer.acquire() else {
            gpu.window.request_redraw();
            return;
        };
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            gpu.renderer
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("frame"),
                });
        gpu.egui_renderer.update_buffers(
            gpu.renderer.device(),
            gpu.renderer.queue(),
            &mut encoder,
            &primitives,
            &screen,
        );

        // Pass 1: the image, inside the central area. The view geometry is
        // relative to that area, the uniforms to the whole surface.
        if let (Some((x, y, _, _)), Some(f)) = (actions.image_rect, &self.session.frame) {
            let v = self.session.view;
            let (c, wdt) = self.session.window;
            gpu.renderer.set_uniforms(Uniforms {
                center: c,
                width: wdt,
                invert: u32::from(f.inverted),
                interp: u32::from(v.smooth),
                scale: v.scale,
                tx: v.tx + x as f32,
                ty: v.ty + y as f32,
                color: u32::from(f.color),
                rot: u32::from(v.rotation),
                flip: u32::from(v.flip_h) | (u32::from(v.flip_v) << 1),
                enhance: u32::from(self.session.enhance),
                radius: self.session.enhance_radius_px(),
                amount: self.session.enhance_amount,
                ..Uniforms::default()
            });
        }
        gpu.renderer
            .draw_image(&mut encoder, &view, actions.image_rect);

        // Pass 2: egui on top, keeping what pass 1 drew.
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            gpu.egui_renderer.render(&mut pass, &primitives, &screen);
        }
        gpu.renderer.present(encoder, target);
        for id in &output.textures_delta.free {
            gpu.egui_renderer.free_texture(id);
        }

        // File dialogs after the frame, so the UI that asked is on screen.
        self.run_actions(&actions);
        let picked = platform::take_picked();
        if !picked.is_empty() {
            self.session.open_paths(&picked);
            self.ui.thumb_textures.clear();
            self.known.extend(picked);
        }

        // egui says when it wants the next frame: now (an animation), after
        // a delay (cine's next step), or never until an event.
        let delay = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay);
        match delay {
            Some(d) if d.is_zero() => {
                self.next_repaint = None;
                if let Some(gpu) = &self.gpu {
                    gpu.window.request_redraw();
                }
            }
            Some(d) if d < std::time::Duration::from_secs(3600) => {
                self.next_repaint = Some(std::time::Instant::now() + d);
            }
            _ => self.next_repaint = None,
        }
    }

    fn run_actions(&mut self, actions: &Actions) {
        let picked = if actions.open_study_folder {
            platform::pick_folder("Open a study folder")
        } else if actions.open_study_file {
            platform::pick_file(
                "Open a zip or a DICOM file",
                "DICOM or zip",
                &["dcm", "zip", "DCM", "ZIP"],
            )
        } else {
            None
        };
        if let Some(p) = &actions.open_entry {
            self.session.open_paths(std::slice::from_ref(p));
            self.ui.thumb_textures.clear();
            self.known.insert(p.clone());
        }
        if actions.reload {
            let paths = platform::initial_paths();
            if paths.is_empty() {
                self.ui.status = Some("The dicomscope folder is empty.".into());
            } else {
                self.session.open_paths(&paths);
                self.ui.thumb_textures.clear();
                self.known.extend(paths);
            }
        }
        if let Some(p) = picked {
            self.session.open_study(&p.display().to_string());
            self.ui.thumb_textures.clear();
        }
        if actions.open_hl7 {
            if let Some(p) =
                platform::pick_file("Open an HL7 v2 order", "HL7", &["hl7", "txt", "HL7"])
            {
                self.session.open_hl7(&p.display().to_string());
            }
        }
        if actions.save_worklist {
            if let Some(Ok(out)) = &self.session.worklist {
                if let Some(p) = platform::save_target("order.mwl.dcm") {
                    self.ui.status = Some(match std::fs::write(&p, &out.bytes) {
                        Ok(()) => format!("Wrote {} ({} bytes).", p.display(), out.bytes.len()),
                        Err(e) => format!("{}: {e}", p.display()),
                    });
                }
            }
        }
        if actions.save_fhir {
            if let Some(f) = &self.session.fhir {
                if let Some(p) = platform::save_target("bundle.json") {
                    self.ui.status = Some(match std::fs::write(&p, f.bundle.as_bytes()) {
                        Ok(()) => format!("Wrote {}.", p.display()),
                        Err(e) => format!("{}: {e}", p.display()),
                    });
                }
            }
        }
        if actions.open_pdf || actions.save_pdf {
            if let Some((_, session::DocumentContent::Pdf { title, bytes, .. })) =
                &self.session.document
            {
                let file_name = format!(
                    "{}.pdf",
                    title
                        .chars()
                        .map(|c| if c.is_alphanumeric() { c } else { '_' })
                        .collect::<String>()
                );
                let target = if actions.save_pdf {
                    platform::save_target(&file_name)
                } else {
                    Some(std::env::temp_dir().join(file_name))
                };
                if let Some(p) = target {
                    self.ui.status = Some(match std::fs::write(&p, bytes) {
                        Ok(()) if actions.open_pdf => match platform::open_external(&p) {
                            Ok(()) => format!("Opened {} in the system viewer.", p.display()),
                            Err(e) => format!("{}: {e}", p.display()),
                        },
                        Ok(()) => format!("Wrote {}.", p.display()),
                        Err(e) => format!("{}: {e}", p.display()),
                    });
                }
            }
        }
        if let Some(gpu) = &self.gpu {
            gpu.window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            // Back in the foreground (iOS): "Open in dicomscope" from another
            // app has put a copy into Documents/Inbox by now.
            let fresh: Vec<String> = platform::inbox_paths()
                .into_iter()
                .filter(|p| !self.known.contains(p))
                .collect();
            if !fresh.is_empty() {
                self.session.open_paths(&fresh);
                self.ui.thumb_textures.clear();
                self.known.extend(fresh);
            }
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
            return;
        }
        match self.create(event_loop) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(e) => {
                eprintln!("{e}");
                event_loop.exit();
            }
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
        if matches!(cause, winit::event::StartCause::ResumeTimeReached { .. }) {
            self.next_repaint = None;
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Sleep until egui's next requested frame. On iOS also wake up
        // regularly: the document picker hands its result to a delegate
        // outside our events.
        if cfg!(target_os = "ios") && platform::has_picked() {
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
        }
        let poll = if cfg!(target_os = "ios") {
            Some(std::time::Instant::now() + std::time::Duration::from_millis(300))
        } else {
            None
        };
        let next = match (self.next_repaint, poll) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        event_loop.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gpu) = &mut self.gpu else { return };
        let response = gpu.egui_state.on_window_event(&gpu.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.renderer.resize(size.width, size.height);
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.frame(),
            _ => {
                if response.repaint {
                    gpu.window.request_redraw();
                }
            }
        }
    }
}
