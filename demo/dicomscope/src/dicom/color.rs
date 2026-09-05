//! Colour: YBR to RGB and palette lookup. Pure functions, host-tested.

use dicom_core::PrimitiveValue;
use dicom_core::Tag;
use dicom_dictionary_std::tags;
use dicom_object::DefaultDicomObject;

/// YBR_FULL to RGB per PS3.3 C.7.6.3.1.2 (full-range JFIF constants).
pub fn ybr_to_rgb(y: u8, cb: u8, cr: u8) -> [u8; 3] {
    let (y, cb, cr) = (y as f32, cb as f32 - 128.0, cr as f32 - 128.0);
    let r = y + 1.402 * cr;
    let g = y - 0.344_136 * cb - 0.714_136 * cr;
    let b = y + 1.772 * cb;
    [clamp8(r), clamp8(g), clamp8(b)]
}

fn clamp8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Interleaved 8-bit samples (3 per pixel) to RGBA. `ybr` converts colour
/// space on the way. Also accepts true 4:2:2 subsampled data (2 bytes per
/// pixel: Y0 Y1 Cb Cr per pixel pair) as some uncompressed YBR_FULL_422
/// files store it.
pub fn samples_to_rgba(samples: &[u8], pixels: usize, ybr: bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(pixels * 4);
    if samples.len() >= pixels * 3 {
        for px in samples[..pixels * 3].chunks_exact(3) {
            let rgb = if ybr {
                ybr_to_rgb(px[0], px[1], px[2])
            } else {
                [px[0], px[1], px[2]]
            };
            out.extend_from_slice(&rgb);
            out.push(255);
        }
        Some(out)
    } else if ybr && samples.len() >= pixels * 2 {
        for quad in samples[..pixels * 2].chunks_exact(4) {
            for y in [quad[0], quad[1]] {
                out.extend_from_slice(&ybr_to_rgb(y, quad[2], quad[3]));
                out.push(255);
            }
        }
        Some(out)
    } else {
        None
    }
}

/// One channel of a palette: 8-bit output values indexed from `first_mapped`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteChannel {
    pub first_mapped: i32,
    pub values: Vec<u8>,
}

impl PaletteChannel {
    /// Build from the LUT descriptor (entries, first mapped, bits per entry)
    /// and the LUT data element value. 16-bit entries keep their high byte.
    pub fn new(descriptor: [i32; 3], data: &PrimitiveValue) -> Result<PaletteChannel, String> {
        let [entries, first_mapped, bits] = descriptor;
        let entries = if entries == 0 {
            65536
        } else {
            entries.max(0) as usize
        };
        let words: Vec<u16> = match data {
            PrimitiveValue::U16(v) => v.to_vec(),
            PrimitiveValue::U8(b) => match bits {
                8 => b.iter().map(|&x| u16::from(x)).collect(),
                _ => b
                    .chunks_exact(2)
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect(),
            },
            other => return Err(format!("palette data has unexpected type {other:?}")),
        };
        let values: Vec<u8> = match bits {
            8 if matches!(data, PrimitiveValue::U16(_)) => {
                // Two 8-bit entries packed per 16-bit word, low byte first.
                words.iter().flat_map(|w| w.to_le_bytes()).collect()
            }
            8 => words.iter().map(|&w| w as u8).collect(),
            16 => words.iter().map(|&w| (w >> 8) as u8).collect(),
            other => return Err(format!("palette entries of {other} bits are not supported")),
        };
        if values.len() < entries {
            return Err(format!(
                "palette declares {entries} entries but carries {}",
                values.len()
            ));
        }
        Ok(PaletteChannel {
            first_mapped,
            values: values[..entries].to_vec(),
        })
    }

    pub fn lookup(&self, index: u16) -> u8 {
        let i = (i32::from(index) - self.first_mapped).clamp(0, self.values.len() as i32 - 1);
        self.values.get(i as usize).copied().unwrap_or(0)
    }
}

pub struct Palette {
    pub red: PaletteChannel,
    pub green: PaletteChannel,
    pub blue: PaletteChannel,
}

impl Palette {
    /// Read (0028,1101..1103) and (0028,1201..1203). Segmented palettes
    /// (0028,1221..1223) are not supported and say so.
    pub fn from_object(obj: &DefaultDicomObject) -> Result<Palette, String> {
        if obj
            .get(tags::SEGMENTED_RED_PALETTE_COLOR_LOOKUP_TABLE_DATA)
            .is_some()
            && obj.get(tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA).is_none()
        {
            return Err("segmented palette colour lookup tables are not supported".into());
        }
        Ok(Palette {
            red: channel(
                obj,
                tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
                tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                "red",
            )?,
            green: channel(
                obj,
                tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
                tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                "green",
            )?,
            blue: channel(
                obj,
                tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
                tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                "blue",
            )?,
        })
    }

