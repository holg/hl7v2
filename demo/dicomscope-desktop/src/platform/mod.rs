//! What differs between a desktop and an iPad: where files come from and
//! how a file is handed to another program. Everything else is shared.

use std::path::PathBuf;

#[cfg(target_os = "ios")]
mod ios;

/// Files to open at start when none were given on the command line.
/// Desktop: none. iOS: everything in the app's Documents folder, which the
/// user fills from Finder or the Files app.
pub fn initial_paths() -> Vec<String> {
    let entries = folder_entries();
    let newest = |order: bool| {
        entries
            .iter()
            .filter(|e| e.is_order == order)
            .max_by_key(|e| e.modified)
            .map(|e| e.path.clone())
    };
    newest(false).into_iter().chain(newest(true)).collect()
}

/// Files that arrived through "Open in dicomscope" from another app: the
/// Inbox folder only. The Documents folder itself is the user's, browsed
/// through the Files menu, never opened behind their back.
pub fn inbox_paths() -> Vec<String> {
    folder_entries()
        .into_iter()
        .filter(|e| e.path.contains("/Inbox/"))
        .map(|e| e.path)
        .collect()
}

/// One entry of the platform's document folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderEntry {
    pub path: String,
    pub name: String,
    /// `.hl7` or `.txt`: an order rather than a study.
    pub is_order: bool,
    pub modified: Option<std::time::SystemTime>,
    pub size: u64,
}

/// What the document folder holds: Documents, which Finder and the Files
/// app show, and Documents/Inbox, where "Open in dicomscope" from another
/// app puts a copy. Empty on the desktop.
pub fn folder_entries() -> Vec<FolderEntry> {
    let Some(docs) = documents_dir() else {
        return Vec::new();
    };
    let mut v = Vec::new();
    for dir in [docs.clone(), docs.join("Inbox")] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for p in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.starts_with('.') || name == "Inbox" {
                continue;
            }
            let meta = std::fs::metadata(&p).ok();
            let lower = name.to_ascii_lowercase();
            v.push(FolderEntry {
                path: p.display().to_string(),
                is_order: lower.ends_with(".hl7") || lower.ends_with(".txt"),
                modified: meta.as_ref().and_then(|m| m.modified().ok()),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                name,
            });
        }
    }
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
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
        ios::present(ios::Pick::Study);
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
        let _ = (title, filter_name);
        let kind = if extensions.iter().any(|e| e.eq_ignore_ascii_case("hl7")) {
            ios::Pick::Order
        } else {
            ios::Pick::Study
        };
        ios::present(kind);
        None
    }
}

/// Files a picker delivered since the last call (iOS; the desktop pickers
/// return their result directly).
/// Whether a picker result is waiting (iOS).
pub fn has_picked() -> bool {
    #[cfg(target_os = "ios")]
    {
        ios::has_picked()
    }
    #[cfg(not(target_os = "ios"))]
    {
        false
    }
}

pub fn take_picked() -> Vec<String> {
    #[cfg(target_os = "ios")]
    {
        ios::take_picked()
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }
    #[cfg(not(target_os = "ios"))]
    {
        Vec::new()
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
        Some("Open picks from Files; or copy studies and .hl7 orders into the dicomscope folder (Finder file sharing), then tap Reload.")
    } else {
        None
    }
}
