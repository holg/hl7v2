//! The interactive viewer: state, the event loop and key handling. Drawing
//! is in [`crate::ui`]; the study, the order and everything derived from
//! them live in the shared [`Session`].

use crate::cli::{GpuArgs, ViewArgs};
use crate::gpu::{self, ImagePlugin, Look, WgpuPlugin};
use crate::window::SharedImage;
use crate::{term, ui};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use dicomscope_core::session::Session;
use image::DynamicImage;
use ratatui::layout::{Rect, Size};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol as Graphic;
use ratatui_image::Resize;
use std::time::{Duration, Instant};

/// The panels on the right, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The slice, rendered on the GPU.
    Image,
    /// The DICOM header.
    Tags,
    /// The HL7 order, fields highlighted.
    Order,
    /// Study against order.
    Link,
    /// The Modality Worklist item.
    Worklist,
    /// FHIR R4 JSON.
    Fhir,
    /// Reports and PDFs.
    Docs,
}

impl Tab {
    /// Every tab, in order.
    pub const ALL: [Tab; 7] = [
        Tab::Image,
        Tab::Tags,
        Tab::Order,
        Tab::Link,
        Tab::Worklist,
        Tab::Fhir,
        Tab::Docs,
    ];

    /// The tab bar label.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Image => "Image",
            Tab::Tags => "Tags",
            Tab::Order => "Order",
            Tab::Link => "Linkage",
            Tab::Worklist => "Worklist",
            Tab::Fhir => "FHIR",
            Tab::Docs => "Documents",
        }
    }

    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn step(self, delta: isize) -> Tab {
        let n = Tab::ALL.len() as isize;
        Tab::ALL[(self.index() as isize + delta).rem_euclid(n) as usize]
    }
}

/// Two presses on the same cell within this are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The FHIR resources the FHIR tab cycles through.
pub const FHIR_RESOURCES: [&str; 4] = ["Bundle", "Patient", "ServiceRequest", "ImagingStudy"];

/// Where the image goes.
pub enum Mode {
    /// Into a terminal, rendered by an [`ImagePlugin`] and sent with this
    /// picker's graphics protocol.
    Terminal(Picker),
    /// Into a window, drawn by the [`crate::window::Compositor`] on the
    /// window's own surface.
    Window(SharedImage),
}

/// The viewer state.
pub struct App {
    /// Study, order and everything derived.
    pub session: Session,
    /// The image renderer for terminal mode, or why none could be opened.
    pub plugin: Result<Box<dyn ImagePlugin>, String>,
    /// Terminal or window.
    pub mode: Mode,
    /// The active panel.
    pub tab: Tab,
    /// Invert and other terminal-side display choices.
    pub look: Look,
    /// Scroll offset per tab, in lines.
    pub scroll: [u16; 7],
    /// The Tags filter text.
    pub tag_filter: String,
    /// Typing into the tag filter.
    pub filtering: bool,
    /// Index into [`FHIR_RESOURCES`].
    pub fhir_resource: usize,
    /// Selected document in the Docs tab.
    pub doc_selected: usize,
    /// The key help is open.
    pub help: bool,
    /// A one-shot note for the status bar.
    pub note: Option<String>,
    /// The rendered image and the cell area it was made for.
    pub graphic: Option<(Rect, Graphic)>,
    /// Why the last render failed, if it did.
    pub render_error: Option<String>,
    /// Milliseconds the last GPU render and read-back took.
    pub render_ms: f32,
    /// Window mode: the image area in surface pixels, last time it was set.
    region: Option<(u32, u32, u32, u32)>,
    /// Terminal mode: the image panel in cells, last time it was fitted.
    image_cells: Option<Rect>,
    /// The last frame's layout, for mouse hits.
    areas: Option<ui::Areas>,
    /// Screen pixels per cell, for turning a drag into a pan.
    cell_px: (f32, f32),
    /// The button held since a press on the image, and where it last was.
    drag: Option<(MouseButton, u16, u16)>,
    /// The last press, for double clicks.
    last_click: Option<(Instant, u16, u16)>,
    dirty: bool,
    quit: bool,
    started: Instant,
}

