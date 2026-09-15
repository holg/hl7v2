//! Reading studies from the file system: folders, zips and single files.
//! Host only; the browser gets its bytes from the File API instead.

use crate::dicom::{self, FileEntry};

/// The first DICOM instance in a folder, zip or single file, read without
/// loading the rest: enough for an order or a header check.
pub fn first_instance(path: &str) -> Result<(String, Vec<u8>), String> {
    let root = std::path::Path::new(path);
    if root.is_dir() {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut children: Vec<_> = std::fs::read_dir(&dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect();
            children.sort();
            children.reverse(); // so the stack pops in sorted order
            for child in children {
                if child.is_dir() {
                    stack.push(child);
                } else if let Ok(bytes) = std::fs::read(&child) {
                    if dicom::load::load_header(&bytes).is_ok() {
                        return Ok((child.display().to_string(), bytes));
                    }
                }
            }
        }
        return Err(format!("{path}: no DICOM instance found"));
    }
    let file = std::fs::File::open(root).map_err(|e| format!("{path}: {e}"))?;
    let mut magic = [0u8; 4];
    {
        use std::io::Read;
        let mut probe = &file;
        probe
            .read_exact(&mut magic)
            .map_err(|e| format!("{path}: {e}"))?;
    }
    if dicom::series::is_zip(&magic) {
        let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{path}: {e}"))?;
        let mut names: Vec<(usize, String)> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| (i, e.name().to_string())))
            .filter(|(_, n)| !n.ends_with('/'))
            .collect();
        names.sort_by(|a, b| a.1.cmp(&b.1));
        for (i, name) in names {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            use std::io::Read;
            entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if dicom::load::load_header(&bytes).is_ok() {
                return Ok((format!("{path}/{name}"), bytes));
            }
        }
        return Err(format!("{path}: no DICOM instance in the archive"));
    }
    let bytes = std::fs::read(root).map_err(|e| format!("{path}: {e}"))?;
    Ok((path.to_string(), bytes))
}

/// Every file under a folder (recursively, sorted), or the single file at
/// `path`, as scan inputs. A zip is handed over as one input; the scanner
/// reads it entry by entry.
pub fn collect_inputs(path: &str) -> Result<Vec<FileEntry>, String> {
    let root = std::path::Path::new(path);
    if !root.is_dir() {
        return Ok(vec![FileEntry::new(
            root.display().to_string(),
            std::fs::read(root).map_err(|e| format!("{path}: {e}"))?,
        )]);
    }
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        children.sort();
        children.reverse();
        for child in children {
            if child.is_dir() {
                stack.push(child);
            } else if let Ok(bytes) = std::fs::read(&child) {
                entries.push(FileEntry::new(
                    child
                        .strip_prefix(root)
                        .unwrap_or(&child)
                        .display()
                        .to_string(),
                    bytes,
                ));
            }
        }
    }
    Ok(entries)
}
