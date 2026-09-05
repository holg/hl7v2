//! Decode frame 0, apply the modality rescale, and report what the shader
//! needs. Greyscale values are stored as `f32` so unsigned 16-bit data and
//! fractional rescale slopes survive without saturation; they go to an
//! `R32Float` texture, which every WebGPU adapter supports for `textureLoad`.
//! Colour (RGB, YBR, palette) is converted to RGBA8 here and windowed by
//! nothing: it is shown as stored.

use crate::dicom::color::{samples_to_rgba, Palette};
use crate::dicom::study::{first_float, first_number};
use crate::dicom::transfer_syntax;
use crate::error::{error_chain, AppError};
use crate::measure::Spacing;
use dicom_dictionary_std::tags;
use dicom_object::DefaultDicomObject;
use dicom_pixeldata::{
    ConvertOptions, ModalityLutOption, PhotometricInterpretation, PixelDecoder, VoiLutOption,
};

/// Decoded samples, row-major, `width * height` pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum Pixels {
    /// Values in modality units (e.g. HU) after rescale, before windowing.
    Gray(Vec<f32>),
    /// 8-bit RGBA, alpha always 255.
    Rgba(Vec<u8>),
}

impl Pixels {
    pub fn is_color(&self) -> bool {
        matches!(self, Pixels::Rgba(_))
    }
}

/// One decoded frame after rescale, before windowing.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    /// (center, width) from (0028,1050)/(0028,1051), first value of each.
    pub default_window: Option<(f32, f32)>,
    /// Min and max of `data`, for fallback windowing and slider ranges.
    pub value_range: (f32, f32),
    /// MONOCHROME1: low values are bright. Applied in the shader, never here.
    pub inverted: bool,
    pub bits_stored: u16,
    pub photometric: String,
    /// (slope, intercept) that was applied.
    pub rescale: (f32, f32),
    /// Which frame this is (zero-based) and how many the file has.
    pub frame_index: u32,
    pub frame_count: u32,
    /// Physical pixel size, when the file says.
    pub spacing: Option<Spacing>,
    /// Milliseconds per frame for cine, from Frame Time or Cine Rate.
    pub frame_time_ms: Option<f32>,
}

/// Everything about a frame except the pixels, for reactive UI state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameInfo {
    pub width: u32,
    pub height: u32,
    pub color: bool,
    pub default_window: Option<(f32, f32)>,
    pub value_range: (f32, f32),
    pub inverted: bool,
    pub bits_stored: u16,
    pub frame_index: u32,
    pub frame_count: u32,
    /// Physical pixel size, when the file says.
    pub spacing: Option<Spacing>,
    /// Milliseconds per frame for cine, from Frame Time or Cine Rate.
    pub frame_time_ms: Option<f32>,
}

impl From<&Frame> for FrameInfo {
    fn from(f: &Frame) -> Self {
        FrameInfo {
            width: f.width,
            height: f.height,
            color: f.pixels.is_color(),
            default_window: f.default_window,
            value_range: f.value_range,
            inverted: f.inverted,
            bits_stored: f.bits_stored,
            frame_index: f.frame_index,
            frame_count: f.frame_count,
            spacing: f.spacing,
            frame_time_ms: f.frame_time_ms,
        }
    }
}

