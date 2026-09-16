//! Semi-automatic mandibular canal tracing: between two points the user
//! marks (the mental and the mandibular foramen), find the path that best
//! follows a dark band of the canal's width. Classical image analysis, no
//! learned model: a Frangi-style valley filter at the canal's scale gives a
//! cost image, and a minimal-cost path (Dijkstra) between the two points
//! follows the canal through it. The result is an editable [`NerveTrace`];
//! it is a proposal for the dentist, not a finding.
//!
//! Where it works: the body of the mandible, where the canal is a
//! continuous dark band between two cortical lines. Where it does not:
//! near the foramina, and where roots or the hyoid cross the canal. The
//! trace is therefore always handed to the manual tool for correction.

use crate::measure::Point;
use crate::nerve::NerveTrace;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// A found canal and how convincing the band was along it (0 to 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub trace: NerveTrace,
    pub confidence: f32,
}

/// Find the canal between `a` and `b` on a greyscale image.
/// `canal_width_px` is the expected band width in source pixels (3 to 5 mm
/// at the image's pixel spacing); the search runs in a window around the
/// two points at a scale where the band is about 8 px wide.
pub fn find_canal(
    gray: &[f32],
    width: usize,
    height: usize,
    a: Point,
    b: Point,
    canal_width_px: f32,
) -> Result<Found, String> {
    if gray.len() != width * height || width < 8 || height < 8 {
        return Err("image too small".into());
    }
    let inside = |p: Point| p.0 >= 0.0 && p.1 >= 0.0 && p.0 < width as f32 && p.1 < height as f32;
    if !inside(a) || !inside(b) {
        return Err("both points must lie on the image".into());
    }
    let dist = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    if dist < 4.0 {
        return Err("the two points are on top of each other".into());
    }
    let canal = canal_width_px.max(2.0);

    // Window around the two points, with room for the canal's curve.
    let margin = (0.6 * dist).max(6.0 * canal);
    let x0 = (a.0.min(b.0) - margin).floor().max(0.0) as usize;
    let y0 = (a.1.min(b.1) - margin).floor().max(0.0) as usize;
    let x1 = (a.0.max(b.0) + margin).ceil().min(width as f32) as usize;
    let y1 = (a.1.max(b.1) + margin).ceil().min(height as f32) as usize;

    // Downsample so the band is about 8 px wide: the filter and the path
    // search then cost the same on a 3000 px panoramic as on a 500 px one.
    let f = ((canal / 8.0).round() as usize).max(1);
    let rw = (x1 - x0) / f;
    let rh = (y1 - y0) / f;
    if rw < 4 || rh < 4 {
        return Err("search window too small".into());
    }
    let mut roi = vec![0.0f32; rw * rh];
    for ry in 0..rh {
        for rx in 0..rw {
            let mut sum = 0.0;
            for dy in 0..f {
                for dx in 0..f {
                    sum += gray[(y0 + ry * f + dy) * width + x0 + rx * f + dx];
                }
            }
            roi[ry * rw + rx] = sum / (f * f) as f32;
        }
    }
    normalize(&mut roi);

    // Valley-ness at two scales around the band's half width.
    let w = canal / f as f32;
    let mut v = vec![0.0f32; rw * rh];
    for sigma in [w / 4.0, w / 2.5] {
        let r = valleyness(&roi, rw, rh, sigma.max(0.7));
        for (o, x) in v.iter_mut().zip(r) {
            *o = o.max(x);
        }
    }
    let vmax = v.iter().cloned().fold(0.0f32, f32::max);
    if vmax <= 0.0 {
        return Err("no band-like structure between the points".into());
    }
    for x in &mut v {
        *x /= vmax;
    }
    // Following the band is cheap, leaving it costs; a floor keeps the path
    // from wandering far to collect weak responses.
    let cost: Vec<f32> = v.iter().map(|&x| 1.0 / (0.03 + x)).collect();

    let to_roi = |p: Point| -> (usize, usize) {
        (
            (((p.0 - x0 as f32) / f as f32) as usize).min(rw - 1),
            (((p.1 - y0 as f32) / f as f32) as usize).min(rh - 1),
        )
    };
    let path =
        shortest_path(&cost, rw, rh, to_roi(a), to_roi(b)).ok_or("no path between the points")?;
    let confidence =
        path.iter().map(|&(x, y)| v[y * rw + x]).sum::<f32>() / path.len().max(1) as f32;

    // Back to source pixels, thinned to a handful of control points, with
    // the user's own end points kept exactly.
    let full: Vec<Point> = path
        .iter()
        .map(|&(x, y)| {
            (
                x0 as f32 + (x as f32 + 0.5) * f as f32,
                y0 as f32 + (y as f32 + 0.5) * f as f32,
            )
        })
        .collect();
    let mut points = simplify(&full, (canal / 2.0).max(2.0));
    if let Some(first) = points.first_mut() {
        *first = a;
    }
    if let Some(last) = points.last_mut() {
        *last = b;
    }
    if points.len() < 2 {
        points = vec![a, b];
    }
    Ok(Found {
        trace: NerveTrace::new(points),
        confidence,
    })
}

