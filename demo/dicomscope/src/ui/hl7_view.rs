//! Raw message with the four order fields highlighted in place, using the
//! byte spans the `hl7kit` crate returns, so nothing is re-searched.

use hl7kit::order::OrderField;
use hl7kit::Span;
use leptos::prelude::*;

/// Text split into plain and highlighted chunks. Pure, so it can be tested on
/// the host if the UI ever moves.
pub fn chunks(raw: &str, spans: &[(Span, OrderField)]) -> Vec<(String, Option<OrderField>)> {
    let mut spans: Vec<_> = spans
        .iter()
        .copied()
        .filter(|(s, _)| s.end <= raw.len())
        .collect();
    spans.sort_by_key(|(s, _)| s.start);
    let mut out = Vec::new();
    let mut pos = 0;
    for (span, field) in spans {
        if span.start < pos {
            continue; // overlapping; keep the first
        }
        if span.start > pos {
            out.push((display(&raw[pos..span.start]), None));
        }
        out.push((display(span.slice(raw)), Some(field)));
        pos = span.end;
    }
    if pos < raw.len() {
        out.push((display(&raw[pos..]), None));
    }
    out
}

/// Segment terminators become newlines for display. Same byte length, so the
/// spans are unaffected.
fn display(s: &str) -> String {
    s.replace('\r', "\n")
}

fn css_class(f: OrderField) -> &'static str {
    match f {
        OrderField::PatientId => "patient",
        OrderField::Accession => "accession",
        OrderField::ProcedureId => "procedure",
        OrderField::StudyUid => "study",
    }
}

#[component]
pub fn Hl7View(
    raw: Signal<String>,
    spans: Signal<Vec<(Span, OrderField)>>,
    warnings: Signal<Vec<String>>,
    summary: Signal<String>,
) -> impl IntoView {
    view! {
        <div>
            <p>{move || summary.get()}</p>
            <p class="legend">
                {OrderField::ALL.iter().map(|&f| view! {
                    <mark class=css_class(f)>{f.label()} " (" {f.paths().join(" or ")} ")"</mark>
                }).collect_view()}
            </p>
            {move || {
                let w = warnings.get();
                (!w.is_empty()).then(|| view! {
                    <p class="banner info">"Tolerated: " {w.join("; ")}</p>
                })
            }}
            <pre class="hl7">
                {move || chunks(&raw.get(), &spans.get()).into_iter().map(|(text, field)| match field {
                    Some(f) => view! { <mark class=css_class(f)>{text}</mark> }.into_any(),
                    None => view! { <span>{text}</span> }.into_any(),
                }).collect_view()}
            </pre>
        </div>
    }
}
