//! Zoom, pan, rotation and flip arithmetic. No browser types, so it is
//! tested on the host.
//!
//! Three coordinate spaces:
//!
//! * **source**: image pixels as stored, `(0,0)` top-left, `x` right, `y` down;
//! * **display**: the image after flip and rotation, still in image pixels,
//!   size [`Viewport::display_size`];
//! * **canvas**: device pixels on the canvas, `display * scale + (tx, ty)`.
//!
//! The shader performs canvas → display → source per fragment; the overlay
//! for measurements performs source → display → canvas per point. Both use
//! the same formulas, defined once here.

pub const MIN_SCALE: f32 = 0.02;
pub const MAX_SCALE: f32 = 128.0;

/// Canvas pixels per display pixel, the canvas position of the display
/// image's top-left corner, and the orientation. All lengths in device
/// pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub scale: f32,
    pub tx: f32,
    pub ty: f32,
    /// Bilinear interpolation when magnified; nearest otherwise.
    pub smooth: bool,
    /// Quarter turns clockwise, 0 to 3.
    pub rotation: u8,
    /// Mirror left-right (applied before rotation, in source space).
    pub flip_h: bool,
    /// Mirror top-bottom (applied before rotation, in source space).
    pub flip_v: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
            smooth: true,
            rotation: 0,
            flip_h: false,
            flip_v: false,
        }
    }
}

impl Viewport {
    /// Size of the image as displayed: width and height swap for 90 and 270
    /// degrees.
    pub fn display_size(&self, image: (u32, u32)) -> (u32, u32) {
        if self.rotation % 2 == 1 {
            (image.1, image.0)
        } else {
            image
        }
    }

    /// Largest scale at which the whole (rotated) image fits, centred.
    /// Orientation and interpolation are kept from `self`.
    pub fn fit(&self, canvas: (u32, u32), image: (u32, u32)) -> Viewport {
        let (cw, ch) = (canvas.0.max(1) as f32, canvas.1.max(1) as f32);
        let (dw, dh) = self.display_size(image);
        let (dw, dh) = (dw.max(1) as f32, dh.max(1) as f32);
        let scale = (cw / dw).min(ch / dh).clamp(MIN_SCALE, MAX_SCALE);
        Viewport {
            scale,
            tx: (cw - dw * scale) / 2.0,
            ty: (ch - dh * scale) / 2.0,
            ..*self
        }
    }

    /// One image pixel per device pixel, centred.
    pub fn one_to_one(&self, canvas: (u32, u32), image: (u32, u32)) -> Viewport {
        let (cw, ch) = (canvas.0 as f32, canvas.1 as f32);
        let (dw, dh) = self.display_size(image);
        Viewport {
            scale: 1.0,
            tx: (cw - dw as f32) / 2.0,
            ty: (ch - dh as f32) / 2.0,
            ..*self
        }
    }

    /// Multiply the scale by `factor`, keeping the image point under canvas
    /// position `(px, py)` fixed.
    pub fn zoom_about(self, factor: f32, px: f32, py: f32) -> Viewport {
        let scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let k = scale / self.scale;
        Viewport {
            scale,
            tx: px - (px - self.tx) * k,
            ty: py - (py - self.ty) * k,
            ..self
        }
    }

    pub fn pan(self, dx: f32, dy: f32) -> Viewport {
        Viewport {
            tx: self.tx + dx,
            ty: self.ty + dy,
            ..self
        }
    }

    /// Rotate by `quarter_turns` clockwise (negative for counter-clockwise),
    /// keeping the display centre where it is on the canvas.
    pub fn rotate(self, quarter_turns: i8, image: (u32, u32)) -> Viewport {
        let (cx, cy) = self.display_centre_on_canvas(image);
        let rotation = (self.rotation as i16 + quarter_turns as i16).rem_euclid(4) as u8;
        let rotated = Viewport { rotation, ..self };
        rotated.centre_display_at(cx, cy, image)
    }

    pub fn flip_horizontal(self) -> Viewport {
        Viewport {
            flip_h: !self.flip_h,
            ..self
        }
    }

    pub fn flip_vertical(self) -> Viewport {
        Viewport {
            flip_v: !self.flip_v,
            ..self
        }
    }

    fn display_centre_on_canvas(&self, image: (u32, u32)) -> (f32, f32) {
        let (dw, dh) = self.display_size(image);
        (
            self.tx + dw as f32 * self.scale / 2.0,
            self.ty + dh as f32 * self.scale / 2.0,
        )
    }

    fn centre_display_at(self, cx: f32, cy: f32, image: (u32, u32)) -> Viewport {
        let (dw, dh) = self.display_size(image);
        Viewport {
            tx: cx - dw as f32 * self.scale / 2.0,
            ty: cy - dh as f32 * self.scale / 2.0,
            ..self
        }
    }

