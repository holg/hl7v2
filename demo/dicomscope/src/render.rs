//! The browser's half of rendering: a WebGPU surface on the canvas. The
//! renderer itself lives in `dicomscope-core` and is shared with the desktop.

pub use dicomscope_core::render::{Renderer, Uniforms};
use dicomscope_core::AppError;

/// Acquire a WebGPU adapter and device for the canvas.
pub async fn for_canvas(canvas: web_sys::HtmlCanvasElement) -> Result<Renderer, AppError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
        .map_err(|e| AppError::Gpu(format!("cannot create a surface for the canvas: {e}")))?;
    Renderer::new(&instance, surface, canvas.width(), canvas.height()).await
}
