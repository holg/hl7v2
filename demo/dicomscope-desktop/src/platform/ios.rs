//! UIKit pieces the iPad needs: the document picker, and copying what it
//! picked into the app's Documents folder so the study stays available.
//! The only unsafe code in the crate lives here, at the Objective-C border.

#![allow(unsafe_code)]

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_ui_kit::{UIApplication, UIDocumentPickerDelegate, UIDocumentPickerViewController};
use objc2_uniform_type_identifiers::UTType;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Paths the picker delivered, drained by the window loop.
static PICKED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

thread_local! {
    static DELEGATE: RefCell<Option<Retained<PickerDelegate>>> = const { RefCell::new(None) };
}

/// What a picker is for.
#[derive(Clone, Copy)]
pub enum Pick {
    /// Studies: DICOM files, zips, or a folder.
    Study,
    /// An HL7 order.
    Order,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the type has no
    // Drop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "DicomscopePickerDelegate"]
    struct PickerDelegate;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for PickerDelegate {}

    // SAFETY: the method signatures match UIDocumentPickerDelegate.
    unsafe impl UIDocumentPickerDelegate for PickerDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn did_pick(&self, _controller: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            let Some(docs) = super::documents_dir() else {
                return;
            };
            let mut picked = Vec::new();
            for i in 0..urls.count() {
                let url = urls.objectAtIndex(i);
                let Some(path) = url.path().map(|p| PathBuf::from(p.to_string())) else {
                    continue;
                };
                // Files outside the sandbox need the security scope while we
                // copy them; copies the picker made for us do not, and the
                // call simply returns false then.
                let scoped = unsafe { url.startAccessingSecurityScopedResource() };
                let result = import(&path, &docs);
                if scoped {
                    unsafe { url.stopAccessingSecurityScopedResource() };
                }
                match result {
                    Ok(p) => picked.push(p),
                    Err(e) => eprintln!("import {}: {e}", path.display()),
                }
            }
            if let Ok(mut q) = PICKED.lock() {
                q.extend(picked);
            }
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _controller: &UIDocumentPickerViewController) {}
    }
);

impl PickerDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

/// Copy a picked file or folder into Documents; returns the new path.
fn import(src: &Path, docs: &Path) -> Result<PathBuf, String> {
    let name = src
        .file_name()
        .ok_or_else(|| "picked path has no name".to_string())?;
    let dst = docs.join(name);
    if src.is_dir() {
        copy_dir(src, &dst)?;
    } else {
        std::fs::copy(src, &dst).map_err(|e| e.to_string())?;
    }
    Ok(dst)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Show the document picker. Returns at once; results arrive through
/// [`take_picked`].
pub fn present(kind: Pick) {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("document picker: not on the main thread");
        return;
    };
    let identifiers: &[&str] = match kind {
        Pick::Study => &[
            "eu.iesna.dicom",
            "public.zip-archive",
            "public.folder",
            "public.data",
        ],
        Pick::Order => &["eu.iesna.hl7", "public.plain-text"],
    };
    let types: Vec<Retained<UTType>> = identifiers
        .iter()
        .filter_map(|id| UTType::typeWithIdentifier(&NSString::from_str(id)))
        .collect();
    let types = NSArray::from_retained_slice(&types);
    // Not as copies: folders cannot be copied by the picker, and a file
    // copied by us ends up in Documents either way.
    let picker = UIDocumentPickerViewController::initForOpeningContentTypes_asCopy(
        UIDocumentPickerViewController::alloc(mtm),
        &types,
        false,
    );
    picker.setAllowsMultipleSelection(matches!(kind, Pick::Study));
    let delegate = DELEGATE.with(|d| {
        d.borrow_mut()
            .get_or_insert_with(|| PickerDelegate::new(mtm))
            .clone()
    });
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let app = UIApplication::sharedApplication(mtm);
    // `windows` is deprecated in favour of scenes, but winit sets up a
    // single window without a scene delegate, and this is the one call
    // that finds it on every iOS the app supports.
    #[allow(deprecated)]
    let windows = app.windows();
    let root = (0..windows.count())
        .map(|i| windows.objectAtIndex(i))
        .find(|w| w.isKeyWindow())
        .or_else(|| (windows.count() > 0).then(|| windows.objectAtIndex(0)))
        .and_then(|w| w.rootViewController());
    match root {
        Some(root) => root.presentViewController_animated_completion(&picker, true, None),
        None => eprintln!("document picker: no root view controller to present from"),
    }
}

/// Whether the picker delivered something not yet taken.
pub fn has_picked() -> bool {
    PICKED.lock().map(|q| !q.is_empty()).unwrap_or(false)
}

/// Paths imported since the last call.
pub fn take_picked() -> Vec<PathBuf> {
    PICKED
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}
