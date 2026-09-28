//! Window mode (`--window`): the same viewer, drawn by [ratatui-wgpu] into
//! a native window instead of a terminal.
//!
//! ratatui-wgpu renders the text grid into a texture and hands it to a
//! [`PostProcessor`] that puts it on the window's surface. [`Compositor`] is
//! that post-processor: it blits the text, then draws the DICOM slice
//! straight into the image panel's rectangle with the core renderer, on the
//! same device, in the same frame. No read-back and no terminal graphics
//! protocol, so the image is at the window's full resolution, on whatever
//! backend wgpu picked (Metal on a Mac).
//!
//! [ratatui-wgpu]: https://docs.rs/ratatui-wgpu

use crate::app::{App, Mode};
use crate::cli::{GpuArgs, ViewArgs};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use dicomscope_core::dicom::Frame;
use dicomscope_core::render::{Renderer, Uniforms};
use ratatui::backend::Backend as _;
use ratatui::style::Color;
use ratatui::Terminal;
use ratatui_wgpu::shaders::DefaultPostProcessor;
use ratatui_wgpu::wgpu::{
    CommandEncoder, Device, Queue, SurfaceConfiguration, TextureFormat, TextureView,
};
use ratatui_wgpu::{Builder, ColorTable, Dimensions, Font, PostProcessor, WgpuBackend};
use std::cell::RefCell;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton as WinitButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

/// The font that ships in the binary: Cascadia Mono, SIL Open Font
/// License 1.1 (`fonts/CascadiaOFL.txt`).
const FONT: &[u8] = include_bytes!("../fonts/CascadiaMono-Regular.ttf");

/// Text size in logical pixels; multiplied by the display's scale factor.
const FONT_SIZE: f64 = 15.0;

/// A dark terminal palette. ratatui-wgpu's default uses the CSS colour
/// names, where "DarkGray" (169) is lighter than "Gray" (128): selection
/// bars and borders came out bright.
const PALETTE: ColorTable = ColorTable {
    BLACK: [0x12, 0x12, 0x14],
    RED: [0xe0, 0x5a, 0x5a],
    GREEN: [0x6c, 0xc6, 0x6c],
    YELLOW: [0xe5, 0xc0, 0x5c],
    BLUE: [0x5c, 0x8f, 0xe5],
    MAGENTA: [0xc6, 0x78, 0xdd],
    CYAN: [0x56, 0xc8, 0xd8],
    GRAY: [0xb4, 0xb4, 0xb4],
    DARKGRAY: [0x4a, 0x4a, 0x50],
    LIGHTRED: [0xff, 0x7b, 0x7b],
    LIGHTGREEN: [0x8e, 0xe0, 0x8e],
    LIGHTYELLOW: [0xff, 0xdc, 0x7c],
    LIGHTBLUE: [0x82, 0xaa, 0xff],
    LIGHTMAGENTA: [0xe0, 0x9a, 0xf0],
    LIGHTCYAN: [0x7c, 0xe0, 0xee],
    WHITE: [0xf0, 0xf0, 0xf0],
};

/// What the viewer tells the compositor, once per change.
#[derive(Default)]
pub struct Shared {
    /// A newly decoded slice to upload.
    pub pending: Option<Frame>,
    /// Uniforms with `tx`/`ty` relative to the image area.
    pub uniforms: Option<Uniforms>,
    /// The image area in surface pixels; `None` hides the image.
    pub region: Option<(u32, u32, u32, u32)>,
    /// Something changed: draw even if the text did not.
    pub dirty: bool,
    /// The adapter, for the status bar; filled in by the compositor.
    pub adapter: String,
}

/// The shared state's handle, owned by both the viewer and the compositor.
pub type SharedImage = Rc<RefCell<Shared>>;

/// ratatui-wgpu's post-processor: text first, then the slice on top.
pub struct Compositor {
    text: DefaultPostProcessor,
    device: Device,
    format: TextureFormat,
    renderer: Option<Renderer>,
    shared: SharedImage,
}