impl App {
    /// Open the paths and the order, go to the start slice, and, in
    /// terminal mode, open the GPU.
    pub fn new(args: &ViewArgs, gpu: GpuArgs, mode: Mode) -> App {
        let mut session = Session::new();
        let paths: Vec<String> = args.paths.iter().map(|p| p.display().to_string()).collect();
        session.open_paths(&paths);
        if let Some(o) = &args.order {
            session.open_hl7(&o.display().to_string());
        }
        if session.set.is_some() && (args.series, args.slice) != (1, 1) {
            session.show_slice(
                args.series.saturating_sub(1),
                args.slice.saturating_sub(1),
                true,
            );
        }
        let plugin = match mode {
            Mode::Terminal(_) => WgpuPlugin::new(gpu.backend)
                .map(|p| Box::new(p) as Box<dyn ImagePlugin>)
                .map_err(|e| e.to_string()),
            Mode::Window(_) => Err("the window draws the image".into()),
        };
        let tab = if session.set.is_none() && session.hl7.is_some() {
            Tab::Order
        } else {
            Tab::Image
        };
        App {
            session,
            plugin,
            mode,
            tab,
            look: Look::default(),
            scroll: [0; 7],
            tag_filter: String::new(),
            filtering: false,
            fhir_resource: 0,
            doc_selected: 0,
            help: false,
            note: None,
            graphic: None,
            render_error: None,
            render_ms: 0.0,
            region: None,
            image_cells: None,
            areas: None,
            cell_px: (10.0, 20.0),
            drag: None,
            last_click: None,
            dirty: true,
            quit: false,
            started: Instant::now(),
        }
    }

    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Re-render the slice when anything changed or the image area moved.
    /// `area` is the image area in cells.
    fn refresh_image(&mut self, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Refit only when the panel changed size, not when the graphic was
        // merely dropped (help box, protocol switch, new series).
        let moved = self.image_cells != Some(area);
        if !self.dirty && !moved && self.graphic.is_some() {
            return;
        }
        let Mode::Terminal(picker) = &self.mode else {
            return;
        };
        self.dirty = false;
        let px = term::cell_pixels(picker, area.width, area.height);
        if moved {
            self.image_cells = Some(area);
            self.session.set_canvas(px);
            if let Some(img) = self.session.image() {
                self.session.view = self.session.view.fit(px, img);
            }
        }
        let Ok(plugin) = self.plugin.as_mut() else {
            return;
        };
        if self.session.frame.is_none() {
            self.graphic = None;
            return;
        }
        let t = Instant::now();
        match plugin.render(&mut self.session, px, self.look) {
            Ok(rgba) => {
                self.render_ms = t.elapsed().as_secs_f32() * 1000.0;
                match picker.new_protocol(
                    DynamicImage::ImageRgba8(rgba),
                    Size::new(area.width, area.height),
                    Resize::Fit(None),
                ) {
                    Ok(g) => {
                        self.graphic = Some((area, g));
                        self.render_error = None;
                    }
                    Err(e) => self.render_error = Some(e.to_string()),
                }
            }
            Err(e) => self.render_error = Some(e.to_string()),
        }
    }

    /// The event loop, until `q`.
    pub fn run(mut self, terminal: &mut ratatui::DefaultTerminal) -> std::io::Result<()> {
        while !self.quit {
            self.frame(terminal)?;
            let timeout = if self.session.playing {
                Duration::from_millis(u64::from(self.session.frame_ms().max(10.0) as u32) / 2)
            } else {
                Duration::from_millis(500)
            };
            if event::poll(timeout)? {
                self.handle(event::read()?);
            }
            self.tick();
        }
        Ok(())
    }

