//! dicomscope on the desktop: a winit window, a native wgpu surface, egui
//! panels, and the same `dicomscope-core` code the browser runs. No WebGPU,
//! no webview, so it runs where those do not.
//!
//! The frame is drawn in two passes on one surface: the core renderer paints
//! the image into the central area, then egui paints the panels over it.

#![forbid(unsafe_code)]
#![warn(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod session;
mod ui;

use dicomscope_core::render::{Renderer, Uniforms};
use dicomscope_core::AppError;
use session::Session;
use std::sync::Arc;
use ui::{Actions, UiState};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

fn main() {
    let mut session = Session::default();
    let mut args = std::env::args().skip(1);
    // Optional: a study path, then an HL7 path.
    if let Some(study) = args.next() {
        session.open_study(&study);
        if let Some(e) = &session.error {
            eprintln!("{e}");
        }
    }
    if let Some(hl7) = args.next() {
        session.open_hl7(&hl7);
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
    let mut app = App {
        session,
        gpu: None,
        ui: UiState::default(),
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
                _pad0: 0,
                _pad1: 0,
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

        if output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero())
        {
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
        }
    }

    fn run_actions(&mut self, actions: &Actions) {
        let picked = if actions.open_study_folder {
            rfd::FileDialog::new()
                .set_title("Open a study folder")
                .pick_folder()
        } else if actions.open_study_file {
            rfd::FileDialog::new()
                .set_title("Open a zip or a DICOM file")
                .add_filter("DICOM or zip", &["dcm", "zip", "DCM", "ZIP"])
                .add_filter("All files", &["*"])
                .pick_file()
        } else {
            None
        };
        if let Some(p) = picked {
            self.session.open_study(&p.display().to_string());
            self.ui.thumb_textures.clear();
        }
        if actions.open_hl7 {
            if let Some(p) = rfd::FileDialog::new()
                .set_title("Open an HL7 v2 order")
                .add_filter("HL7", &["hl7", "txt", "HL7"])
                .add_filter("All files", &["*"])
                .pick_file()
            {
                self.session.open_hl7(&p.display().to_string());
            }
        }
        if actions.save_worklist {
            if let Some(Ok(out)) = &self.session.worklist {
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name("order.mwl.dcm")
                    .save_file()
                {
                    self.ui.status = Some(match std::fs::write(&p, &out.bytes) {
                        Ok(()) => format!("Wrote {} ({} bytes).", p.display(), out.bytes.len()),
                        Err(e) => format!("{}: {e}", p.display()),
                    });
                }
            }
        }
        if actions.save_fhir {
            if let Some(f) = &self.session.fhir {
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name("bundle.json")
                    .save_file()
                {
                    self.ui.status = Some(match std::fs::write(&p, f.bundle.as_bytes()) {
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