/// Decode one frame (zero-based) of a possibly multi-frame instance.
pub fn decode_frame(obj: &DefaultDicomObject, frame: u32) -> Result<Frame, AppError> {
    let ts = obj
        .meta()
        .transfer_syntax()
        .trim_end_matches('\0')
        .trim()
        .to_string();
    // Fail early with a useful message rather than a generic decoder error.
    if let Some((name, reason)) = transfer_syntax::unsupported(&ts) {
        return Err(AppError::UnsupportedTransferSyntax {
            uid: ts,
            name: name.to_string(),
            reason: reason.to_string(),
        });
    }

    // One frame at a time: decoding every frame of a multi-frame file wastes
    // memory, and only one is shown.
    let decoded = obj
        .decode_pixel_data_frame(frame)
        .map_err(|e| AppError::Decode(error_chain(&e)))?;

    let pi = decoded.photometric_interpretation().clone();
    let spp = decoded.samples_per_pixel();
    let width = decoded.columns();
    let height = decoded.rows();
    let count = width as usize * height as usize;

    // Raw stored values: no modality LUT, no VOI LUT. Both are applied
    // explicitly below (rescale) and in the shader (window).
    let options = ConvertOptions::new()
        .with_modality_lut(ModalityLutOption::None)
        .with_voi_lut(VoiLutOption::Identity);

    let (pixels, inverted, default_window) = match (spp, &pi) {
        (1, PhotometricInterpretation::Monochrome1 | PhotometricInterpretation::Monochrome2) => {
            let mut data: Vec<f32> = decoded
                .to_vec_with_options(&options)
                .map_err(|e| AppError::Decode(error_chain(&e)))?;
            check_len(data.len(), count, width, height)?;
            data.truncate(count);
            let slope = first_float(obj, tags::RESCALE_SLOPE).unwrap_or(1.0);
            let intercept = first_float(obj, tags::RESCALE_INTERCEPT).unwrap_or(0.0);
            apply_rescale(&mut data, slope, intercept);
            let window = window_from(
                first_float(obj, tags::WINDOW_CENTER),
                first_float(obj, tags::WINDOW_WIDTH),
            );
            return finish(
                obj,
                frame,
                Pixels::Gray(data),
                width,
                height,
                pi == PhotometricInterpretation::Monochrome1,
                window,
                (slope, intercept),
                decoded.bits_stored(),
                pi.to_string(),
            );
        }
        (1, PhotometricInterpretation::PaletteColor) => {
            let palette = Palette::from_object(obj).map_err(AppError::Decode)?;
            let indices: Vec<u16> = decoded
                .to_vec_with_options(&options)
                .map_err(|e| AppError::Decode(error_chain(&e)))?;
            check_len(indices.len(), count, width, height)?;
            (Pixels::Rgba(palette.apply(&indices[..count])), false, None)
        }
        (
            3,
            PhotometricInterpretation::Rgb
            | PhotometricInterpretation::YbrFull
            | PhotometricInterpretation::YbrFull422,
        ) => {
            let ybr = pi != PhotometricInterpretation::Rgb;
            let samples: Vec<u8> = if decoded.bits_allocated() > 8 {
                let wide: Vec<u16> = decoded
                    .to_vec_with_options(&options)
                    .map_err(|e| AppError::Decode(error_chain(&e)))?;
                wide.iter().map(|&s| (s >> 8) as u8).collect()
            } else {
                decoded
                    .to_vec_with_options(&options)
                    .map_err(|e| AppError::Decode(error_chain(&e)))?
            };
            let rgba = samples_to_rgba(&samples, count, ybr).ok_or_else(|| {
                AppError::Decode(format!(
                    "decoder returned {} samples for a {}x{} {} frame",
                    samples.len(),
                    width,
                    height,
                    pi
                ))
            })?;
            (Pixels::Rgba(rgba), false, None)
        }
        _ => {
            return Err(AppError::UnsupportedPhotometric {
                photometric: format!("{pi} with {spp} sample(s) per pixel"),
            })
        }
    };
    finish(
        obj,
        frame,
        pixels,
        width,
        height,
        inverted,
        default_window,
        (1.0, 0.0),
        decoded.bits_stored(),
        pi.to_string(),
    )
}

