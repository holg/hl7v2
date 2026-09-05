//! File inputs that are also drop targets. web-sys only, no JS.
//!
//! Two pickers share one drop zone: a multi-file picker and a folder picker
//! (`webkitdirectory`). Dropping several files works; dropping a folder does
//! not, because reading directory entries from a drop needs the
//! non-standard `webkitGetAsEntry` API and is left out on purpose.

use crate::error::AppError;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos::web_sys::{DragEvent, Event, File, FileList, HtmlInputElement};
use wasm_bindgen_futures::JsFuture;

/// Names and bytes of every file read, or why reading stopped.
pub type FilesResult = Result<Vec<(String, Vec<u8>)>, AppError>;

#[component]
pub fn FileDrop(
    label: &'static str,
    accept: &'static str,
    /// Offer a folder picker and allow multi-select.
    #[prop(default = false)]
    multiple: bool,
    /// What is currently loaded, if anything.
    loaded: Signal<Option<String>>,
    on_files: Callback<FilesResult>,
) -> impl IntoView {
    let progress = RwSignal::new(None::<String>);
    // `webkitdirectory` has no typed attribute in Leptos; set it on the DOM
    // node once it exists.
    let folder_ref = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(input) = folder_ref.get() {
            let _ = input.set_attribute("webkitdirectory", "");
            let _ = input.set_attribute("directory", "");
        }
    });
    let on_change = move |ev: Event| {
        let input: HtmlInputElement = event_target(&ev);
        if let Some(list) = input.files() {
            read_files(list, on_files, progress);
        }
        // Allow re-selecting the same file(s) later.
        input.set_value("");
    };
    let on_drop = move |ev: DragEvent| {
        ev.prevent_default();
        if let Some(list) = ev.data_transfer().and_then(|dt| dt.files()) {
            read_files(list, on_files, progress);
        }
    };
    view! {
        <label
            class="drop"
            class:loaded=move || loaded.get().is_some()
            on:dragover=|ev: DragEvent| ev.prevent_default()
            on:drop=on_drop
        >
            <strong>{label}</strong>
            <div>
                {move || match (progress.get(), loaded.get()) {
                    (Some(p), _) => view! { <span class="name">{p}</span> }.into_any(),
                    (None, Some(name)) => view! { <span class="name">{name}</span> }.into_any(),
                    (None, None) => view! { <small>{if multiple {
                        "Choose files, a folder or a zip, or drop files here."
                    } else {
                        "Choose a file or drop it here."
                    }}</small> }.into_any(),
                }}
            </div>
            <input type="file" accept=accept multiple=multiple on:change=on_change />
            {multiple.then(|| view! {
                <input type="file" node_ref=folder_ref on:change=on_change />
            })}
        </label>
    }
}

/// Read every file in the list into memory with `File.arrayBuffer()`. This
/// is the only place bytes enter the application.
fn read_files(list: FileList, on_files: Callback<FilesResult>, progress: RwSignal<Option<String>>) {
    let files: Vec<File> = (0..list.length()).filter_map(|i| list.get(i)).collect();
    if files.is_empty() {
        return;
    }
    spawn_local(async move {
        let total = files.len();
        let mut out = Vec::with_capacity(total);
        for (i, file) in files.into_iter().enumerate() {
            if total > 1 {
                progress.set(Some(format!("Reading {} of {total}…", i + 1)));
            }
            // `webkitRelativePath` is set for folder picks; web-sys has no
            // binding for it, so read the property directly.
            let path = js_sys::Reflect::get(&file, &"webkitRelativePath".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            let name = if path.is_empty() { file.name() } else { path };
            match JsFuture::from(file.array_buffer()).await {
                Ok(buf) => out.push((name, js_sys::Uint8Array::new(&buf).to_vec())),
                Err(e) => {
                    progress.set(None);
                    on_files.run(Err(AppError::FileRead(format!(
                        "{name}: {}",
                        e.as_string()
                            .unwrap_or_else(|| "arrayBuffer() rejected".to_string())
                    ))));
                    return;
                }
            }
        }
        progress.set(None);
        on_files.run(Ok(out));
    });
}
