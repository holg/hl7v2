//! Mandibular canal (inferior alveolar nerve) tracing on a panoramic or
//! CBCT-derived image: the dentist's own tool, a smooth tube through the
//! points they click, painted yellow by convention. Nothing is detected
//! here; see the notes on semi-automatic tracing for what could feed it.

use crate::measure::{length_px, Point, Spacing};

/// Default tube diameter when the trace carries none: the canal is 3 to
/// 5 mm wide on an adult mandible.
pub const DEFAULT_DIAMETER_MM: f32 = 3.0;

/// Which canal, from where the trace lies on the image. Panoramic images
/// show the patient's right on the viewer's left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Side {
    Right,
    Left,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Right => "R",
            Side::Left => "L",
        }
    }
}

/// One traced canal: the clicked points in source pixels, in order from
/// either end, and the tube diameter.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NerveTrace {
    pub points: Vec<Point>,
    pub diameter_mm: f32,
}

impl NerveTrace {
    pub fn new(points: Vec<Point>) -> NerveTrace {
        NerveTrace {
            points,
            diameter_mm: DEFAULT_DIAMETER_MM,
        }
    }

    /// The side the trace lies on, from its mean x against the image width.
    pub fn side(&self, image_width: u32) -> Side {
        let n = self.points.len().max(1) as f32;
        let mean_x = self.points.iter().map(|p| p.0).sum::<f32>() / n;
        if mean_x < image_width as f32 / 2.0 {
            Side::Right
        } else {
            Side::Left
        }
    }

    /// A smooth curve through the points: centripetal Catmull-Rom, with the
    /// end points doubled so the curve starts and ends exactly on them.
    /// `per_segment` samples between each pair of points.
    pub fn spline(&self, per_segment: usize) -> Vec<Point> {
        let p = &self.points;
        if p.len() < 2 {
            return p.clone();
        }
        let per = per_segment.max(1);
        let mut out = Vec::with_capacity((p.len() - 1) * per + 1);
        let at = |i: isize| -> Point {
            let i = i.clamp(0, p.len() as isize - 1) as usize;
            p[i]
        };
        for i in 0..p.len() - 1 {
            let (p0, p1, p2, p3) = (
                at(i as isize - 1),
                at(i as isize),
                at(i as isize + 1),
                at(i as isize + 2),
            );
            for s in 0..per {
                let t = s as f32 / per as f32;
                out.push(catmull_rom(p0, p1, p2, p3, t));
            }
        }
        out.push(p[p.len() - 1]);
        out
    }

    /// Length along the spline.
    pub fn length_px(&self) -> f32 {
        polyline_length(&self.spline(8), None)
    }

    /// Length along the spline in millimetres, when the image says how big
    /// a pixel is.
    pub fn length_mm(&self, spacing: Spacing) -> f32 {
        polyline_length(&self.spline(8), Some(spacing))
    }

    /// `R nerve 42.3 mm` or `L nerve 310 px`.
    pub fn label(&self, image_width: u32, spacing: Option<Spacing>) -> String {
        let side = self.side(image_width).label();
        match spacing {
            Some(s) => format!("{side} nerve {:.1} mm", self.length_mm(s)),
            None => format!("{side} nerve {:.0} px", self.length_px()),
        }
    }

    /// Tube diameter in source pixels (columns), 8 px without spacing.
    pub fn diameter_px(&self, spacing: Option<Spacing>) -> f32 {
        match spacing {
            Some(s) => self.diameter_mm / s.col_mm,
            None => 8.0,
        }
    }
}

fn catmull_rom(p0: Point, p1: Point, p2: Point, p3: Point, t: f32) -> Point {
    // Uniform Catmull-Rom (alpha 0); adequate for hand-clicked points a few
    // millimetres apart, and it passes exactly through p1 and p2.
    let t2 = t * t;
    let t3 = t2 * t;
    let f = |a: f32, b: f32, c: f32, d: f32| {
        0.5 * ((2.0 * b)
            + (-a + c) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
            + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    (f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1))
}

fn polyline_length(pts: &[Point], spacing: Option<Spacing>) -> f32 {
    pts.windows(2)
        .map(|w| match spacing {
            Some(s) => crate::measure::length_mm(w[0], w[1], s),
            None => length_px(w[0], w[1]),
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_passes_through_the_points_and_measures() {
        let t = NerveTrace::new(vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)]);
        let s = t.spline(4);
        assert_eq!(s.first(), Some(&(0.0, 0.0)));
        assert_eq!(s.last(), Some(&(20.0, 0.0)));
        assert_eq!(s.len(), 9);
        assert!((t.length_px() - 20.0).abs() < 1e-3);
        let sp = Spacing {
            row_mm: 0.1,
            col_mm: 0.1,
            at_detector: false,
        };
        assert!((t.length_mm(sp) - 2.0).abs() < 1e-3);
        assert_eq!(t.label(100, Some(sp)), "R nerve 2.0 mm");
        assert_eq!(t.side(10), Side::Left);
        assert!((t.diameter_px(Some(sp)) - 30.0).abs() < 1e-3);
    }

    #[test]
    fn degenerate_traces_do_not_panic() {
        assert!(NerveTrace::new(vec![]).spline(4).is_empty());
        assert_eq!(NerveTrace::new(vec![(1.0, 1.0)]).spline(4).len(), 1);
        assert_eq!(NerveTrace::new(vec![(1.0, 1.0)]).length_px(), 0.0);
    }
}