/// Robust 0..1 scaling by the 1st and 99th percentile.
fn normalize(img: &mut [f32]) {
    let mut sorted: Vec<f32> = img.iter().cloned().filter(|x| x.is_finite()).collect();
    if sorted.is_empty() {
        return;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let lo = sorted[sorted.len() / 100];
    let hi = sorted[sorted.len() * 99 / 100];
    let span = (hi - lo).max(1e-6);
    for x in img.iter_mut() {
        *x = ((*x - lo) / span).clamp(0.0, 1.0);
    }
}

/// Separable Gaussian blur.
fn blur(img: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as isize;
    let kernel: Vec<f32> = (-r..=r)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.iter().map(|k| k / sum).collect();
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (k, kv) in kernel.iter().enumerate() {
                let sx = (x as isize + k as isize - r).clamp(0, w as isize - 1) as usize;
                acc += img[y * w + sx] * kv;
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (k, kv) in kernel.iter().enumerate() {
                let sy = (y as isize + k as isize - r).clamp(0, h as isize - 1) as usize;
                acc += tmp[sy * w + x] * kv;
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// Frangi-style response for dark line-like structures (a valley) at one
/// scale: large positive curvature across the line, little along it.
fn valleyness(img: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let g = blur(img, w, h, sigma);
    let at = |x: isize, y: isize| {
        g[(y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize]
    };
    let mut out = vec![0.0f32; w * h];
    let mut s_max = 0.0f32;
    let mut raw = Vec::with_capacity(w * h);
    for y in 0..h as isize {
        for x in 0..w as isize {
            let ixx = at(x + 1, y) - 2.0 * at(x, y) + at(x - 1, y);
            let iyy = at(x, y + 1) - 2.0 * at(x, y) + at(x, y - 1);
            let ixy =
                (at(x + 1, y + 1) - at(x + 1, y - 1) - at(x - 1, y + 1) + at(x - 1, y - 1)) / 4.0;
            // Scale-normalised Hessian eigenvalues, |l1| <= |l2|.
            let s2 = sigma * sigma;
            let (ixx, iyy, ixy) = (ixx * s2, iyy * s2, ixy * s2);
            let mean = (ixx + iyy) / 2.0;
            let d = (((ixx - iyy) / 2.0).powi(2) + ixy * ixy).sqrt();
            let (mut l1, mut l2) = (mean - d, mean + d);
            if l1.abs() > l2.abs() {
                std::mem::swap(&mut l1, &mut l2);
            }
            let s = (l1 * l1 + l2 * l2).sqrt();
            s_max = s_max.max(s);
            raw.push((l1, l2, s));
        }
    }
    let beta = 0.5f32;
    let c = (0.5 * s_max).max(1e-6);
    for (o, (l1, l2, s)) in out.iter_mut().zip(raw) {
        // A dark line has a positive second derivative across it.
        if l2 <= 0.0 {
            continue;
        }
        let rb = l1.abs() / l2.abs().max(1e-6);
        *o = (-(rb * rb) / (2.0 * beta * beta)).exp() * (1.0 - (-(s * s) / (2.0 * c * c)).exp());
    }
    out
}

#[derive(PartialEq)]
struct Node {
    cost: f32,
    idx: usize,
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}

/// Dijkstra on the 8-connected grid; edge cost is the mean of the two
/// pixel costs times the step length.
fn shortest_path(
    cost: &[f32],
    w: usize,
    h: usize,
    from: (usize, usize),
    to: (usize, usize),
) -> Option<Vec<(usize, usize)>> {
    let n = w * h;
    let start = from.1 * w + from.0;
    let goal = to.1 * w + to.0;
    let mut dist = vec![f32::INFINITY; n];
    let mut prev = vec![usize::MAX; n];
    let mut heap = BinaryHeap::new();
    dist[start] = 0.0;
    heap.push(Node {
        cost: 0.0,
        idx: start,
    });
    const STEPS: [(isize, isize, f32); 8] = [
        (1, 0, 1.0),
        (-1, 0, 1.0),
        (0, 1, 1.0),
        (0, -1, 1.0),
        (1, 1, std::f32::consts::SQRT_2),
        (1, -1, std::f32::consts::SQRT_2),
        (-1, 1, std::f32::consts::SQRT_2),
        (-1, -1, std::f32::consts::SQRT_2),
    ];
    while let Some(Node { cost: d, idx }) = heap.pop() {
        if idx == goal {
            break;
        }
        if d > dist[idx] {
            continue;
        }
        let (x, y) = ((idx % w) as isize, (idx / w) as isize);
        for (dx, dy, len) in STEPS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                continue;
            }
            let ni = ny as usize * w + nx as usize;
            let nd = d + len * (cost[idx] + cost[ni]) / 2.0;
            if nd < dist[ni] {
                dist[ni] = nd;
                prev[ni] = idx;
                heap.push(Node { cost: nd, idx: ni });
            }
        }
    }
    if !dist[goal].is_finite() {
        return None;
    }
    let mut path = vec![(to.0, to.1)];
    let mut cur = goal;
    while cur != start {
        cur = prev[cur];
        if cur == usize::MAX {
            return None;
        }
        path.push((cur % w, cur / w));
    }
    path.reverse();
    Some(path)
}

/// Douglas-Peucker line simplification.
fn simplify(pts: &[Point], tol: f32) -> Vec<Point> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (mut best, mut best_d) = (0usize, 0.0f32);
    for (i, p) in pts.iter().enumerate().skip(1).take(pts.len() - 2) {
        let d = ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len;
        if d > best_d {
            best_d = d;
            best = i;
        }
    }
    if best_d <= tol {
        return vec![a, b];
    }
    let mut left = simplify(&pts[..=best], tol);
    let right = simplify(&pts[best..], tol);
    left.pop();
    left.extend(right);
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    type Centre = Box<dyn Fn(f32) -> f32>;

    /// A bright image with a dark curved band 8 px wide, the canal's shape.
    fn synthetic() -> (Vec<f32>, usize, usize, Centre) {
        let (w, h) = (320usize, 200usize);
        let centre = |x: f32| 100.0 + 30.0 * (x / 50.0).sin();
        let mut img = vec![1000.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let d = (y as f32 - centre(x as f32)).abs();
                if d < 4.0 {
                    img[y * w + x] = 300.0;
                } else if d < 6.0 {
                    img[y * w + x] = 1400.0; // cortical lines
                }
            }
        }
        (img, w, h, Box::new(centre))
    }

    #[test]
    fn follows_a_curved_dark_band_between_two_points() {
        let (img, w, h, centre) = synthetic();
        let a = (20.0, centre(20.0));
        let b = (300.0, centre(300.0));
        let found = find_canal(&img, w, h, a, b, 8.0).unwrap();
        assert!(found.confidence > 0.3, "confidence {}", found.confidence);
        assert_eq!(found.trace.points.first(), Some(&a));
        assert_eq!(found.trace.points.last(), Some(&b));
        assert!(
            found.trace.points.len() >= 4,
            "control points: {}",
            found.trace.points.len()
        );
        for p in found.trace.spline(8) {
            let err = (p.1 - centre(p.0)).abs();
            assert!(err < 6.0, "off the band by {err} px at x={}", p.0);
        }
    }

    #[test]
    fn refuses_bad_input() {
        let (img, w, h, _) = synthetic();
        assert!(find_canal(&img, w, h, (10.0, 10.0), (11.0, 10.0), 8.0).is_err());
        assert!(find_canal(&img, w, h, (-1.0, 10.0), (50.0, 10.0), 8.0).is_err());
        assert!(find_canal(&img[..10], w, h, (0.0, 0.0), (5.0, 5.0), 8.0).is_err());
    }

    #[test]
    fn simplify_keeps_ends_and_corners() {
        let pts = vec![(0.0, 0.0), (1.0, 0.1), (2.0, 0.0), (3.0, 5.0), (4.0, 0.0)];
        let s = simplify(&pts, 0.5);
        assert_eq!(s.first(), Some(&(0.0, 0.0)));
        assert_eq!(s.last(), Some(&(4.0, 0.0)));
        assert!(s.contains(&(3.0, 5.0)));
        assert!(!s.contains(&(1.0, 0.1)));
    }
}
