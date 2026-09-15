//! What differs between a desktop and an iPad: where files come from and
//! how a file is handed to another program. Everything else is shared.

use std::path::PathBuf;

/// Files to open at start when none were given on the command line.
/// Desktop: none. iOS: everything in the app's Documents folder, which the
/// user fills from Finder or the Files app.
pub fn initial_paths() -> Vec<String> {
    #[cfg(target_os = "ios")]
    {
        documents_dir()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|rd| {
                let mut v: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        !p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                    })
                    .map(|p| p.display().to_string())
                    .collect();
                v.sort();
                v
            })
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "ios"))]
    {
        Vec::new()
    }
}

/// The app's Documents folder on iOS; `None` elsewhere.
pub fn documents_dir() -> Option<PathBuf> {
    if cfg!(target_os = "ios") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents"))
    } else {
        None
    }
}

/// Native folder picker; `None` when cancelled or unavailable.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(not(target_os = "ios"))]
    {
        rfd::FileDialog::new().set_title(title).pick_folder()
    }
    #[cfg(target_os = "ios")]
    {
        let _ = title;
        None
    }
}

/// Native file picker with extension filters; `None` when cancelled or
/// unavailable.
pub fn pick_file(title: &str, filter_name: &str, extensions: &[&str]) -> Option<PathBuf> {
    #[cfg(not(target_os = "ios"))]
    {
        rfd::FileDialog::new()
            .set_title(title)
            .add_filter(filter_name, extensions)
            .add_filter("All files", &["*"])
            .pick_file()
    }
    #[cfg(target_os = "ios")]
    {
        let _ = (title, filter_name, extensions);
        None
    }
}

/// Where to save a file named `name`: a save dialog on the desktop, the
/// Documents folder on iOS (visible in Finder and the Files app).
pub fn save_target(name: &str) -> Option<PathBuf> {
    #[cfg(not(target_os = "ios"))]
    {
        rfd::FileDialog::new().set_file_name(name).save_file()
    }
    #[cfg(target_os = "ios")]
    {
        documents_dir().map(|d| d.join(name))
    }
}

/// Hand a file to the system's default application for it.
pub fn open_external(path: &std::path::Path) -> Result<(), String> {
    #[cfg(not(target_os = "ios"))]
    {
        open::that(path).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "ios")]
    {
        Err(format!(
            "saved to {}; open it from the Files app",
            path.display()
        ))
    }
}

/// Why the pickers are unavailable, for the status line.
pub fn picker_note() -> Option<&'static str> {
    if cfg!(target_os = "ios") {
        Some("On iPad: copy studies and .hl7 orders into the dicomscope folder (Finder file sharing or the Files app), then tap Reload.")
    } else {
        None
    }
}