impl PostProcessor for Compositor {
    type UserData = SharedImage;

    fn compile(
        device: &Device,
        text_view: &TextureView,
        surface_config: &SurfaceConfiguration,
        shared: SharedImage,
    ) -> Self {
        let info = device.adapter_info();
        shared.borrow_mut().adapter = format!("{:?} · {}", info.backend, info.name);
        Compositor {
            text: DefaultPostProcessor::compile(device, text_view, surface_config, ()),
            device: device.clone(),
            format: surface_config.format,
            renderer: None,
            shared,
        }
    }

    fn resize(
        &mut self,
        device: &Device,
        text_view: &TextureView,
        surface_config: &SurfaceConfiguration,
    ) {
        self.text.resize(device, text_view, surface_config);
        if let Some(r) = &mut self.renderer {
            r.resize(surface_config.width, surface_config.height);
        }
        self.shared.borrow_mut().dirty = true;
    }

    fn process(
        &mut self,
        encoder: &mut CommandEncoder,
        queue: &Queue,
        text_view: &TextureView,
        surface_config: &SurfaceConfiguration,
        surface_view: &TextureView,
    ) {
        self.text
            .process(encoder, queue, text_view, surface_config, surface_view);
        // The queue arrives only here, so the renderer is made on the first
        // frame, on ratatui-wgpu's own device.
        let renderer = self.renderer.get_or_insert_with(|| {
            Renderer::on_device(
                self.device.clone(),
                queue.clone(),
                self.format,
                surface_config.width,
                surface_config.height,
            )
        });
        renderer.resize(surface_config.width, surface_config.height);
        let mut shared = self.shared.borrow_mut();
        shared.dirty = false;
        if let Some(frame) = shared.pending.take() {
            renderer.upload(&frame);
        }
        if let (Some(region), Some(mut u)) = (shared.region, shared.uniforms) {
            // The shader works in whole-surface coordinates.
            u.tx += region.0 as f32;
            u.ty += region.1 as f32;
            renderer.set_uniforms(u);
            renderer.draw_image_over(encoder, surface_view, region);
        }
    }

    fn needs_update(&self) -> bool {
        self.shared.borrow().dirty
    }
}

type WindowTerminal = Terminal<WgpuBackend<'static, 'static, Compositor>>;

struct Handler {
    args: ViewArgs,
    gpu: GpuArgs,
    font: &'static [u8],
    shared: SharedImage,
    app: Option<App>,
    window: Option<Arc<Window>>,
    terminal: Option<WindowTerminal>,
    modifiers: ModifiersState,
    /// Pointer position in surface pixels.
    cursor: (f64, f64),
    /// The mouse button held down, for drags.
    held: Option<MouseButton>,
    /// Wheel pixels not yet turned into a step.
    wheel: f64,
    error: Option<String>,
}

impl Handler {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let attrs = Window::default_attributes()
            .with_title("dicomscope")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("cannot open a window: {e}"))?,
        );
        let size = window.inner_size();
        let font = Font::new(self.font).ok_or("the font file is not a TrueType/OpenType font")?;
        let px = (FONT_SIZE * window.scale_factor()).round().max(8.0) as u32;
        let backend = pollster::block_on(
            Builder::<Compositor>::from_font_and_user_data(font, self.shared.clone())
                // With the display handle, as the desktop app does: GL on
                // Wayland (EGL) cannot open a surface without it.
                .with_instance(crate::gpu::instance_from(
                    self.gpu.backend,
                    wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
                        event_loop.owned_display_handle(),
                    )),
                ))
                .with_font_size_px(px)
                .with_color_table(PALETTE)
                .with_fg_color(Color::Rgb(0xd8, 0xd8, 0xd8))
                .with_bg_color(Color::Rgb(0x12, 0x12, 0x14))
                .with_width_and_height(Dimensions {
                    width: NonZeroU32::new(size.width.max(1)).unwrap_or(NonZeroU32::MIN),
                    height: NonZeroU32::new(size.height.max(1)).unwrap_or(NonZeroU32::MIN),
                })
                .build_with_target(window.clone()),
        )
        .map_err(|e| format!("no GPU for the window: {e}"))?;
        let terminal = Terminal::new(backend).map_err(|e| e.to_string())?;
        self.app = Some(App::new(
            &self.args,
            self.gpu,
            Mode::Window(self.shared.clone()),
        ));
        self.terminal = Some(terminal);
        window.request_redraw();
        self.window = Some(window);
        Ok(())
    }

    fn redraw(&mut self) {
        let (Some(app), Some(t)) = (self.app.as_mut(), self.terminal.as_mut()) else {
            return;
        };
        if let Err(e) = app.frame(t) {
            self.error = Some(e.to_string());
        }
    }
}