fn check_len(got: usize, expected: usize, width: u32, height: u32) -> Result<(), AppError> {
    if got < expected {
        return Err(AppError::Decode(format!(
            "decoder returned {got} samples for a {width}x{height} frame"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish(
    obj: &DefaultDicomObject,
    frame_index: u32,
    pixels: Pixels,
    width: u32,
    height: u32,
    inverted: bool,
    default_window: Option<(f32, f32)>,
    rescale: (f32, f32),
    bits_stored: u16,
    photometric: String,
) -> Result<Frame, AppError> {
    let value_range = match &pixels {
        Pixels::Gray(d) => value_range(d),
        Pixels::Rgba(_) => (0.0, 255.0),
    };
    // Pixel Spacing is in the patient; Imager Pixel Spacing is at the
    // detector and only a fallback, flagged as such in labels.
    let spacing = obj
        .get(tags::PIXEL_SPACING)
        .and_then(|e| e.to_str().ok())
        .and_then(|s| Spacing::parse(&s, false))
        .or_else(|| {
            obj.get(tags::IMAGER_PIXEL_SPACING)
                .and_then(|e| e.to_str().ok())
                .and_then(|s| Spacing::parse(&s, true))
        });
    // Frame Time (ms) wins; Cine Rate and Recommended Display Frame Rate
    // are frames per second.
    let frame_time_ms = first_float(obj, tags::FRAME_TIME)
        .filter(|t| *t > 0.0)
        .or_else(|| {
            first_float(obj, tags::CINE_RATE)
                .or_else(|| first_float(obj, tags::RECOMMENDED_DISPLAY_FRAME_RATE))
                .filter(|r| *r > 0.0)
                .map(|r| 1000.0 / r)
        });
    let frames_total = obj
        .get(tags::NUMBER_OF_FRAMES)
        .and_then(|e| e.to_str().ok())
        .and_then(|s| first_number(&s))
        .map(|n| n.max(1.0) as u32)
        .unwrap_or(1);

    Ok(Frame {
        width,
        height,
        pixels,
        default_window,
        value_range,
        inverted,
        bits_stored,
        photometric,
        rescale,
        frame_index,
        frame_count: frames_total,
        spacing,
        frame_time_ms,
    })
}

/// Rescale Slope / Intercept (0028,1053) / (0028,1052): stored value to
/// modality units.
pub fn apply_rescale(data: &mut [f32], slope: f32, intercept: f32) {
    if slope == 1.0 && intercept == 0.0 {
        return;
    }
    for v in data.iter_mut() {
        *v = *v * slope + intercept;
    }
}

/// Min and max, ignoring NaN. `(0, 0)` for an empty slice.
pub fn value_range(data: &[f32]) -> (f32, f32) {
    let mut it = data.iter().copied().filter(|v| !v.is_nan());
    let Some(first) = it.next() else {
        return (0.0, 0.0);
    };
    it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

/// A usable (center, width) only when both attributes are present and the
/// width is positive.
pub fn window_from(center: Option<f32>, width: Option<f32>) -> Option<(f32, f32)> {
    match (center, width) {
        (Some(c), Some(w)) if w > 0.0 && c.is_finite() && w.is_finite() => Some((c, w)),
        _ => None,
    }
}

/// Window covering the whole value range, for files without (0028,1050).
pub fn fallback_window((min, max): (f32, f32)) -> (f32, f32) {
    let width = (max - min).max(1.0);
    ((min + max) / 2.0, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescale_applies_slope_and_intercept() {
        let mut d = vec![0.0, 1024.0, 2048.0];
        apply_rescale(&mut d, 1.0, -1024.0);
        assert_eq!(d, vec![-1024.0, 0.0, 1024.0]);
        let mut d = vec![10.0];
        apply_rescale(&mut d, 0.5, 2.0);
        assert_eq!(d, vec![7.0]);
    }

    #[test]
    fn rescale_defaults_are_identity() {
        let mut d = vec![3.0, 65535.0];
        apply_rescale(&mut d, 1.0, 0.0);
        assert_eq!(d, vec![3.0, 65535.0]);
    }

    #[test]
    fn range_and_fallback() {
        assert_eq!(value_range(&[5.0, -3.0, f32::NAN, 9.0]), (-3.0, 9.0));
        assert_eq!(value_range(&[]), (0.0, 0.0));
        assert_eq!(fallback_window((-1000.0, 3000.0)), (1000.0, 4000.0));
        assert_eq!(fallback_window((7.0, 7.0)), (7.0, 1.0));
    }

    #[test]
    fn window_requires_both_values() {
        assert_eq!(window_from(Some(40.0), Some(400.0)), Some((40.0, 400.0)));
        assert_eq!(window_from(Some(40.0), None), None);
        assert_eq!(window_from(None, Some(400.0)), None);
        assert_eq!(window_from(Some(40.0), Some(0.0)), None);
        assert_eq!(window_from(Some(f32::NAN), Some(1.0)), None);
    }
}
