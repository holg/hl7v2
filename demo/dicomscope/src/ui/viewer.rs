//! Keyboard, mouse and wheel handling for the canvas. The arithmetic is in
//! [`crate::view::Viewport`] and [`crate::measure`], both tested on the
//! host; [`ViewControls`] only translates events into calls on them.

use crate::measure::{Measurement, Point};
use leptos::prelude::*;
use leptos::web_sys::{HtmlCanvasElement, KeyboardEvent, MouseEvent, WheelEvent};

pub use crate::view::Viewport;

/// What a mouse drag on the canvas does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Pan,
    Length,
    Angle,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Pan => "Pan",
            Tool::Length => "Length",
            Tool::Angle => "Angle",
        }
    }
}

/// A measurement being drawn: the points fixed so far.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Draft {
    pub tool: Tool,
    pub points: Vec<Point>,
}

/// Device pixel ratio, 1.0 when unavailable.
pub fn device_pixel_ratio() -> f32 {
    leptos::web_sys::window()
        .map(|w| w.device_pixel_ratio() as f32)
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(1.0)
}

/// Size the canvas backing store to its CSS box times the pixel ratio.
/// Returns the new size in device pixels.
pub fn sync_backing_size(canvas: &HtmlCanvasElement) -> (u32, u32) {
    let dpr = device_pixel_ratio() as f64;
    let w = ((canvas.client_width() as f64) * dpr).round().max(1.0) as u32;
    let h = ((canvas.client_height() as f64) * dpr).round().max(1.0) as u32;
    if canvas.width() != w {
        canvas.set_width(w);
    }
    if canvas.height() != h {
        canvas.set_height(h);
    }
    (w, h)
}

/// Mouse position relative to the canvas, in device pixels.
fn canvas_position(canvas: &HtmlCanvasElement, ev: &MouseEvent) -> (f32, f32) {
    let rect = canvas.get_bounding_client_rect();
    let dpr = device_pixel_ratio();
    (
        (ev.client_x() as f32 - rect.left() as f32) * dpr,
        (ev.client_y() as f32 - rect.top() as f32) * dpr,
    )
}

/// Wheel, drag, click and keyboard handlers for the canvas.
///
/// The canvas size is read from the element (and its backing store synced)
/// whenever it is needed, so layout timing cannot leave a stale size behind.
#[derive(Clone, Copy)]
pub struct ViewControls {
    pub view: RwSignal<Viewport>,
    pub canvas: NodeRef<leptos::html::Canvas>,
    pub image_size: Signal<Option<(u32, u32)>>,
    pub tool: RwSignal<Tool>,
    /// Measurements on the slice currently shown, in source pixels.
    pub measurements: RwSignal<Vec<Measurement>>,
    /// The measurement being drawn, with the cursor as its last point.
    pub draft: RwSignal<Option<Draft>>,
    drag: StoredValue<Option<(i32, i32)>>,
}

impl ViewControls {
    pub fn new(
        view: RwSignal<Viewport>,
        canvas: NodeRef<leptos::html::Canvas>,
        image_size: Signal<Option<(u32, u32)>>,
    ) -> Self {
        ViewControls {
            view,
            canvas,
            image_size,
            tool: RwSignal::new(Tool::Pan),
            measurements: RwSignal::new(Vec::new()),
            draft: RwSignal::new(None),
            drag: StoredValue::new(None),
        }
    }

    /// Current backing-store size in device pixels, synced to the CSS box.
    pub fn canvas_size(&self) -> (u32, u32) {
        self.canvas
            .get_untracked()
            .map(|c| sync_backing_size(&c))
            .unwrap_or((1, 1))
    }

    fn image(&self) -> Option<(u32, u32)> {
        self.image_size.get_untracked()
    }

    pub fn fit(&self) {
        if let Some(img) = self.image() {
            let size = self.canvas_size();
            self.view.update(|v| *v = v.fit(size, img));
        }
    }