impl ApplicationHandler for Handler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(e) = self.start(event_loop) {
            self.error = Some(e);
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let (Some(app), Some(window)) = (self.app.as_mut(), self.window.clone()) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(t) = self.terminal.as_mut() {
                    t.backend_mut().resize(size.width, size.height);
                    // The backend starts a blank text texture on every
                    // resize, but ratatui only sends cells that changed
                    // unless the grid size did too: force a full redraw.
                    let _ = t.clear();
                }
                app.handle(Event::Resize(0, 0));
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(k) = key_event(&event.logical_key, self.modifiers) {
                    app.handle(Event::Key(k));
                    if app.quitting() {
                        event_loop.exit();
                        return;
                    }
                    window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Trackpads send many small pixel deltas: one step per
                // WHEEL_STEP pixels, the rest carried over.
                self.wheel += match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y) * WHEEL_STEP,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                let steps = (self.wheel / WHEEL_STEP).trunc();
                self.wheel -= steps * WHEEL_STEP;
                let (column, row) = cell_at(self.terminal.as_mut(), self.cursor);
                let kind = if steps > 0.0 {
                    MouseEventKind::ScrollUp
                } else {
                    MouseEventKind::ScrollDown
                };
                for _ in 0..steps.abs() as u32 {
                    app.handle(Event::Mouse(MouseEvent {
                        kind,
                        column,
                        row,
                        modifiers: mouse_modifiers(self.modifiers),
                    }));
                }
                if steps != 0.0 {
                    window.request_redraw();
                }
            }
            // Trackpad pinch: zoom about the pointer, as in Preview.
            WindowEvent::PinchGesture { delta, .. } => {
                let (column, row) = cell_at(self.terminal.as_mut(), self.cursor);
                app.zoom_at(column, row, (1.0 + delta as f32).clamp(0.5, 2.0));
                window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let before = cell_at(self.terminal.as_mut(), self.cursor);
                self.cursor = (position.x, position.y);
                let (column, row) = cell_at(self.terminal.as_mut(), self.cursor);
                if let (Some(b), true) = (self.held, (column, row) != before) {
                    app.handle(Event::Mouse(MouseEvent {
                        kind: MouseEventKind::Drag(b),
                        column,
                        row,
                        modifiers: mouse_modifiers(self.modifiers),
                    }));
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    WinitButton::Left => MouseButton::Left,
                    WinitButton::Right => MouseButton::Right,
                    WinitButton::Middle => MouseButton::Middle,
                    _ => return,
                };
                let (column, row) = cell_at(self.terminal.as_mut(), self.cursor);
                let kind = match state {
                    ElementState::Pressed => {
                        self.held = Some(button);
                        MouseEventKind::Down(button)
                    }
                    ElementState::Released => {
                        self.held = None;
                        MouseEventKind::Up(button)
                    }
                };
                app.handle(Event::Mouse(MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers: mouse_modifiers(self.modifiers),
                }));
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        if app.session.playing {
            if app.tick() {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            let step = Duration::from_secs_f32(app.session.frame_ms().max(10.0) / 2000.0);
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + step));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// Wheel pixels per step: one slice, or three lines of text.
const WHEEL_STEP: f64 = 24.0;

