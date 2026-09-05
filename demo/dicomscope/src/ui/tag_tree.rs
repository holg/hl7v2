//! Scrollable, filterable list of every data element.

use crate::dicom::TagRow;
use leptos::prelude::*;
use std::sync::Arc;

#[component]
pub fn TagTree(rows: Signal<Option<Arc<Vec<TagRow>>>>) -> impl IntoView {
    let filter = RwSignal::new(String::new());
    view! {
        <div class="tags">
            <input type="search" placeholder="Filter by tag, keyword or value"
                prop:value=move || filter.get()
                on:input=move |ev| filter.set(event_target_value(&ev)) />
            {move || match rows.get() {
                None => view! { <p class="placeholder">"No DICOM file loaded."</p> }.into_any(),
                Some(rows) => {
                    let needle = filter.get();
                    let shown: Vec<_> = rows.iter().filter(|r| r.matches(&needle)).cloned().collect();
                    view! {
                        <table>
                            <thead><tr><th>"Tag"</th><th>"Keyword"</th><th>"VR"</th><th>"Value"</th></tr></thead>
                            <tbody>
                                {shown.into_iter().map(|r| view! {
                                    <tr class=format!("depth{}", r.depth.min(3))>
                                        <td>{r.tag}</td>
                                        <td class="k">{r.keyword}</td>
                                        <td>{r.vr}</td>
                                        <td>{r.value}</td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any()
                }
            }}
        </div>
    }
}
