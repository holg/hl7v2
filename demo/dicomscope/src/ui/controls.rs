//! Window center/width sliders and presets. Sliders write the signal on
//! every `input` event; the effect in `app` pushes a 16-byte uniform and
//! redraws, so there is nothing to debounce.

use leptos::prelude::*;

const PRESETS: &[(&str, f32, f32)] = &[
    ("Soft tissue", 40.0, 400.0),
    ("Lung", -600.0, 1500.0),
    ("Bone", 500.0, 2000.0),
];

#[component]
pub fn WindowControls(
    window: RwSignal<(f32, f32)>,
    /// Min and max sample value of the loaded frame, for slider ranges.
    range: Signal<(f32, f32)>,
    /// The file's own (0028,1050)/(0028,1051), if any.
    default_window: Signal<Option<(f32, f32)>>,
) -> impl IntoView {
    let center_min = move || range.get().0.floor();
    let center_max = move || range.get().1.ceil();
    let width_max = move || ((range.get().1 - range.get().0) * 2.0).max(2.0).ceil();
    let set_center = move |ev| {
        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
            window.update(|w| w.0 = v);
        }
    };
    let set_width = move |ev| {
        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
            window.update(|w| w.1 = v.max(1.0));
        }
    };
    view! {
        <div class="controls">
            <label>
                <span>"Center"</span>
                <input type="range" min=center_min max=center_max step="1"
                    prop:value=move || window.get().0.to_string() on:input=set_center />
                <span>{move || format!("{:.0}", window.get().0)}</span>
            </label>
            <label>
                <span>"Width"</span>
                <input type="range" min="1" max=width_max step="1"
                    prop:value=move || window.get().1.to_string() on:input=set_width />
                <span>{move || format!("{:.0}", window.get().1)}</span>
            </label>
            <div class="presets">
                {PRESETS.iter().map(|&(name, c, w)| view! {
                    <button type="button" on:click=move |_| window.set((c, w))>
                        {name} " " {format!("{c:.0}/{w:.0}")}
                    </button>
                }).collect_view()}
                <button type="button" disabled=move || default_window.get().is_none()
                    on:click=move |_| { if let Some(d) = default_window.get() { window.set(d) } }>
                    "File default"
                </button>
            </div>
            <small>"Presets are in Hounsfield units and only meaningful for CT."</small>
        </div>
    }
}