    pub fn one_to_one(&self) {
        if let Some(img) = self.image() {
            let size = self.canvas_size();
            self.view.update(|v| *v = v.one_to_one(size, img));
        }
    }

    pub fn toggle_smooth(&self) {
        self.view.update(|v| v.smooth = !v.smooth);
    }

    pub fn rotate(&self, quarter_turns: i8) {
        if let Some(img) = self.image() {
            self.view.update(|v| *v = v.rotate(quarter_turns, img));
        }
    }

    pub fn flip_horizontal(&self) {
        self.view.update(|v| *v = v.flip_horizontal());
    }

    pub fn flip_vertical(&self) {
        self.view.update(|v| *v = v.flip_vertical());
    }

    /// Back to upright, unmirrored, fitted.
    pub fn reset_orientation(&self) {
        self.view.update(|v| {
            v.rotation = 0;
            v.flip_h = false;
            v.flip_v = false;
        });
        self.fit();
    }

    pub fn zoom_in(&self) {
        self.zoom_centre(1.25);
    }

    pub fn zoom_out(&self) {
        self.zoom_centre(0.8);
    }

    fn zoom_centre(&self, factor: f32) {
        let (w, h) = self.canvas_size();
        self.view
            .update(|v| *v = v.zoom_about(factor, w as f32 / 2.0, h as f32 / 2.0));
    }

    pub fn set_tool(&self, tool: Tool) {
        self.tool.set(tool);
        self.draft.set(None);
    }

    pub fn clear_measurements(&self) {
        self.measurements.set(Vec::new());
        self.draft.set(None);
    }

    pub fn remove_last_measurement(&self) {
        if self.draft.get_untracked().is_some() {
            self.draft.set(None);
        } else {
            self.measurements.update(|m| {
                m.pop();
            });
        }
    }

    pub fn on_wheel(&self, canvas: &HtmlCanvasElement, ev: &WheelEvent) {
        ev.prevent_default();
        // Trackpads deliver many small deltas, wheels a few large ones; an
        // exponential map keeps both usable.
        let factor = (-(ev.delta_y() as f32) * 0.0025).exp();
        let (px, py) = canvas_position(canvas, ev);
        self.view.update(|v| *v = v.zoom_about(factor, px, py));
    }

    /// Source pixel under the mouse, or `None` without an image.
    fn source_point(&self, canvas: &HtmlCanvasElement, ev: &MouseEvent) -> Option<Point> {
        let img = self.image()?;
        let c = canvas_position(canvas, ev);
        Some(self.view.get_untracked().canvas_to_source(c, img))
    }

    pub fn on_mouse_down(&self, canvas: &HtmlCanvasElement, ev: &MouseEvent) {
        if ev.button() != 0 {
            return;
        }
        ev.prevent_default();
        match self.tool.get_untracked() {
            Tool::Pan => self.drag.set_value(Some((ev.client_x(), ev.client_y()))),
            Tool::Length => {
                if let Some(p) = self.source_point(canvas, ev) {
                    self.draft.set(Some(Draft {
                        tool: Tool::Length,
                        points: vec![p, p],
                    }));
                }
            }
            Tool::Angle => {
                let Some(p) = self.source_point(canvas, ev) else {
                    return;
                };
                let mut draft = self.draft.get_untracked().unwrap_or(Draft {
                    tool: Tool::Angle,
                    points: vec![p],
                });
                // The last point tracks the cursor; a click fixes it.
                if draft.points.len() >= 3 {
                    let m = Measurement::Angle {
                        a: draft.points[0],
                        vertex: draft.points[1],
                        c: p,
                    };
                    self.measurements.update(|ms| ms.push(m));
                    self.draft.set(None);
                    return;
                }
                if let Some(last) = draft.points.last_mut() {
                    *last = p;
                }
                draft.points.push(p);
                self.draft.set(Some(draft));
            }
        }
    }

