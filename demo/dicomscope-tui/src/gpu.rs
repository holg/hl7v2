//! Image plugins: whatever turns the session's current slice into pixels.
//!
//! The terminal only ever receives finished RGBA images; how they are made
//! is behind [`ImagePlugin`]. The plugin that ships is [`WgpuPlugin`]: the
//! same WGSL shader and renderer the browser and desktop apps use, running
//! headless on a real GPU backend (Metal on a Mac, Vulkan, DX12 or GL
//! elsewhere) and read back from an offscreen texture. Windowing, zoom,
//! rotation, flips, interpolation and contrast enhancement all happen on
//! the GPU, exactly as in the other apps.

use crate::cli::Backend;
use dicomscope_core::render::{Renderer, Uniforms};
use dicomscope_core::session::Session;
use dicomscope_core::AppError;
use image::RgbaImage;

/// Display choices that belong to the terminal app rather than the session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Look {
    /// Invert greyscale on top of what the file asks for.
    pub invert: bool,
}

/// Something that renders the session's slice at a given pixel size.
pub trait ImagePlugin {
    /// A one-line description for the status bar: backend and device.
    fn describe(&self) -> String;

    /// Render the slice on screen into a `size.0` x `size.1` RGBA image.
    /// Takes the session's pending frame, if any, so a newly decoded slice
    /// is uploaded once. The session's canvas must already be `size`.
    fn render(
        &mut self,
        session: &mut Session,
        size: (u32, u32),
        look: Look,
    ) -> Result<RgbaImage, AppError>;
}

/// The wgpu plugin: a headless [`Renderer`] on the chosen backend.
pub struct WgpuPlugin {
    renderer: Renderer,
}

impl WgpuPlugin {
    /// Open an adapter on `backend`. `auto` honours `WGPU_BACKEND` from
    /// the environment, as the desktop app does.
    pub fn new(backend: Backend) -> Result<WgpuPlugin, AppError> {
        let instance = instance(backend);
        let renderer = pollster::block_on(Renderer::headless(&instance, 1, 1))?;
        Ok(WgpuPlugin { renderer })
    }
}

impl ImagePlugin for WgpuPlugin {
    fn describe(&self) -> String {
        let info = self.renderer.adapter_info();
        format!("{:?} · {}", info.backend, info.name)
    }

    fn render(
        &mut self,
        session: &mut Session,
        size: (u32, u32),
        look: Look,
    ) -> Result<RgbaImage, AppError> {
        let (w, h) = (size.0.max(1), size.1.max(1));
        self.renderer.resize(w, h);
        if let Some(frame) = session.take_pending_frame() {
            self.renderer.upload(&frame);
        }
        if let Some(u) = uniforms(session, look) {
            self.renderer.set_uniforms(u);
        }
        let rgba = self.renderer.read_rgba()?;
        RgbaImage::from_raw(w, h, rgba)
            .ok_or_else(|| AppError::Gpu("read-back image has the wrong size".into()))
    }
}

/// The shader uniforms for the session's slice, with the image area's
/// top-left corner at the canvas origin. `None` before a slice is decoded.
pub fn uniforms(session: &Session, look: Look) -> Option<Uniforms> {
    let f = session.frame.as_ref()?;
    let v = session.view;
    let (center, width) = session.window;
    Some(Uniforms {
        center,
        width,
        invert: u32::from(f.inverted != look.invert),
        interp: u32::from(v.smooth),
        scale: v.scale,
        tx: v.tx,
        ty: v.ty,
        color: u32::from(f.color),
        rot: u32::from(v.rotation),
        flip: u32::from(v.flip_h) | (u32::from(v.flip_v) << 1),
        enhance: u32::from(session.enhance),
        radius: session.enhance_radius_px(),
        amount: session.enhance_amount,
        ..Uniforms::default()
    })
}

/// A wgpu instance limited to `backend`, with no window system attached.
pub fn instance(backend: Backend) -> wgpu::Instance {
    instance_from(
        backend,
        wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    )
}

/// A wgpu instance limited to `backend`, starting from `from_env`: a
/// descriptor that may carry the window system's display handle, which GL
/// on Wayland (EGL) needs.
pub fn instance_from(backend: Backend, from_env: wgpu::InstanceDescriptor) -> wgpu::Instance {
    let backends = match backend {
        Backend::Auto if std::env::var_os("WGPU_BACKEND").is_some() => from_env.backends,
        other => other.backends(),
    };
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..from_env
    })
}

/// Every adapter wgpu can open, per backend: (backend, name, device type,
/// driver).
pub fn adapters() -> Vec<wgpu::AdapterInfo> {
    let all = wgpu::Backends::all();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: all,
        ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
    });
    pollster::block_on(instance.enumerate_adapters(all))
        .iter()
        .map(wgpu::Adapter::get_info)
        .collect()
}