    pub fn apply(&self, indices: &[u16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(indices.len() * 4);
        for &i in indices {
            out.extend_from_slice(&[
                self.red.lookup(i),
                self.green.lookup(i),
                self.blue.lookup(i),
                255,
            ]);
        }
        out
    }
}

fn channel(
    obj: &DefaultDicomObject,
    desc: Tag,
    data: Tag,
    name: &str,
) -> Result<PaletteChannel, String> {
    let d = obj
        .get(desc)
        .ok_or_else(|| format!("{name} palette descriptor {desc} is missing"))?;
    let values: Vec<i32> = match d.value().primitive() {
        Some(PrimitiveValue::U16(v)) => v.iter().map(|&x| i32::from(x)).collect(),
        Some(PrimitiveValue::I16(v)) => v.iter().map(|&x| i32::from(x)).collect(),
        Some(p) => p
            .to_multi_str()
            .iter()
            .filter_map(|s| s.trim().parse::<i32>().ok())
            .collect(),
        None => Vec::new(),
    };
    if values.len() != 3 {
        return Err(format!(
            "{name} palette descriptor {desc} should have 3 values, has {}",
            values.len()
        ));
    }
    let data_el = obj
        .get(data)
        .ok_or_else(|| format!("{name} palette data {data} is missing"))?;
    let prim = data_el
        .value()
        .primitive()
        .ok_or_else(|| format!("{name} palette data {data} is not a primitive value"))?;
    PaletteChannel::new([values[0], values[1], values[2]], prim)
        .map_err(|e| format!("{name} palette: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ybr_grey_and_primaries() {
        assert_eq!(ybr_to_rgb(128, 128, 128), [128, 128, 128]);
        assert_eq!(ybr_to_rgb(0, 128, 128), [0, 0, 0]);
        assert_eq!(ybr_to_rgb(255, 128, 128), [255, 255, 255]);
        // Pure red in JFIF YCbCr is (76, 85, 255).
        let [r, g, b] = ybr_to_rgb(76, 85, 255);
        assert!(r >= 253 && g <= 2 && b <= 2, "{r} {g} {b}");
    }

    #[test]
    fn samples_interleaved_and_subsampled() {
        assert_eq!(
            samples_to_rgba(&[1, 2, 3, 4, 5, 6], 2, false),
            Some(vec![1, 2, 3, 255, 4, 5, 6, 255])
        );
        let sub = samples_to_rgba(&[128, 0, 128, 128], 2, true).unwrap();
        assert_eq!(sub, vec![128, 128, 128, 255, 0, 0, 0, 255]);
        assert_eq!(samples_to_rgba(&[1, 2], 2, false), None);
    }

    #[test]
    fn palette_channel_formats() {
        let words = PrimitiveValue::U16(vec![0x0000u16, 0x8000, 0xFFFF].into());
        let c = PaletteChannel::new([3, 0, 16], &words).unwrap();
        assert_eq!(c.values, vec![0, 128, 255]);
        let packed = PrimitiveValue::U16(vec![0x2010u16, 0x0030].into());
        let c = PaletteChannel::new([3, 0, 8], &packed).unwrap();
        assert_eq!(c.values, vec![0x10, 0x20, 0x30]);
        let bytes = PrimitiveValue::U8(vec![1u8, 2, 3].into());
        let c = PaletteChannel::new([3, 0, 8], &bytes).unwrap();
        assert_eq!(c.values, vec![1, 2, 3]);
        assert!(PaletteChannel::new([4, 0, 8], &bytes).is_err());
        assert!(PaletteChannel::new([3, 0, 12], &bytes).is_err());
    }

    #[test]
    fn lookup_respects_first_mapped_and_clamps() {
        let c = PaletteChannel {
            first_mapped: 100,
            values: vec![10, 20, 30],
        };
        assert_eq!(c.lookup(99), 10);
        assert_eq!(c.lookup(100), 10);
        assert_eq!(c.lookup(101), 20);
        assert_eq!(c.lookup(500), 30);
        let p = Palette {
            red: c.clone(),
            green: c.clone(),
            blue: c,
        };
        assert_eq!(p.apply(&[101]), vec![20, 20, 20, 255]);
    }
}
