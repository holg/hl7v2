//! wgpu device setup, one texture per loaded file, one uniform for the window.

use crate::dicom::{Frame, Pixels};
use crate::error::AppError;
use wgpu::util::DeviceExt;

/// Everything the shader needs besides the pixels. Matches `Uniforms` in
/// `shader.wgsl`; 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub center: f32,
    pub width: f32,
    pub invert: u32,
    pub interp: u32,
    /// Canvas pixels per image pixel.
    pub scale: f32,
    /// Canvas position of the image's top-left corner.
    pub tx: f32,
    pub ty: f32,
    /// 1 for RGBA colour textures, shown as stored.
    pub color: u32,
    /// Quarter turns clockwise, 0 to 3.
    pub rot: u32,
    /// Bit 1 mirrors horizontally, bit 2 vertically (source space).
    pub flip: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

impl Default for Uniforms {
    fn default() -> Self {
        Uniforms {
            center: 0.0,
            width: 1.0,
            invert: 0,
            interp: 1,
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
            color: 0,
            rot: 0,
            flip: 0,
            _pad0: 0,
            _pad1: 0,
        }
    }
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    uniforms: Uniforms,
}

impl Renderer {
    /// Acquire an adapter and device for the canvas. Fails with
    /// `AppError::NoWebGpu` when the browser has no WebGPU adapter.
    pub async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Renderer, AppError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|e| AppError::Gpu(format!("cannot create a surface for the canvas: {e}")))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| AppError::NoWebGpu(e.to_string()))?;
        // No optional features: R32Float with textureLoad needs none, and
        // requiring any would exclude adapters for no benefit.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("dicomscope"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                ..Default::default()
            })
            .await
            .map_err(|e| AppError::Gpu(format!("device request failed: {e}")))?;

        let mut config = surface
            .get_default_config(&adapter, canvas.width().max(1), canvas.height().max(1))
            .ok_or_else(|| AppError::Gpu("surface has no supported texture format".into()))?;
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("window"),
            source: wgpu::ShaderSource::Wgsl(super::SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("window"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("window"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = Uniforms::default();
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Ok(Renderer {
            device,
            queue,
            surface,
            config,
            pipeline,
            layout,
            uniform,
            bind_group: None,
            uniforms,
        })
    }

    /// Match the surface to a new canvas size (device pixels).
    pub fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        if (w, h) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
    }

    /// Upload a frame as an `R32Float` (greyscale) or `Rgba8Unorm` (colour)
    /// texture. Both bind as `texture_2d<f32>`. Called once per loaded file;
    /// the frame is dropped by the caller.
    pub fn upload(&mut self, frame: &Frame) {
        let (w, h) = (frame.width.max(1), frame.height.max(1));

        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: match &frame.pixels {
                Pixels::Gray(_) => wgpu::TextureFormat::R32Float,
                Pixels::Rgba(_) => wgpu::TextureFormat::Rgba8Unorm,
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Rows are padded to COPY_BYTES_PER_ROW_ALIGNMENT (256 bytes). A
        // 64-pixel-wide image is exactly 256 bytes per row; anything else is
        // padded. Doing this unconditionally keeps one code path.
        let (bytes, bytes_per_row) = match &frame.pixels {
            Pixels::Gray(d) => padded_rows(bytemuck::cast_slice(d), w as usize * 4, h as usize),
            Pixels::Rgba(d) => padded_rows(d, w as usize * 4, h as usize),
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        }));
    }

    /// Rewrite the uniform buffer. Cheap enough for every slider, wheel or
    /// mouse-move event.
    pub fn set_uniforms(&mut self, uniforms: Uniforms) {
        if uniforms == self.uniforms {
            return;
        }
        self.uniforms = uniforms;
        self.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&self.uniforms));
    }

    /// Draw the current frame with the current window. No-op before upload.
    pub fn draw(&mut self) -> Result<(), AppError> {
        let Some(bind_group) = &self.bind_group else {
            return Ok(());
        };
        let target = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            _ => return Ok(()),
        };
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("draw"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("window"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(target);
        Ok(())
    }
}

/// Lay out sample bytes row by row with each row padded to a multiple of
/// `COPY_BYTES_PER_ROW_ALIGNMENT`. Returns the bytes and the padded row stride.
fn padded_rows(data: &[u8], row_bytes: usize, height: usize) -> (Vec<u8>, u32) {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = row_bytes.div_ceil(align) * align;
    let mut out = vec![0u8; padded * height];
    for (row, src) in data.chunks(row_bytes).take(height).enumerate() {
        let start = row * padded;
        out[start..start + src.len()].copy_from_slice(src);
    }
    (out, padded as u32)
}
