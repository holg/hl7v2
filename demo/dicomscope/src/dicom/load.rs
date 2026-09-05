//! Bytes to a parsed DICOM object.

use crate::error::{error_chain, AppError};
use dicom_encoding::TransferSyntax;
use dicom_object::file::ReadPreamble;
use dicom_object::meta::FileMetaTableBuilder;
use dicom_object::{DefaultDicomObject, InMemDicomObject, OpenFileOptions};
use dicom_transfer_syntax_registry::entries;
use std::io::{Cursor, Read};

const PREAMBLE_LEN: usize = 128;
const MAGIC: &[u8] = b"DICM";

/// Where the `DICM` magic sits, or why the bytes are not a DICOM file.
///
/// Part 10 files carry a 128-byte preamble before the magic; some files in the
/// wild omit the preamble entirely. Both are accepted.
pub fn magic_offset(bytes: &[u8]) -> Result<usize, AppError> {
    if bytes.len() >= PREAMBLE_LEN + MAGIC.len()
        && &bytes[PREAMBLE_LEN..PREAMBLE_LEN + MAGIC.len()] == MAGIC
    {
        return Ok(PREAMBLE_LEN);
    }
    if bytes.len() >= MAGIC.len() && &bytes[..MAGIC.len()] == MAGIC {
        return Ok(0);
    }
    Err(AppError::NotDicom {
        reason: if bytes.len() < PREAMBLE_LEN + MAGIC.len() {
            format!(
                "the file is {} bytes, shorter than the 132-byte Part 10 header, and has no DICM magic",
                bytes.len()
            )
        } else {
            "no DICM magic at offset 128 (after the preamble) or at offset 0".to_string()
        },
    })
}

/// Parse a DICOM Part 10 file from memory. Files without a Part 10 header
/// (a bare data set, as some archives export) are read as a raw data set
/// when they look like one.
pub fn load(bytes: &[u8]) -> Result<DefaultDicomObject, AppError> {
    load_with(bytes, None)
}

/// Like [`load`] but stops before Pixel Data. The browser scans through
/// [`load_header_from`]; this in-memory form serves the host tools and tests.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn load_header(bytes: &[u8]) -> Result<DefaultDicomObject, AppError> {
    load_with(bytes, Some(dicom_dictionary_std::tags::PIXEL_DATA))
}

