//! The DICOM slice as a scopekit [`GpuView`]: the same WGSL shader and
//! renderer the browser and desktop apps use, on whatever device scopekit
//! gives it. In a terminal that is an offscreen texture sent as an image;
//! in a window it is the window's own surface. Windowing, zoom, rotation,
//! flips, interpolation and contrast enhancement all happen on the GPU.

use dicomscope_core::dicom::Frame;
use dicomscope_core::render::{Renderer, Uniforms};
use dicomscope_core::session::Session;
use scopekit::{wgpu, Gpu, GpuView, Target};

/// Display choices that belong to the viewer rather than the session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Look {
    /// Invert greyscale on top of what the file asks for.
    pub invert: bool,
}

/// The slice on screen, as a GPU view.
#[derive(Default)]
pub struct SliceView {
    renderer: Option<Renderer>,
    pending: Option<Frame>,
    uniforms: Option<Uniforms>,
    changed: bool,
}

impl SliceView {
    /// Take the session's newly decoded frame, if any, and its current
    /// window, view and look. The next render shows them.
    pub fn update(&mut self, session: &mut Session, look: Look) {
        if let Some(frame) = session.take_pending_frame() {
            self.pending = Some(frame);
        }
        self.uniforms = uniforms(session, look);
        self.changed = true;
    }
}

impl GpuView for SliceView {
    fn prepare(&mut self, gpu: &Gpu, format: wgpu::TextureFormat) {
        self.renderer = Some(Renderer::on_device(
            gpu.device.clone(),
            gpu.queue.clone(),
            format,
            1,
            1,
        ));
    }

    fn render(&mut self, _: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &Target<'_>) {
        let Some(r) = self.renderer.as_mut() else {
            return;
        };
        r.resize(target.size.0, target.size.1);
        if let Some(frame) = self.pending.take() {
            r.upload(&frame);
        }
        let reg = target.region;
        if let Some(mut u) = self.uniforms {
            // The shader works in whole-target coordinates.
            u.tx += reg.x as f32;
            u.ty += reg.y as f32;
            r.set_uniforms(u);
        }
        r.draw_image_over(encoder, target.view, (reg.x, reg.y, reg.width, reg.height));
        self.changed = false;
    }

    fn changed(&self) -> bool {
        self.changed
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

/// Every adapter wgpu can open, on every backend.
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
