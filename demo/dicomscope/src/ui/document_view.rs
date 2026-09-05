//! A Structured Report as an indented tree, or an Encapsulated PDF in the
//! browser's own PDF viewer through a blob URL (no network, no plugin).

use crate::dicom::sr::SrLine;
use leptos::prelude::*;
use leptos::web_sys::{Blob, BlobPropertyBag, Url};
use std::sync::Arc;

/// What the app decoded for the selected document.
#[derive(Clone, PartialEq)]
pub enum DocumentContent {
    Report {
        title: String,
        lines: Arc<Vec<SrLine>>,
    },
    Pdf {
        title: String,
        bytes: Arc<Vec<u8>>,
        mime: String,
    },
    Failed(String),
}

#[component]
pub fn DocumentView(content: Signal<Option<DocumentContent>>) -> impl IntoView {
    // Blob URLs are per document; revoke the previous one when it changes.
    let last_url: StoredValue<Option<String>> = StoredValue::new(None);
    on_cleanup(move || {
        if let Some(url) = last_url.get_value() {
            let _ = Url::revoke_object_url(&url);
        }
    });
    view! {
        <div class="doc">
            {move || match content.get() {
                None => None,
                Some(DocumentContent::Failed(e)) => Some(view! { <p class="banner danger">{e}</p> }.into_any()),
                Some(DocumentContent::Report { title, lines }) => Some(view! {
                    <h3>{title}</h3>
                    <pre>{crate::dicom::sr::sr_to_text(&lines)}</pre>
                }.into_any()),
                Some(DocumentContent::Pdf { title, bytes, mime }) => {
                    if let Some(old) = last_url.get_value() {
                        let _ = Url::revoke_object_url(&old);
                    }
                    let url = blob_url(&bytes, &mime);
                    last_url.set_value(url.clone());
                    Some(match url {
                        Some(url) => view! {
                            <h3>{title}</h3>
                            <iframe src=url title="Encapsulated PDF"></iframe>
                            <small>{format!("{} bytes, {mime}", bytes.len())}</small>
                        }.into_any(),
                        None => view! { <p class="banner danger">"The browser refused to create a blob URL for the PDF."</p> }.into_any(),
                    })
                }
            }}
        </div>
    }
}

fn blob_url(bytes: &[u8], mime: &str) -> Option<String> {
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let options = BlobPropertyBag::new();
    options.set_type(mime);
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options).ok()?;
    Url::create_object_url_with_blob(&blob).ok()
}