    pub fn on_mouse_move(&self, canvas: &HtmlCanvasElement, ev: &MouseEvent) {
        if let Some(mut draft) = self.draft.get_untracked() {
            if let Some(p) = self.source_point(canvas, ev) {
                if let Some(last) = draft.points.last_mut() {
                    *last = p;
                }
                self.draft.set(Some(draft));
            }
            return;
        }
        let Some((x0, y0)) = self.drag.get_value() else {
            return;
        };
        if ev.buttons() & 1 == 0 {
            self.drag.set_value(None);
            return;
        }
        let dpr = device_pixel_ratio();
        let (x, y) = (ev.client_x(), ev.client_y());
        self.drag.set_value(Some((x, y)));
        self.view
            .update(|v| *v = v.pan((x - x0) as f32 * dpr, (y - y0) as f32 * dpr));
    }

    pub fn on_mouse_up(&self, canvas: &HtmlCanvasElement, ev: &MouseEvent) {
        self.drag.set_value(None);
        if let Some(draft) = self.draft.get_untracked() {
            if draft.tool == Tool::Length && draft.points.len() == 2 {
                let a = draft.points[0];
                let b = self.source_point(canvas, ev).unwrap_or(draft.points[1]);
                // Ignore clicks; a length needs a drag of a few pixels.
                let scale = self.view.get_untracked().scale.max(1e-3);
                if crate::measure::length_px(a, b) * scale >= 3.0 {
                    self.measurements
                        .update(|ms| ms.push(Measurement::Length { a, b }));
                }
                self.draft.set(None);
            }
        }
    }

    pub fn on_mouse_leave(&self) {
        self.drag.set_value(None);
        if self
            .draft
            .get_untracked()
            .map(|d| d.tool == Tool::Length)
            .unwrap_or(false)
        {
            self.draft.set(None);
        }
    }

    /// Arrow keys pan, `+`/`-` zoom, `0` fits, `1` is 1:1, `i` toggles
    /// interpolation, `r`/`R` rotate, `h`/`v` flip, `l`/`a`/`p` pick a tool,
    /// Escape cancels, Delete removes the last measurement. Returns whether
    /// the key was handled.
    pub fn on_key(&self, ev: &KeyboardEvent) -> bool {
        let step = if ev.shift_key() { 100.0 } else { 20.0 } * device_pixel_ratio();
        let handled = match ev.key().as_str() {
            "ArrowLeft" => {
                self.view.update(|v| *v = v.pan(step, 0.0));
                true
            }
            "ArrowRight" => {
                self.view.update(|v| *v = v.pan(-step, 0.0));
                true
            }
            "ArrowUp" => {
                self.view.update(|v| *v = v.pan(0.0, step));
                true
            }
            "ArrowDown" => {
                self.view.update(|v| *v = v.pan(0.0, -step));
                true
            }
            "+" | "=" => {
                self.zoom_in();
                true
            }
            "-" | "_" => {
                self.zoom_out();
                true
            }
            "0" | "f" | "F" => {
                self.fit();
                true
            }
            "1" => {
                self.one_to_one();
                true
            }
            "i" | "I" => {
                self.toggle_smooth();
                true
            }
            "r" => {
                self.rotate(1);
                true
            }
            "R" => {
                self.rotate(-1);
                true
            }
            "h" | "H" => {
                self.flip_horizontal();
                true
            }
            "v" | "V" => {
                self.flip_vertical();
                true
            }
            "o" | "O" => {
                self.reset_orientation();
                true
            }
            "l" | "L" => {
                self.set_tool(Tool::Length);
                true
            }
            "a" | "A" => {
                self.set_tool(Tool::Angle);
                true
            }
            "p" | "P" => {
                self.set_tool(Tool::Pan);
                true
            }
            "Escape" => {
                self.draft.set(None);
                self.set_tool(Tool::Pan);
                true
            }
            "Delete" | "Backspace" => {
                self.remove_last_measurement();
                true
            }
            _ => false,
        };
        if handled {
            ev.prevent_default();
        }
        handled
    }
}
