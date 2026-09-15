//! Length and angle measurements in source pixel coordinates, reported in
//! millimetres when the image says how big a pixel is. Host-tested.

/// Physical size of one pixel, from Pixel Spacing (0028,0030) or, for
/// projection images, Imager Pixel Spacing (0018,1164). DICOM orders the
/// pair as row spacing (vertical) then column spacing (horizontal).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    pub row_mm: f32,
    pub col_mm: f32,
    /// True when taken from Imager Pixel Spacing, which measures at the
    /// detector rather than in the patient.
    pub at_detector: bool,
}

impl Spacing {
    /// Parse a `row\col` decimal string pair.
    pub fn parse(text: &str, at_detector: bool) -> Option<Spacing> {
        let mut it = text.split('\\').map(|s| s.trim().parse::<f32>().ok());
        let row = it.next()??;
        let col = it.next().flatten().unwrap_or(row);
        (row > 0.0 && col > 0.0 && row.is_finite() && col.is_finite()).then_some(Spacing {
            row_mm: row,
            col_mm: col,
            at_detector,
        })
    }

    fn mm(self, p: (f32, f32)) -> (f32, f32) {
        (p.0 * self.col_mm, p.1 * self.row_mm)
    }
}

/// A point in source pixel coordinates (continuous; pixel centres at +0.5).
pub type Point = (f32, f32);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Measurement {
    Length {
        a: Point,
        b: Point,
    },
    /// Angle at `vertex` between the rays to `a` and `c`.
    Angle {
        a: Point,
        vertex: Point,
        c: Point,
    },
}

impl Measurement {
    /// The value with its unit, e.g. `12.3 mm`, `47 px` or `38.5°`.
    pub fn label(&self, spacing: Option<Spacing>) -> String {
        match *self {
            Measurement::Length { a, b } => match spacing {
                Some(s) => format!(
                    "{:.1} mm{}",
                    length_mm(a, b, s),
                    if s.at_detector { "*" } else { "" }
                ),
                None => format!("{:.0} px", length_px(a, b)),
            },
            Measurement::Angle { a, vertex, c } => {
                format!("{:.1}°", angle_deg(a, vertex, c, spacing))
            }
        }
    }

    /// The points that define it, for drawing.
    pub fn points(&self) -> Vec<Point> {
        match *self {
            Measurement::Length { a, b } => vec![a, b],
            Measurement::Angle { a, vertex, c } => vec![a, vertex, c],
        }
    }
}

pub fn length_px(a: Point, b: Point) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn length_mm(a: Point, b: Point, s: Spacing) -> f32 {
    let (a, b) = (s.mm(a), s.mm(b));
    length_px(a, b)
}

/// Angle in degrees at `v`, measured in physical space when spacing is
/// known so that non-square pixels do not distort it.
pub fn angle_deg(a: Point, v: Point, c: Point, spacing: Option<Spacing>) -> f32 {
    let (a, v, c) = match spacing {
        Some(s) => (s.mm(a), s.mm(v), s.mm(c)),
        None => (a, v, c),
    };
    let u = (a.0 - v.0, a.1 - v.1);
    let w = (c.0 - v.0, c.1 - v.1);
    let dot = u.0 * w.0 + u.1 * w.1;
    let cross = u.0 * w.1 - u.1 * w.0;
    cross.abs().atan2(dot).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_parses_row_then_col() {
        let s = Spacing::parse("0.5\\0.25", false).unwrap();
        assert_eq!((s.row_mm, s.col_mm), (0.5, 0.25));
        assert_eq!(Spacing::parse("0.7", true).unwrap().col_mm, 0.7);
        assert!(Spacing::parse("0\\1", false).is_none());
        assert!(Spacing::parse("abc", false).is_none());
    }

    #[test]
    fn lengths() {
        let s = Spacing {
            row_mm: 2.0,
            col_mm: 0.5,
            at_detector: false,
        };
        assert_eq!(length_px((0.0, 0.0), (3.0, 4.0)), 5.0);
        // 3 px across at 0.5 mm, 4 px down at 2 mm: 1.5 and 8 mm -> 8.14 mm.
        assert!((length_mm((0.0, 0.0), (3.0, 4.0), s) - 8.139).abs() < 1e-2);
        let m = Measurement::Length {
            a: (0.0, 0.0),
            b: (3.0, 4.0),
        };
        assert_eq!(m.label(None), "5 px");
        assert_eq!(m.label(Some(s)), "8.1 mm");
        assert_eq!(
            m.label(Some(Spacing {
                at_detector: true,
                ..s
            })),
            "8.1 mm*"
        );
    }

    #[test]
    fn angles() {
        let right = angle_deg((1.0, 0.0), (0.0, 0.0), (0.0, 1.0), None);
        assert!((right - 90.0).abs() < 1e-4);
        let straight = angle_deg((-1.0, 0.0), (0.0, 0.0), (1.0, 0.0), None);
        assert!((straight - 180.0).abs() < 1e-4);
        let acute = angle_deg((1.0, 0.0), (0.0, 0.0), (1.0, 1.0), None);
        assert!((acute - 45.0).abs() < 1e-4);
        // Non-square pixels: the same pixel triangle is not 45 degrees in mm.
        let s = Spacing {
            row_mm: 2.0,
            col_mm: 1.0,
            at_detector: false,
        };
        let stretched = angle_deg((1.0, 0.0), (0.0, 0.0), (1.0, 1.0), Some(s));
        assert!((stretched - 63.43).abs() < 1e-2);
        let m = Measurement::Angle {
            a: (1.0, 0.0),
            vertex: (0.0, 0.0),
            c: (0.0, 1.0),
        };
        assert_eq!(m.label(None), "90.0°");
        assert_eq!(m.points().len(), 3);
    }
}
