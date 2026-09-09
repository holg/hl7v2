//! The generated FHIR resources as pretty-printed JSON, with copy and
//! download. Both use local browser APIs only: the Clipboard API and a
//! blob URL. No request leaves the page.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos::web_sys::{Blob, BlobPropertyBag, Url};
use std::sync::Arc;
use wasm_bindgen_futures::JsFuture;

/// Pretty-printed JSON for each tab.
#[derive(Clone, PartialEq)]
pub struct FhirText {
    pub patient: String,
    pub service_request: String,
    pub imaging_study: String,
    pub bundle: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Bundle,
    Patient,
    ServiceRequest,
    ImagingStudy,
}

impl Tab {
    const ALL: [Tab; 4] = [
        Tab::Bundle,
        Tab::Patient,
        Tab::ServiceRequest,
        Tab::ImagingStudy,
    ];
    fn label(self) -> &'static str {
        match self {
            Tab::Bundle => "Bundle",
            Tab::Patient => "Patient",
            Tab::ServiceRequest => "ServiceRequest",
            Tab::ImagingStudy => "ImagingStudy",
        }
    }
    fn text(self, t: &FhirText) -> &str {
        match self {
            Tab::Bundle => &t.bundle,
            Tab::Patient => &t.patient,
            Tab::ServiceRequest => &t.service_request,
            Tab::ImagingStudy => &t.imaging_study,
        }
    }
}

#[component]
pub fn FhirPanel(output: Signal<Option<Arc<FhirText>>>) -> impl IntoView {
    let tab = RwSignal::new(Tab::Bundle);
    let status = RwSignal::new(None::<String>);
    let pre_ref = NodeRef::<leptos::html::Pre>::new();
    // One blob URL for the bundle at a time; revoked when replaced.
    let last_url: StoredValue<Option<String>> = StoredValue::new(None);
    on_cleanup(move || {
        if let Some(url) = last_url.get_value() {
            let _ = Url::revoke_object_url(&url);
        }
    });

    let current_text = move || {
        output
            .get()
            .map(|t| tab.get().text(&t).to_string())
            .unwrap_or_default()
    };

    let copy = move |_| {
        let text = current_text();
        if text.is_empty() {
            return;
        }
        let Some(window) = leptos::web_sys::window() else {
            return;
        };
        let clipboard = window.navigator().clipboard();
        spawn_local(async move {
            match JsFuture::from(clipboard.write_text(&text)).await {
                Ok(_) => status.set(Some(format!("Copied {} characters.", text.chars().count()))),
                Err(_) => {
                    // Insecure context or permission denied: select the text
                    // so a manual copy is one keystroke away.
                    if let Some(pre) = pre_ref.get_untracked() {
                        if let Some(sel) = window.get_selection().ok().flatten() {
                            let _ = sel.select_all_children(&pre);
                        }
                    }
                    status.set(Some(
                        "Clipboard API unavailable here; the text is selected, press Ctrl/Cmd+C."
                            .into(),
                    ));
                }
            }
        });
    };

    let download_url = move || {
        let text = output.get()?.bundle.clone();
        if let Some(old) = last_url.get_value() {
            let _ = Url::revoke_object_url(&old);
        }
        let url = blob_url(text.as_bytes(), "application/fhir+json");
        last_url.set_value(url.clone());
        url
    };

    view! {
        <div class="fhir">
            {move || match output.get() {
                None => view! {
                    <p><small>"Load both a DICOM study and an HL7 order; the FHIR R4 bundle is generated from the pair."</small></p>
                }.into_any(),
                Some(_) => view! {
                    <div class="viewbar">
                        {Tab::ALL.iter().map(|&t| view! {
                            <button type="button" class:active=move || tab.get() == t
                                on:click=move |_| { tab.set(t); status.set(None); }>{t.label()}</button>
                        }).collect_view()}
                        <button type="button" on:click=copy>"Copy"</button>
                        {move || download_url().map(|url| view! {
                            <a download="bundle.json" href=url>"Download bundle.json"</a>
                        })}
                        {move || status.get().map(|s| view! { <small>{s}</small> })}
                    </div>
                    <pre class="json" node_ref=pre_ref>{current_text}</pre>
                    <p><small>
                        "FHIR R4 (4.0.1). The ImagingStudy declares the MII Bildgebung profile, version 2025.0.0-ballot, \
                         and populates its core elements; it is not validated here. Bundles built by this mapping from \
                         the sample study and orders are validated on every commit with the HL7 FHIR validator against \
                         R4 and the MII package, with 0 errors: see the "
                        <a href="fhir/" target="_blank" rel="noopener">"published reference bundles and report"</a>
                        ". Expected warnings there: no narrative (best practice), a code without a URI system when \
                         OBR-4.3 names a local table, and DICOM value sets the validator cannot fetch."
                    </small></p>
                }.into_any(),
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