/// Header-only parse from a stream, consuming only what the header needs.
///
/// With a deflated zip entry this inflates a few kilobytes instead of the
/// whole file, which is what makes scanning a large archive cheap. Files
/// without a Part 10 header fall back to a full read, since the raw data set
/// reader has no stop tag.
pub fn load_header_from<R: std::io::Read>(mut reader: R) -> Result<DefaultDicomObject, AppError> {
    let mut prefix = vec![0u8; PREAMBLE_LEN + MAGIC.len()];
    let mut filled = 0;
    while filled < prefix.len() {
        match reader.read(&mut prefix[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => return Err(AppError::DicomParse(e.to_string())),
        }
    }
    prefix.truncate(filled);
    let offset = match magic_offset(&prefix) {
        Ok(o) => o,
        Err(not_dicom) => {
            return match guess_raw_transfer_syntax(&prefix) {
                Some(ts) => {
                    let mut all = prefix;
                    reader
                        .read_to_end(&mut all)
                        .map_err(|e| AppError::DicomParse(e.to_string()))?;
                    load_raw_dataset(&all, &ts)
                }
                None => Err(not_dicom),
            }
        }
    };
    let rest = Cursor::new(prefix).chain(reader);
    let mut rest = rest;
    if offset > 0 {
        std::io::copy(&mut (&mut rest).take(offset as u64), &mut std::io::sink())
            .map_err(|e| AppError::DicomParse(e.to_string()))?;
    }
    OpenFileOptions::new()
        .read_preamble(ReadPreamble::Never)
        .read_until(dicom_dictionary_std::tags::PIXEL_DATA)
        .from_reader(rest)
        .map_err(|e| AppError::DicomParse(error_chain(&e)))
}

fn load_with(bytes: &[u8], until: Option<dicom_core::Tag>) -> Result<DefaultDicomObject, AppError> {
    let offset = match magic_offset(bytes) {
        Ok(o) => o,
        Err(not_dicom) => {
            return match guess_raw_transfer_syntax(bytes) {
                Some(ts) => load_raw_dataset(bytes, &ts),
                None => Err(not_dicom),
            }
        }
    };
    // dicom-object's reader consumes the DICM magic itself, so the reader is
    // positioned at the magic, not after it, and the preamble is skipped here.
    let mut options = OpenFileOptions::new().read_preamble(ReadPreamble::Never);
    if let Some(tag) = until {
        options = options.read_until(tag);
    }
    options
        .from_reader(Cursor::new(&bytes[offset..]))
        .map_err(|e| AppError::DicomParse(error_chain(&e)))
}

/// A data set with no preamble, magic or meta group. Its transfer syntax is
/// guessed from the first element header and recorded in a synthesised meta
/// group so the rest of the app sees a normal object.
fn load_raw_dataset(bytes: &[u8], ts: &TransferSyntax) -> Result<DefaultDicomObject, AppError> {
    let ds = InMemDicomObject::read_dataset_with_ts(Cursor::new(bytes), ts).map_err(|e| {
        AppError::DicomParse(format!(
            "no Part 10 header; reading as {} failed: {}",
            ts.name(),
            error_chain(&e)
        ))
    })?;
    ds.with_meta(FileMetaTableBuilder::new().transfer_syntax(ts.uid()))
        .map_err(|e| {
            AppError::DicomParse(format!(
                "no Part 10 header and the data set has no SOP identifiers: {}",
                error_chain(&e)
            ))
        })
}

/// Does this look like a raw little-endian data set, and is it explicit VR?
///
/// The first element of a data set is in group 0008 (or another small even
/// group), so byte 1 is zero for little endian. Explicit VR puts two
/// uppercase ASCII letters at bytes 4 and 5; implicit VR has a length there.
pub fn guess_raw_transfer_syntax(bytes: &[u8]) -> Option<TransferSyntax> {
    if bytes.len() < 8 || bytes[1] != 0 || bytes[0] % 2 != 0 {
        return None;
    }
    let explicit = bytes[4].is_ascii_uppercase() && bytes[5].is_ascii_uppercase();
    Some(if explicit {
        entries::EXPLICIT_VR_LITTLE_ENDIAN.erased()
    } else {
        entries::IMPLICIT_VR_LITTLE_ENDIAN.erased()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_detection() {
        let mut with_preamble = vec![0u8; 128];
        with_preamble.extend_from_slice(b"DICM");
        assert_eq!(magic_offset(&with_preamble).unwrap(), 128);
        assert_eq!(magic_offset(b"DICM\x02\x00").unwrap(), 0);
        assert!(matches!(
            magic_offset(b"MSH|^~\\&|"),
            Err(AppError::NotDicom { .. })
        ));
        let text = vec![b'x'; 200];
        let err = magic_offset(&text).unwrap_err().to_string();
        assert!(err.contains("offset 128"), "{err}");
        let short = magic_offset(&[0u8; 10]).unwrap_err().to_string();
        assert!(short.contains("10 bytes"), "{short}");
    }

    #[test]
    fn raw_dataset_guess() {
        // (0008,0016) UI, explicit VR LE
        assert_eq!(
            guess_raw_transfer_syntax(b"\x08\x00\x16\x00UI\x1a\x00").map(|t| t.uid()),
            Some("1.2.840.10008.1.2.1")
        );
        // (0008,0016), implicit VR LE (length follows tag)
        assert_eq!(
            guess_raw_transfer_syntax(b"\x08\x00\x16\x00\x1a\x00\x00\x00").map(|t| t.uid()),
            Some("1.2.840.10008.1.2")
        );
        assert!(guess_raw_transfer_syntax(b"MSH|^~\\&|").is_none());
        assert!(
            guess_raw_transfer_syntax(b"\x00\x08\x00\x16UI\x00\x1a").is_none(),
            "big endian is not guessed"
        );
        assert!(guess_raw_transfer_syntax(&[0u8; 4]).is_none());
    }

    #[test]
    fn garbage_after_magic_is_a_parse_error_not_a_panic() {
        let mut bytes = vec![0u8; 128];
        bytes.extend_from_slice(b"DICM");
        bytes.extend_from_slice(&[0xFF; 40]);
        assert!(matches!(load(&bytes), Err(AppError::DicomParse(_))));
    }
}
