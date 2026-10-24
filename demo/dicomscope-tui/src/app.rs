//! The interactive viewer: state and input. Drawing is in [`crate::ui`];
//! the study, the order and everything derived from them live in the
//! shared [`Session`]. Terminal and window both come from scopekit: this
//! file only says what to draw and where the slice goes.

use crate::cli::{GpuArgs, Protocol, ViewArgs};
use crate::gpu::{Look, SliceView};
use crate::ui;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use dicomscope_core::session::Session;
use ratatui::layout::Rect;
use ratatui::Frame;
use scopekit::{Flow, ViewSlot, Views};
use std::cell::RefCell;
use std::rc::Rc;
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

/// The name the slice's GPU view is registered and placed under.
pub const SLICE: &str = "slice";

/// Two presses on the same cell within this are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The FHIR resources the FHIR tab cycles through.
pub const FHIR_RESOURCES: [&str; 4] = ["Bundle", "Patient", "ServiceRequest", "ImagingStudy"];

/// The viewer state.
pub struct App {
    /// Study, order and everything derived.
    pub session: Session,
    /// The slice on the GPU, shared with scopekit.
    pub slice: Rc<RefCell<SliceView>>,
    /// The active panel.
    pub tab: Tab,
    /// Invert and other viewer-side display choices.
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
    /// GPU and graphics protocol, as scopekit reports them.
    pub describe: String,
    /// The image panel in cells, last time the view was fitted to it.
    image_cells: Option<Rect>,
    /// The last frame's layout, for mouse hits.
    areas: Option<ui::Areas>,
    /// Screen pixels per cell, for turning a drag into a pan.
    cell_px: (f32, f32),
    /// The button held since a press on the image, and where it last was.
    drag: Option<(MouseButton, u16, u16)>,
    /// The last press, for double clicks.
    last_click: Option<(Instant, u16, u16)>,
    /// The slice view needs the session's latest state.
    dirty: bool,
    quit: bool,
    started: Instant,
}

impl App {
    /// Open the paths and the order and go to the start slice. Returns the
    /// app and its GPU view, registered as [`SLICE`], to hand to scopekit.
    pub fn new(args: &ViewArgs) -> (App, Views) {
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
        let tab = if session.set.is_none() && session.hl7.is_some() {
            Tab::Order
        } else {
            Tab::Image
        };
        let (slice, view) = scopekit::share(SliceView::default());
        let app = App {
            session,
            slice,
            tab,
            look: Look::default(),
            scroll: [0; 7],
            tag_filter: String::new(),
            filtering: false,
            fhir_resource: 0,
            doc_selected: 0,
            help: false,
            note: None,
            describe: String::new(),
            image_cells: None,
            areas: None,
            cell_px: (10.0, 20.0),
            drag: None,
            last_click: None,
            dirty: true,
            quit: false,
            started: Instant::now(),
        };
        (app, Views::new().with(SLICE, view))
    }

    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Whether the slice is on screen: the Image panel, no document or help
    /// box over it, and a decoded frame to show.
    fn image_on_screen(&self) -> bool {
        self.tab == Tab::Image
            && self.session.document.is_none()
            && !self.help
            && self.session.frame.is_some()
    }
}

impl scopekit::App for App {
    fn draw(&mut self, f: &mut Frame, view: &mut ViewSlot) {
        let layout = ui::layout(f.area());
        self.areas = Some(layout);
        self.cell_px = view.cell_px();
        self.describe = view.describe().to_string();
        if self.image_on_screen() {
            let px = view.px_size(layout.image);
            // Refit only when the panel changed size, so zoom and pan
            // survive switching panels and the help box.
            if self.image_cells != Some(layout.image) {
                self.image_cells = Some(layout.image);
                self.session.set_canvas(px);
                if let Some(img) = self.session.image() {
                    self.session.view = self.session.view.fit(px, img);
                }
                self.dirty = true;
            }
            if self.dirty {
                self.slice.borrow_mut().update(&mut self.session, self.look);
                self.dirty = false;
            }
            view.place(SLICE, layout.image);
        }
        ui::draw(f, self, &layout);
    }

    fn event(&mut self, event: Event) -> Flow {
        self.handle(event);
        if self.quit {
            Flow::Quit
        } else {
            Flow::Continue
        }
    }

    fn tick(&mut self) -> bool {
        if !self.session.playing {
            return false;
        }
        let before = self.session.current;
        self.session.cine_tick(self.now());
        let changed = self.session.current != before;
        self.dirty |= changed;
        changed
    }

    fn tick_interval(&self) -> Option<Duration> {
        self.session
            .playing
            .then(|| Duration::from_millis(u64::from(self.session.frame_ms().max(10.0) as u32) / 2))
    }
}

impl App {
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

    /// Open or close the key help. While it is open the slice is not
    /// placed: in a window it would be drawn over the help box.
    fn set_help(&mut self, on: bool) {
        self.help = on;
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

/// Run the viewer, in the terminal or with `--window` in its own window,
/// until the user quits.
pub fn run(args: &ViewArgs, gpu: GpuArgs) -> Result<(), String> {
    let (mut app, views) = App::new(args);
    let config = scopekit::Config {
        mode: if args.window {
            scopekit::Mode::Window
        } else {
            scopekit::Mode::Terminal
        },
        backend: gpu.backend.into(),
        protocol: args.protocol.into(),
        title: "dicomscope".into(),
        font: args.font.clone(),
        ..scopekit::Config::default()
    };
    scopekit::run(&mut app, views, &config)
}

impl From<Protocol> for scopekit::Protocol {
    fn from(p: Protocol) -> Self {
        match p {
            Protocol::Auto => scopekit::Protocol::Auto,
            Protocol::Kitty => scopekit::Protocol::Kitty,
            Protocol::Iterm2 => scopekit::Protocol::Iterm2,
            Protocol::Sixel => scopekit::Protocol::Sixel,
            Protocol::Halfblocks => scopekit::Protocol::Halfblocks,
        }
    }
}
