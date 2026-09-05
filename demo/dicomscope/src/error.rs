//! One error type, surfaced in the banner.

use std::fmt;

/// Every failure the app can show. `Display` is what the user reads.
#[derive(Debug, Clone, PartialEq)]
pub enum AppError {
    /// The bytes are not a DICOM Part 10 file.
    NotDicom { reason: String },
    /// dicom-rs could not parse the data set.
    DicomParse(String),
    /// The pixel data is compressed with a transfer syntax this build cannot decode.
    UnsupportedTransferSyntax {
        uid: String,
        name: String,
        reason: String,
    },
    /// A photometric interpretation this build cannot render.
    UnsupportedPhotometric { photometric: String },
    /// Pixel decoding failed for a supported transfer syntax.
    Decode(String),
    /// The HL7 message could not be parsed.
    Hl7(hl7kit::ParseError),
    /// No WebGPU adapter was available.
    NoWebGpu(String),
    /// Any other GPU failure.
    Gpu(String),
    /// The browser failed to read a file.
    FileRead(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::NotDicom { reason } => write!(f, "Not a DICOM file: {reason}."),
            AppError::DicomParse(e) => write!(f, "DICOM parse error: {e}."),
            AppError::UnsupportedTransferSyntax { uid, name, reason } => write!(
                f,
                "Transfer syntax {uid} ({name}) is not supported in the browser build: {reason}."
            ),
            AppError::UnsupportedPhotometric { photometric } => write!(
                f,
                "Photometric Interpretation {photometric} is not supported. Supported: MONOCHROME1, \
                 MONOCHROME2, RGB, YBR_FULL, YBR_FULL_422 and PALETTE COLOR."
            ),
            AppError::Decode(e) => write!(f, "Pixel data could not be decoded: {e}."),
            AppError::Hl7(e) => write!(f, "Malformed HL7 message: {e}."),
            AppError::NoWebGpu(e) => write!(
                f,
                "WebGPU is not available in this browser, so the image cannot be rendered ({e}). \
                 Chrome, Edge and Safari 26 support it; Firefox needs it enabled."
            ),
            AppError::Gpu(e) => write!(f, "GPU error: {e}."),
            AppError::FileRead(e) => write!(f, "The browser could not read the file: {e}."),
        }
    }
}

impl std::error::Error for AppError {}

impl From<hl7kit::ParseError> for AppError {
    fn from(e: hl7kit::ParseError) -> Self {
        AppError::Hl7(e)
    }
}

/// Render an error and every `source()` beneath it on one line.
pub fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(s) = cur {
        out.push_str(": ");
        out.push_str(&s.to_string());
        cur = s.source();
    }
    out
}