    /// Advance cine playback when its frame period has passed; `true` when
    /// the slice changed and the screen needs drawing.
    pub fn tick(&mut self) -> bool {
        if !self.session.playing {
            return false;
        }
        let before = self.session.current;
        self.session.cine_tick(self.now());
        let changed = self.session.current != before;
        self.dirty |= changed;
        changed
    }

    /// Render the image if needed and draw one frame, on any backend.
    pub fn frame<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
    ) -> std::io::Result<()> {
        let area = terminal
            .size()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let layout = ui::layout(Rect::new(0, 0, area.width, area.height));
        self.areas = Some(layout);
        let image_on_screen = self.tab == Tab::Image && self.session.document.is_none();
        match &self.mode {
            Mode::Terminal(picker) => {
                let font = picker.font_size();
                self.cell_px = (f32::from(font.width), f32::from(font.height));
                if image_on_screen && !self.help {
                    self.refresh_image(layout.image);
                }
            }
            Mode::Window(_) => {
                let size = terminal
                    .backend_mut()
                    .window_size()
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                self.cell_px = (
                    f32::from(size.pixels.width) / f32::from(size.columns_rows.width.max(1)),
                    f32::from(size.pixels.height) / f32::from(size.columns_rows.height.max(1)),
                );
                // The help box covers the image area; the image would be
                // drawn on top of it.
                let show = image_on_screen && !self.help;
                self.refresh_window(layout.image, size, show);
            }
        }
        terminal
            .draw(|f| ui::draw(f, self, &layout))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    }

    /// Window mode: tell the compositor where the image goes and what
    /// changed. The text grid is stretched over the whole surface, so cell
    /// column `x` starts at `x * surface_width / columns`.
    fn refresh_window(&mut self, area: Rect, size: ratatui::backend::WindowSize, show: bool) {
        let Mode::Window(shared) = &self.mode else {
            return;
        };
        let (cols, rows) = (
            u32::from(size.columns_rows.width.max(1)),
            u32::from(size.columns_rows.height.max(1)),
        );
        let (pw, ph) = (u32::from(size.pixels.width), u32::from(size.pixels.height));
        let x0 = u32::from(area.x) * pw / cols;
        let y0 = u32::from(area.y) * ph / rows;
        let x1 = u32::from(area.x + area.width) * pw / cols;
        let y1 = u32::from(area.y + area.height) * ph / rows;
        let region = (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0));
        let mut sh = shared.borrow_mut();
        if !show || region.2 == 0 || region.3 == 0 {
            if sh.region.take().is_some() {
                sh.dirty = true;
            }
            return;
        }
        let moved = self.region != Some(region);
        if moved {
            self.region = Some(region);
            let px = (region.2, region.3);
            self.session.set_canvas(px);
            if let Some(img) = self.session.image() {
                self.session.view = self.session.view.fit(px, img);
            }
        }
        if !self.dirty && !moved && sh.region.is_some() {
            return;
        }
        self.dirty = false;
        if let Some(frame) = self.session.take_pending_frame() {
            sh.pending = Some(frame);
        }
        sh.uniforms = gpu::uniforms(&self.session, self.look);
        sh.region = self.session.frame.as_ref().map(|_| region);
        sh.dirty = true;
    }

    /// Apply one terminal event.
    pub fn handle(&mut self, e: Event) {
        match e {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.key(k),
            Event::Mouse(m) => self.mouse(m),
            Event::Resize(..) => self.dirty = true,
            _ => {}
        }
    }

    /// `q` or `Ctrl-C` was pressed.
    pub fn quitting(&self) -> bool {
        self.quit
    }

    /// Open or close the key help. Terminal graphics are drawn above the
    /// text, so the image is dropped while help is up and sent again after.
    fn set_help(&mut self, on: bool) {
        self.help = on;
        self.graphic = None;
        self.dirty = true;
    }

    fn mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        if self.help {
            if matches!(m.kind, MouseEventKind::Down(_)) {
                self.set_help(false);
            }
            return;
        }
        let Some(a) = self.areas else {
            return;
        };
        let on_image =
            self.tab == Tab::Image && self.session.document.is_none() && ui::inside(a.image, x, y);
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d = if m.kind == MouseEventKind::ScrollDown {
                    1
                } else {
                    -1
                };
                if ui::inside(a.series, x, y) {
                    self.series(d);
                } else if on_image
                    // Option (Alt) for the Mac, where Ctrl + scroll is the
                    // system's screen zoom and never reaches the app.
                    && m.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    self.zoom_at(x, y, if d < 0 { 1.25 } else { 0.8 });
                } else if on_image {
                    self.slices(d);
                } else if ui::inside(a.body, x, y) {
                    self.scroll_by(d * 3);
                }
            }
            MouseEventKind::Down(button) => {
                let double = self
                    .last_click
                    .is_some_and(|(t, cx, cy)| (cx, cy) == (x, y) && t.elapsed() < DOUBLE_CLICK);
                self.last_click = Some((Instant::now(), x, y));
                if button == MouseButton::Left && ui::inside(a.tabs, x, y) {
                    if let Some(t) = ui::tab_at(a.tabs, x) {
                        self.set_tab(t);
                    }
                } else if button == MouseButton::Left && ui::inside(a.series, x, y) {
                    let n = self.session.set.as_ref().map_or(0, |s| s.series.len());
                    let cur = self.session.current.0;
                    if let Some(i) = ui::list_row_at(a.series, y, n, cur) {
                        self.series(i as i32 - cur as i32);
                    }
                } else if button == MouseButton::Left
                    && self.tab == Tab::Docs
                    && self.session.document.is_none()
                    && ui::inside(a.body, x, y)
                {
                    let n = self.session.set.as_ref().map_or(0, |s| s.documents.len());
                    if let Some(i) = ui::list_row_at(a.body, y, n, self.doc_selected) {
                        self.doc_selected = i;
                        if double {
                            self.session.open_document(i);
                            self.scroll[Tab::Docs.index()] = 0;
                        }
                    }
                } else if on_image {
                    if double && button == MouseButton::Left {
                        if let Some(img) = self.session.image() {
                            self.session.view = self.session.view.fit(self.session.canvas(), img);
                            self.dirty = true;
                        }
                    }
                    self.drag = Some((button, x, y));
                }
            }
            MouseEventKind::Drag(button) => {
                let Some((b, x0, y0)) = self.drag else {
                    return;
                };
                if b != button {
                    return;
                }
                let dx = f32::from(x) - f32::from(x0);
                let dy = f32::from(y) - f32::from(y0);
                self.drag = Some((b, x, y));
                match button {
                    // Pan: the image follows the pointer.
                    MouseButton::Left => {
                        self.session.view = self
                            .session
                            .view
                            .pan(dx * self.cell_px.0, dy * self.cell_px.1);
                    }
                    // Window: right for more width (less contrast), down for
                    // a higher centre (darker), as in most PACS viewers.
                    MouseButton::Right => {
                        let w = &mut self.session.window;
                        w.1 = (w.1 * (dx * 0.04).exp()).max(1.0);
                        w.0 += dy * w.1 * 0.02;
                    }
                    MouseButton::Middle => return,
                }
                self.dirty = true;
            }
            MouseEventKind::Up(_) => self.drag = None,
            _ => {}
        }
    }

    /// Zoom by `factor` about the image point under cell (`x`, `y`); no-op
    /// when that cell is not on the image. For wheels and pinch gestures.
    pub fn zoom_at(&mut self, x: u16, y: u16, factor: f32) {
        let Some(a) = self.areas else {
            return;
        };
        if self.tab != Tab::Image || self.help || !ui::inside(a.image, x, y) {
            return;
        }
        let (px, py) = self.canvas_point(a.image, x, y);
        self.session.view = self.session.view.zoom_about(factor, px, py);
        self.dirty = true;
    }

    /// Canvas pixel under cell (`x`, `y`), its centre, for zooming there.
    fn canvas_point(&self, image: Rect, x: u16, y: u16) -> (f32, f32) {
        (
            (f32::from(x - image.x) + 0.5) * self.cell_px.0,
            (f32::from(y - image.y) + 0.5) * self.cell_px.1,
        )
    }

    fn slices(&mut self, delta: i32) {
        self.session.scroll_slices(delta);
        self.dirty = true;
    }

    fn series(&mut self, delta: i32) {
        self.session.switch_series(delta);
        self.graphic = None;
        self.dirty = true;
    }

    fn scroll_by(&mut self, delta: i32) {
        let s = &mut self.scroll[self.tab.index()];
        *s = (i32::from(*s) + delta).clamp(0, i32::from(u16::MAX)) as u16;
    }

    fn key(&mut self, k: KeyEvent) {
        self.note = None;
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.filtering {
            match k.code {
                KeyCode::Enter => self.filtering = false,
                KeyCode::Esc => {
                    self.filtering = false;
                    self.tag_filter.clear();
                }
                KeyCode::Backspace => {
                    self.tag_filter.pop();
                }
                KeyCode::Char(c) => self.tag_filter.push(c),
                _ => {}
            }
            self.scroll[Tab::Tags.index()] = 0;
            return;
        }
        if self.help {
            // `?` again, Esc, or any other key: all of them only close it.
            return self.set_help(false);
        }
        match k.code {
            KeyCode::Char('q') => return self.quit = true,
            KeyCode::Char('?') => return self.set_help(true),
            KeyCode::Tab => return self.set_tab(self.tab.step(1)),
            KeyCode::BackTab => return self.set_tab(self.tab.step(-1)),
            KeyCode::F(n @ 1..=7) => return self.set_tab(Tab::ALL[usize::from(n) - 1]),
            KeyCode::Char('[') => return self.series(-1),
            KeyCode::Char(']') => return self.series(1),
            KeyCode::Char('p') => return self.next_protocol(),
            _ => {}
        }
        match self.tab {
            Tab::Image => self.image_key(k),
            Tab::Docs => self.docs_key(k),
            Tab::Tags if k.code == KeyCode::Char('/') => {
                self.filtering = true;
            }
            Tab::Fhir if matches!(k.code, KeyCode::Left | KeyCode::Right) => {
                let n = FHIR_RESOURCES.len();
                self.fhir_resource = if k.code == KeyCode::Left {
                    (self.fhir_resource + n - 1) % n
                } else {
                    (self.fhir_resource + 1) % n
                };
                self.scroll[Tab::Fhir.index()] = 0;
            }
            _ => self.text_key(k),
        }
    }

    fn set_tab(&mut self, t: Tab) {
        self.tab = t;
        self.dirty = true;
    }

    fn next_protocol(&mut self) {
        let Mode::Terminal(picker) = &mut self.mode else {
            self.note = Some("the window draws the image itself; no protocol to switch".into());
            return;
        };
        let next = match picker.protocol_type() {
            ProtocolType::Kitty => ProtocolType::Iterm2,
            ProtocolType::Iterm2 => ProtocolType::Sixel,
            ProtocolType::Sixel => ProtocolType::Halfblocks,
            ProtocolType::Halfblocks => ProtocolType::Kitty,
        };
        picker.set_protocol_type(next);
        self.graphic = None;
        self.dirty = true;
        self.note = Some(format!(
            "graphics: {} (if the image vanishes, this terminal does not speak it)",
            term::protocol_name(picker)
        ));
    }

    fn text_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(20),
            KeyCode::PageUp => self.scroll_by(-20),
            KeyCode::Home | KeyCode::Char('g') => self.scroll[self.tab.index()] = 0,
            KeyCode::End | KeyCode::Char('G') => self.scroll[self.tab.index()] = u16::MAX,
            _ => {}
        }
    }

    fn docs_key(&mut self, k: KeyEvent) {
        let n = self.session.set.as_ref().map_or(0, |s| s.documents.len());
        if self.session.document.is_some() {
            match k.code {
                KeyCode::Esc | KeyCode::Backspace => {
                    self.session.close_document();
                    self.scroll[Tab::Docs.index()] = 0;
                }
                _ => self.text_key(k),
            }
            return;
        }
        match k.code {
            KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                self.doc_selected = (self.doc_selected + 1).min(n - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.doc_selected = self.doc_selected.saturating_sub(1);
            }
            KeyCode::Enter if n > 0 => {
                self.session.open_document(self.doc_selected);
                self.scroll[Tab::Docs.index()] = 0;
            }
            _ => {}
        }
    }

    fn image_key(&mut self, k: KeyEvent) {
        let s = &mut self.session;
        let (cw, ch) = s.canvas();
        let (cx, cy) = (cw as f32 / 2.0, ch as f32 / 2.0);
        let step = cw.min(ch) as f32 * 0.08;
        let img = s.image();
        match k.code {
            KeyCode::Down | KeyCode::Char('j') => return self.slices(1),
            KeyCode::Up | KeyCode::Char('k') => return self.slices(-1),
            KeyCode::PageDown => return self.slices(10),
            KeyCode::PageUp => return self.slices(-10),
            KeyCode::Home => return self.slices(i32::MIN / 2),
            KeyCode::End => return self.slices(i32::MAX / 2),
            KeyCode::Right | KeyCode::Char('l') => return self.series(1),
            KeyCode::Left | KeyCode::Char('h') => return self.series(-1),
            KeyCode::Char('+') => s.view = s.view.zoom_about(1.25, cx, cy),
            KeyCode::Char('-') => s.view = s.view.zoom_about(0.8, cx, cy),
            KeyCode::Char('0') => {
                if let Some(img) = img {
                    s.view = s.view.fit((cw, ch), img);
                }
            }
            KeyCode::Char('=') => {
                if let Some(img) = img {
                    s.view = s.view.one_to_one((cw, ch), img);
                }
            }
            KeyCode::Char('H') => s.view = s.view.pan(step, 0.0),
            KeyCode::Char('L') => s.view = s.view.pan(-step, 0.0),
            KeyCode::Char('K') => s.view = s.view.pan(0.0, step),
            KeyCode::Char('J') => s.view = s.view.pan(0.0, -step),
            KeyCode::Char('r') => s.rotate(1),
            KeyCode::Char('R') => s.rotate(-1),
            KeyCode::Char('f') => s.view = s.view.flip_horizontal(),
            KeyCode::Char('v') => s.view = s.view.flip_vertical(),
            KeyCode::Char('w') => s.window.1 = (s.window.1 * 0.85).max(1.0),
            KeyCode::Char('W') => s.window.1 *= 1.0 / 0.85,
            KeyCode::Char('c') => s.window.0 -= s.window.1 * 0.05,
            KeyCode::Char('C') => s.window.0 += s.window.1 * 0.05,
            KeyCode::Char('a') => s.reset_window(),
            KeyCode::Char('i') => self.look.invert = !self.look.invert,
            KeyCode::Char('e') => s.enhance = !s.enhance,
            KeyCode::Char('s') => s.view.smooth = !s.view.smooth,
            KeyCode::Char(' ') => {
                let now = self.now();
                self.session.toggle_cine(now);
            }
            _ => return,
        }
        self.dirty = true;
    }
}

/// Run the viewer until the user quits, restoring the terminal on the way
/// out, panics included.
pub fn run(args: &ViewArgs, gpu: GpuArgs) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    // After entering the alternate screen, before reading events: the
    // query writes escape sequences and reads the answers from stdin.
    let picker = term::picker(args.protocol);
    let app = App::new(args, gpu, Mode::Terminal(picker));
    let result = app.run(&mut terminal);
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
