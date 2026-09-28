//! dicomscope as a native application: a winit window, a native wgpu
//! surface, egui panels, and the same `dicomscope-core` code the browser
//! runs. No WebGPU, no webview, so it runs where those do not: Linux LTS
//! desktops, and iPads through the Metal backend.
//!
//! The frame is drawn in two passes on one surface: the core renderer paints
//! the image into the central area, then egui paints the panels over it.

// `deny`, not `forbid`: the bundle entry point below needs `no_mangle`, which
// the lint counts as unsafe. Nothing else in the crate is allowed to.
#![deny(unsafe_code)]
#![warn(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod app;
pub mod platform;
/// The session lives in the core, shared with the terminal app.
mod session {
    pub use dicomscope_core::session::*;
}
mod ui;

pub use app::run;

/// Entry point for the iOS and macOS bundles in the Xcode project: a C
/// `main` calls this and never returns. On iOS files come from the app's
/// Documents folder (Finder file sharing); on macOS nothing opens at start,
/// since Xcode and Finder put their own flags into the arguments.
#[cfg(any(target_os = "ios", target_os = "macos"))]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn dicomscope_main() {
    run(platform::initial_paths());
}