/// The text cell under a surface pixel. The grid is stretched over the
/// whole surface, so the mapping is a plain proportion.
fn cell_at(terminal: Option<&mut WindowTerminal>, (x, y): (f64, f64)) -> (u16, u16) {
    let Some(size) = terminal.and_then(|t| t.backend_mut().window_size().ok()) else {
        return (0, 0);
    };
    let cell = |p: f64, px: u16, cells: u16| {
        let v = p * f64::from(cells) / f64::from(px.max(1));
        (v.max(0.0) as u16).min(cells.saturating_sub(1))
    };
    (
        cell(x, size.pixels.width, size.columns_rows.width),
        cell(y, size.pixels.height, size.columns_rows.height),
    )
}

/// Modifiers as the viewer sees them on a mouse event. Cmd counts as Alt:
/// on a Mac, Cmd or Option + wheel zooms (Ctrl + wheel is the system's
/// screen zoom).
fn mouse_modifiers(m: ModifiersState) -> KeyModifiers {
    let mut k = KeyModifiers::NONE;
    if m.control_key() {
        k |= KeyModifiers::CONTROL;
    }
    if m.alt_key() || m.super_key() {
        k |= KeyModifiers::ALT;
    }
    k
}

/// A winit key as the crossterm key the viewer understands.
fn key_event(key: &Key, mods: ModifiersState) -> Option<KeyEvent> {
    let code = match key {
        Key::Named(n) => match n {
            NamedKey::ArrowUp => KeyCode::Up,
            NamedKey::ArrowDown => KeyCode::Down,
            NamedKey::ArrowLeft => KeyCode::Left,
            NamedKey::ArrowRight => KeyCode::Right,
            NamedKey::PageUp => KeyCode::PageUp,
            NamedKey::PageDown => KeyCode::PageDown,
            NamedKey::Home => KeyCode::Home,
            NamedKey::End => KeyCode::End,
            NamedKey::Enter => KeyCode::Enter,
            NamedKey::Escape => KeyCode::Esc,
            NamedKey::Backspace => KeyCode::Backspace,
            NamedKey::Space => KeyCode::Char(' '),
            NamedKey::Tab if mods.shift_key() => KeyCode::BackTab,
            NamedKey::Tab => KeyCode::Tab,
            NamedKey::F1 => KeyCode::F(1),
            NamedKey::F2 => KeyCode::F(2),
            NamedKey::F3 => KeyCode::F(3),
            NamedKey::F4 => KeyCode::F(4),
            NamedKey::F5 => KeyCode::F(5),
            NamedKey::F6 => KeyCode::F(6),
            NamedKey::F7 => KeyCode::F(7),
            _ => return None,
        },
        Key::Character(s) => KeyCode::Char(s.chars().next()?),
        _ => return None,
    };
    let mut modifiers = KeyModifiers::NONE;
    if mods.control_key() {
        modifiers |= KeyModifiers::CONTROL;
    }
    // Cmd-Q and Cmd-W close the window, as any Mac app does.
    if mods.super_key() && matches!(code, KeyCode::Char('q' | 'w')) {
        return Some(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    }
    Some(KeyEvent::new(code, modifiers))
}

/// Open the viewer in a window and run until it is closed.
pub fn run(args: &ViewArgs, gpu: GpuArgs, font: Option<&PathBuf>) -> Result<(), String> {
    let font: &'static [u8] = match font {
        None => FONT,
        Some(p) => Box::leak(
            std::fs::read(p)
                .map_err(|e| format!("{}: {e}", p.display()))?
                .into_boxed_slice(),
        ),
    };
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut handler = Handler {
        args: args.clone(),
        gpu,
        font,
        shared: SharedImage::default(),
        app: None,
        window: None,
        terminal: None,
        modifiers: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        held: None,
        wheel: 0.0,
        error: None,
    };
    event_loop
        .run_app(&mut handler)
        .map_err(|e| e.to_string())?;
    match handler.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
