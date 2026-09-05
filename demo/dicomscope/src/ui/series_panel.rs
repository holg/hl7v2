//! Series picker with thumbnails, slice slider, document list, and the list
//! of files that were skipped.

use crate::dicom::StudySet;
use crate::thumbnail::Thumbnail;
use leptos::prelude::*;
use leptos::web_sys::{CanvasRenderingContext2d, ImageData};
use std::sync::Arc;
use wasm_bindgen::{Clamped, JsCast};

/// One thumbnail per series, in series order; `None` where decoding failed.
pub type Thumbs = Arc<Vec<Option<Arc<Thumbnail>>>>;

/// Paint an RGBA thumbnail into a `<canvas>` through the 2D context. This
/// is the one place a 2D canvas is used; the image itself is WebGPU.
#[component]
pub fn Thumb(thumb: Arc<Thumbnail>) -> impl IntoView {
    let node = NodeRef::<leptos::html::Canvas>::new();
    let t = thumb.clone();
    Effect::new(move |_| {
        let Some(canvas) = node.get() else {
            return;
        };
        canvas.set_width(t.width);
        canvas.set_height(t.height);
        let Some(ctx) = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
        else {
            return;
        };
        if let Ok(data) =
            ImageData::new_with_u8_clamped_array_and_sh(Clamped(&t.rgba), t.width, t.height)
        {
            let _ = ctx.put_image_data(&data, 0.0, 0.0);
        }
    });
    view! { <canvas node_ref=node width=thumb.width height=thumb.height /> }
}

#[component]
pub fn SeriesPanel(
    set: Signal<Option<Arc<StudySet>>>,
    thumbs: Signal<Option<Thumbs>>,
    /// (series index, slice index)
    current: Signal<(usize, usize)>,
    current_document: Signal<Option<usize>>,
    on_select: Callback<(usize, usize)>,
    on_document: Callback<usize>,
) -> impl IntoView {
    let slice_count = move || {
        let (s, _) = current.get();
        set.get()
            .and_then(|st| st.series.get(s).map(|x| x.slices.len()))
            .unwrap_or(0)
    };
    view! {
        <div class="series">
            {move || set.get().map(|st| {
                let (cur_series, cur_slice) = current.get();
                let thumbs = thumbs.get();
                view! {
                    <div class="series-list">
                        {st.series.iter().enumerate().map(|(i, s)| {
                            let label = s.label();
                            let thumb = thumbs.as_ref().and_then(|t| t.get(i).cloned().flatten());
                            view! {
                                <button type="button" class:active=move || current.get().0 == i
                                    on:click=move |_| on_select.run((i, 0))>
                                    {thumb.map(|t| view! { <Thumb thumb=t /> })}
                                    <span>{label}</span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                    {(slice_count() > 1).then(|| view! {
                        <label class="slider">
                            <span>"Slice"</span>
                            <input type="range" min="0" max=(slice_count() - 1).to_string() step="1"
                                prop:value=cur_slice.to_string()
                                on:input=move |ev| {
                                    if let Ok(i) = event_target_value(&ev).parse::<usize>() {
                                        on_select.run((cur_series, i));
                                    }
                                } />
                            <span>{format!("{} / {}", cur_slice + 1, slice_count())}</span>
                        </label>
                    })}
                    {(!st.documents.is_empty()).then(|| view! {
                        <div class="documents">
                            <span>"Documents:"</span>
                            {st.documents.iter().enumerate().map(|(i, d)| {
                                let label = d.label();
                                view! {
                                    <button type="button" class:active=move || current_document.get() == Some(i)
                                        on:click=move |_| on_document.run(i)>{label}</button>
                                }
                            }).collect_view()}
                        </div>
                    })}
                    {(!st.skipped.is_empty()).then(|| view! {
                        <details>
                            <summary>{format!("{} file(s) skipped", st.skipped.len())}</summary>
                            <ul>
                                {st.skipped.iter().map(|s| view! {
                                    <li><span class="name">{s.name.clone()}</span>": "{s.reason.clone()}</li>
                                }).collect_view()}
                            </ul>
                        </details>
                    })}
                    <small>{format!("{} series, {} slices, {} document(s), {} files read{}",
                        st.series.len(), st.slice_count(), st.documents.len(), st.files.len(),
                        if st.ignored_metadata > 0 { format!(", {} OS metadata entries ignored", st.ignored_metadata) } else { String::new() })}</small>
                }
            })}
        </div>
    }
}
