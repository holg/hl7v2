//! Everything dicomscope does that is not a user interface: DICOM loading,
//! series scanning and pixel decoding, the HL7 order linkage, the FHIR R4
//! output, the Modality Worklist item, the wgpu renderer and the view
//! geometry. The browser app and the desktop app are thin shells over this
//! crate, so the same code path is tested on the host, shown in the browser
//! and shipped as a native binary.
//!
//! No `unwrap()` on user data, no network I/O anywhere.

#![forbid(unsafe_code)]
#![warn(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod annotations;
pub mod dicom;
pub mod error;
pub mod fhir;
#[cfg(not(target_arch = "wasm32"))]
pub mod fs;
pub mod link;
pub mod measure;
pub mod nerve;
pub mod nervefind;
pub mod order_gen;
pub mod render;
pub mod thumbnail;
pub mod view;
pub mod worklist;

pub use error::AppError;
