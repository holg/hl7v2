//! The Modality Worklist item `mwlkit` builds from the order: where its
//! Study Instance UID came from, every decision the mapping had to make,
//! the chain order → worklist → image, the data set itself, and a download
//! of the Part 10 file. Built locally; the blob URL never leaves the page.

use crate::link::{Chain, ChainFinding, ChainRow};
use crate::ui::fhir_panel::blob_url;
use crate::ui::TagTree;
use crate::worklist::WorklistOutput;
use leptos::prelude::*;
use leptos::web_sys::Url;
use mwlkit::StudyUidOrigin;
use std::sync::Arc;

#[component]
pub fn WorklistPanel(
    /// `None` until an order is loaded; `Err` when `mwlkit` refused it.
    output: Signal<Option<Result<Arc<WorklistOutput>, String>>>,
    /// The three hops, once a study is loaded as well.
    chain: Signal<Option<Chain>>,
) -> impl IntoView {
    let last_url: StoredValue<Option<String>> = StoredValue::new(None);
    on_cleanup(move || {
        if let Some(url) = last_url.get_value() {
            let _ = Url::revoke_object_url(&url);
        }
    });
    let download_url = move || {
        let out = output.get()?.ok()?;
        if let Some(old) = last_url.get_value() {
            let _ = Url::revoke_object_url(&old);
        }
        let url = blob_url(&out.bytes, "application/dicom");
        last_url.set_value(url.clone());
        url
    };
    let rows = Signal::derive(move || {
        output
            .get()
            .and_then(|r| r.ok())
            .map(|o| Arc::new(o.rows.clone()))
    });

    view! {
        <div class="worklist">
            {move || match output.get() {
                None => view! {
                    <p><small>"Load an HL7 order; the worklist item is built from it alone. \
                        Load a DICOM study as well to follow the identifiers from the order through the worklist into the images."</small></p>
                }.into_any(),
                Some(Err(e)) => view! {
                    <p class="banner danger"><strong>"Refused: "</strong>{e}
                        " A worklist SCP that filled this in with a placeholder would hand the modality an entry the RIS cannot reconcile; "
                        <code>"mwlkit"</code>" refuses instead."</p>
                }.into_any(),
                Some(Ok(out)) => {
                    let origin = match out.item.study_uid.origin {
                        StudyUidOrigin::FromOrder(src) => (
                            "ok",
                            format!("Study Instance UID {} taken from the order ({}). The RIS knows this UID.", out.item.study_uid.value, src.path()),
                        ),
                        StudyUidOrigin::Generated(from) => (
                            "warn",
                            format!(
                                "Study Instance UID {} generated {from}: the order carried none. The modality will copy this UID into every image, and the RIS has never seen it.",
                                out.item.study_uid.value
                            ),
                        ),
                    };
                    let warnings: Vec<String> = out.item.warnings.iter().map(|w| w.to_string()).collect();
                    view! {
                        <p class=format!("banner {}", origin.0)>{origin.1}</p>
                        {warnings.into_iter().map(|w| view! {
                            <p class="banner warn"><strong>"Worklist note: "</strong>{w}</p>
                        }).collect_view()}
                        {move || chain.get().map(|c| view! { <ChainView chain=c /> })}
                        <div class="viewbar">
                            {move || download_url().map(|url| view! {
                                <a download="order.mwl.dcm" href=url>"Download order.mwl.dcm"</a>
                            })}
                            <small>{format!("{} bytes, Explicit VR Little Endian, Modality Worklist Information Model - FIND", out.bytes.len())}</small>
                        </div>
                        <TagTree rows=rows />
                        <p><small>
                            "Mapping per IHE Radiology RAD-4 with dcm4che's behaviour where they differ; the source of every \
                             attribute is in the "
                            <a href="https://docs.rs/mwlkit" target="_blank" rel="noopener">"mwlkit documentation"</a>
                            ". The demo asks for truncation of over-long SH/LO/PN values so the effect is visible above; the crate's \
                             default refuses. Station AE title defaults to DICOMSCOPE when the order carries none."
                        </small></p>
                    }.into_any()
                }
            }}
        </div>
    }
}

#[component]
fn ChainView(chain: Chain) -> impl IntoView {
    let findings = chain.findings.clone();
    view! {
        {findings.into_iter().map(|f| {
            let class = if f == ChainFinding::Intact { "banner ok" } else { "banner danger" };
            view! { <p class=class><strong>{finding_title(f)}</strong>{f.explanation()}</p> }
        }).collect_view()}
        <table>
            <thead><tr>
                <th>"Identifier"</th><th>"Order (HL7)"</th><th></th><th>"Worklist item"</th><th></th><th>"Image (DICOM)"</th>
            </tr></thead>
            <tbody>
                {chain.rows.into_iter().map(|r| view! { <ChainRowView row=r /> }).collect_view()}
            </tbody>
        </table>
    }
}

fn finding_title(f: ChainFinding) -> &'static str {
    match f {
        ChainFinding::GeneratedUidReachedImage => "Generated UID reached the image. ",
        ChainFinding::GeneratedUidNotInImage => "Generated UID is not in the image. ",
        ChainFinding::OrderUidReplacedDownstream => "Order UID replaced downstream. ",
        ChainFinding::TruncatedAccessionBreaksLink => {
            "Truncated accession number breaks the RIS link. "
        }
        ChainFinding::TruncatedAccessionAndImageDiffers => {
            "Truncated accession number, and the image differs. "
        }
        ChainFinding::Intact => "Chain intact. ",
    }
}

#[component]
fn ChainRowView(row: ChainRow) -> impl IntoView {
    let arrow = |m: Option<bool>| match m {
        Some(true) => ("match", "→"),
        Some(false) => ("mismatch", "≠"),
        None => ("", "–"),
    };
    let a = arrow(row.order_to_worklist());
    let b = arrow(row.worklist_to_image());
    let cell = |v: Option<String>| v.unwrap_or_else(|| "absent".into());
    view! {
        <tr>
            <td>{row.name}</td>
            <td>{cell(row.order)}</td>
            <td class=a.0>{a.1}</td>
            <td>{cell(row.worklist)}</td>
            <td class=b.0>{b.1}</td>
            <td>{cell(row.image)}</td>
        </tr>
    }
}