    /// Source pixel coordinates to display coordinates (flip, then rotate).
    pub fn source_to_display(&self, s: (f32, f32), image: (u32, u32)) -> (f32, f32) {
        let (sw, sh) = (image.0 as f32, image.1 as f32);
        let x = if self.flip_h { sw - s.0 } else { s.0 };
        let y = if self.flip_v { sh - s.1 } else { s.1 };
        match self.rotation {
            1 => (sh - y, x),
            2 => (sw - x, sh - y),
            3 => (y, sw - x),
            _ => (x, y),
        }
    }

    /// Display coordinates back to source pixel coordinates.
    pub fn display_to_source(&self, d: (f32, f32), image: (u32, u32)) -> (f32, f32) {
        let (sw, sh) = (image.0 as f32, image.1 as f32);
        let (x, y) = match self.rotation {
            1 => (d.1, sh - d.0),
            2 => (sw - d.0, sh - d.1),
            3 => (sw - d.1, d.0),
            _ => d,
        };
        (
            if self.flip_h { sw - x } else { x },
            if self.flip_v { sh - y } else { y },
        )
    }

    pub fn source_to_canvas(&self, s: (f32, f32), image: (u32, u32)) -> (f32, f32) {
        let d = self.source_to_display(s, image);
        (d.0 * self.scale + self.tx, d.1 * self.scale + self.ty)
    }

    pub fn canvas_to_source(&self, c: (f32, f32), image: (u32, u32)) -> (f32, f32) {
        let d = ((c.0 - self.tx) / self.scale, (c.1 - self.ty) / self.scale);
        self.display_to_source(d, image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn fit_centres_and_limits() {
        let v = Viewport::default().fit((800, 600), (64, 64));
        assert!((v.scale - 9.375).abs() < 1e-5);
        assert!((v.tx - 100.0).abs() < 1e-3);
        assert!((v.ty - 0.0).abs() < 1e-3);
        let v = Viewport::default().fit((10, 10), (100_000, 10));
        assert_eq!(v.scale, MIN_SCALE);
        assert_eq!(
            Viewport::default().one_to_one((100, 100), (10, 10)).tx,
            45.0
        );
        // A rotated wide image fits as a tall one.
        let r = Viewport {
            rotation: 1,
            ..Viewport::default()
        }
        .fit((100, 400), (200, 50));
        assert_eq!(r.display_size((200, 50)), (50, 200));
        assert!((r.scale - 2.0).abs() < 1e-5);
    }

    #[test]
    fn zoom_keeps_point_fixed() {
        let v = Viewport {
            scale: 2.0,
            tx: 10.0,
            ty: 20.0,
            ..Viewport::default()
        };
        let (px, py) = (110.0, 220.0);
        let before = v.canvas_to_source((px, py), (500, 500));
        let z = v.zoom_about(1.5, px, py);
        let after = z.canvas_to_source((px, py), (500, 500));
        assert!(close(before, after));
        assert_eq!(z.scale, 3.0);
        assert!(v.zoom_about(1e9, 0.0, 0.0).scale <= MAX_SCALE);
        assert_eq!(
            v.pan(5.0, -5.0),
            Viewport {
                tx: 15.0,
                ty: 15.0,
                ..v
            }
        );
    }

    #[test]
    fn rotation_and_flip_round_trip() {
        let image = (40, 30);
        let point = (3.0, 7.0);
        for rotation in 0..4 {
            for (flip_h, flip_v) in [(false, false), (true, false), (false, true), (true, true)] {
                let v = Viewport {
                    scale: 1.5,
                    tx: 11.0,
                    ty: -4.0,
                    rotation,
                    flip_h,
                    flip_v,
                    ..Viewport::default()
                };
                let c = v.source_to_canvas(point, image);
                assert!(
                    close(v.canvas_to_source(c, image), point),
                    "r{rotation} h{flip_h} v{flip_v}"
                );
            }
        }
    }

    #[test]
    fn ninety_degrees_clockwise_moves_top_left_to_top_right() {
        let image = (40, 30);
        let v = Viewport {
            rotation: 1,
            ..Viewport::default()
        };
        // Display is 30 wide, 40 tall; source top-left lands at display (30, 0).
        assert!(close(v.source_to_display((0.0, 0.0), image), (30.0, 0.0)));
        assert!(close(v.source_to_display((40.0, 0.0), image), (30.0, 40.0)));
        let ccw = Viewport {
            rotation: 3,
            ..Viewport::default()
        };
        assert!(close(ccw.source_to_display((0.0, 0.0), image), (0.0, 40.0)));
        let h = Viewport::default().flip_horizontal();
        assert!(close(h.source_to_display((0.0, 5.0), image), (40.0, 5.0)));
    }

    #[test]
    fn rotate_keeps_the_centre_on_the_canvas() {
        let image = (200, 50);
        let v = Viewport::default().fit((300, 300), image);
        let centre_before = v.source_to_canvas((100.0, 25.0), image);
        let r = v.rotate(1, image);
        assert_eq!(r.rotation, 1);
        let centre_after = r.source_to_canvas((100.0, 25.0), image);
        assert!(close(centre_before, centre_after));
        assert_eq!(r.rotate(-1, image).rotation, 0);
        assert_eq!(Viewport::default().rotate(-1, image).rotation, 3);
    }
}
