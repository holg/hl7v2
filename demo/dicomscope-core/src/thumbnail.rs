//! Small RGBA previews of a frame for the series list. Host-tested.

use crate::dicom::{fallback_window, Frame, Pixels};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, RGBA, alpha 255.
    pub rgba: Vec<u8>,
}

/// Downsample a frame to at most `max` pixels on its longer side, applying
/// the file's window (or a full-range fallback) and MONOCHROME1 inversion.
/// Box-averaged so thin structures do not alias away.
pub fn thumbnail(frame: &Frame, max: u32) -> Thumbnail {
    let (w, h) = (frame.width.max(1), frame.height.max(1));
    let step = w.max(h).div_ceil(max.max(1)).max(1);
    let tw = w.div_ceil(step);
    let th = h.div_ceil(step);
    let mut rgba = Vec::with_capacity((tw * th * 4) as usize);
    let (center, width) = frame
        .default_window
        .unwrap_or_else(|| fallback_window(frame.value_range));
    let lo = center - 0.5 - (width - 1.0) / 2.0;
    let span = (width - 1.0).max(1e-3);

    for ty in 0..th {
        for tx in 0..tw {
            let x0 = tx * step;
            let y0 = ty * step;
            let x1 = (x0 + step).min(w);
            let y1 = (y0 + step).min(h);
            let n = ((x1 - x0) * (y1 - y0)) as f32;
            match &frame.pixels {
                Pixels::Gray(data) => {
                    let mut sum = 0.0f32;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            sum += data.get((y * w + x) as usize).copied().unwrap_or(0.0);
                        }
                    }
                    let mut g = ((sum / n - lo) / span).clamp(0.0, 1.0);
                    if frame.inverted {
                        g = 1.0 - g;
                    }
                    let v = (g * 255.0).round() as u8;
                    rgba.extend_from_slice(&[v, v, v, 255]);
                }
                Pixels::Rgba(data) => {
                    let mut acc = [0u32; 3];
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let i = ((y * w + x) * 4) as usize;
                            for (c, a) in acc.iter_mut().enumerate() {
                                *a += u32::from(data.get(i + c).copied().unwrap_or(0));
                            }
                        }
                    }
                    let n = n as u32;
                    rgba.extend_from_slice(&[
                        (acc[0] / n) as u8,
                        (acc[1] / n) as u8,
                        (acc[2] / n) as u8,
                        255,
                    ]);
                }
            }
        }
    }
    Thumbnail {
        width: tw,
        height: th,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(w: u32, h: u32, data: Vec<f32>, window: Option<(f32, f32)>, inverted: bool) -> Frame {
        Frame {
            width: w,
            height: h,
            pixels: Pixels::Gray(data),
            default_window: window,
            value_range: (0.0, 100.0),
            inverted,
            bits_stored: 16,
            photometric: "MONOCHROME2".into(),
            rescale: (1.0, 0.0),
            frame_index: 0,
            frame_count: 1,
            spacing: None,
            frame_time_ms: None,
        }
    }

    #[test]
    fn downsample_and_window() {
        // 4x2 image, window 50/100 -> 0 is black, 100 is white.
        let f = gray(
            4,
            2,
            vec![0.0, 0.0, 100.0, 100.0, 0.0, 0.0, 100.0, 100.0],
            Some((50.5, 100.0)),
            false,
        );
        let t = thumbnail(&f, 2);
        assert_eq!((t.width, t.height), (2, 1));
        assert_eq!(&t.rgba[..4], &[0, 0, 0, 255]);
        assert_eq!(&t.rgba[4..8], &[255, 255, 255, 255]);
        let inv = thumbnail(&gray(4, 2, vec![0.0; 8], Some((50.5, 100.0)), true), 2);
        assert_eq!(&inv.rgba[..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn colour_averages_and_small_images_are_not_upscaled() {
        let f = Frame {
            pixels: Pixels::Rgba(vec![255, 0, 0, 255, 0, 0, 255, 255]),
            ..gray(2, 1, vec![], None, false)
        };
        let t = thumbnail(&f, 1);
        assert_eq!((t.width, t.height), (1, 1));
        assert_eq!(&t.rgba[..4], &[127, 0, 127, 255]);
        let same = thumbnail(&gray(3, 2, vec![1.0; 6], None, false), 64);
        assert_eq!((same.width, same.height), (3, 2));
    }
}
