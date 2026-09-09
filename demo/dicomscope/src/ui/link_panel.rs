//! Which path linked, the compared values with their HL7, DICOM and FHIR
//! locations, and the warnings that matter: patient mismatch, conflicting
//! IPC study UIDs, and an unlinked study whose ImagingStudy carries no
//! `basedOn`.

use crate::fhir::fhir_element;
use crate::link::{LinkPath, Linkage, Pair};
use hl7kit::order::{Order, OrderField};
use leptos::prelude::*;

#[component]
pub fn LinkPanel(
    linkage: Signal<Option<Linkage>>,
    /// The extracted order, for the path that supplied each field and for
    /// its warnings.
    order: Signal<Option<Order>>,
    /// PID-3.4 as found in the message, if any; drives the "no assigning
    /// authority" note on the patient row.
    patient_authority: Signal<Option<String>>,
) -> impl IntoView {
    view! {
        <div>
            {move || match linkage.get() {
                None => view! {
                    <p class="banner info">"Load both a DICOM file and an HL7 order to compare their identifiers."</p>
                }.into_any(),
                Some(l) => {
                    let order = order.get();
                    let warnings: Vec<String> = order.as_ref()
                        .map(|o| o.warnings.iter().map(|w| w.to_string()).collect())
                        .unwrap_or_default();
                    let path_of = move |f: OrderField| -> String {
                        order.as_ref()
                            .and_then(|o| o.source_path(f))
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("{} (absent)", f.paths().join(" / ")))
                    };
                    let authority_note = match patient_authority.get() {
                        Some(a) => format!("assigning authority {a} from PID-3.4"),
                        None => "no assigning authority in message; identifier has no system".to_string(),
                    };
                    view! {
                        {if l.linked_with_patient_mismatch() {
                            view! {
                                <p class="banner danger">
                                    <strong>"Patient mismatch on a linked study. "</strong>
                                    "The study links to this order by " {l.path.label()}
                                    ", but Patient ID (0010,0020) and PID-3.1 disagree. \
                                     This is the case worth stopping for: the images are filed under \
                                     an order for a different patient."
                                </p>
                            }.into_any()
                        } else if l.path == LinkPath::None {
                            view! {
                                <p class="banner info"><strong>"No link. "</strong>
                                "Neither the Study Instance UID nor the Accession Number matches. \
                                 The FHIR ImagingStudy carries no basedOn: a relationship that did not \
                                 resolve is not asserted."</p>
                            }.into_any()
                        } else {
                            view! {
                                <p class="banner ok"><strong>"Linked by " {l.path.label()} ". "</strong>
                                {match l.patient_match {
                                    Some(true) => "Patient identifiers agree.",
                                    Some(false) => "",
                                    None => "Patient identifier is absent on one side, so it could not be compared.",
                                }}</p>
                            }.into_any()
                        }}
                        {warnings.into_iter().map(|w| view! {
                            <p class="banner danger"><strong>"Order warning: "</strong>{w}</p>
                        }).collect_view()}
                        <table>
                            <thead><tr>
                                <th>"Identifier"</th><th>"HL7 path"</th><th>"DICOM"</th><th>"HL7"</th><th></th><th>"FHIR element"</th>
                            </tr></thead>
                            <tbody>
                                <PairRow name="Study Instance UID" hl7_path=path_of(OrderField::StudyUid)
                                    dicom_tag="(0020,000D)" pair=l.study_uid.clone()
                                    fhir=fhir_element(OrderField::StudyUid) note=None />
                                <PairRow name="Accession Number" hl7_path=path_of(OrderField::Accession)
                                    dicom_tag="(0008,0050)" pair=l.accession.clone()
                                    fhir=fhir_element(OrderField::Accession) note=None />
                                <PairRow name="Patient ID" hl7_path=path_of(OrderField::PatientId)
                                    dicom_tag="(0010,0020)" pair=l.patient_id.clone()
                                    fhir=fhir_element(OrderField::PatientId) note=Some(authority_note) />
                                <PairRow name="Requested Procedure ID" hl7_path=path_of(OrderField::ProcedureId)
                                    dicom_tag="(0040,1001)" pair=l.procedure_id.clone()
                                    fhir=fhir_element(OrderField::ProcedureId)
                                    note=Some("carried, not standardised: v2-0203 has no type code for it".into()) />
                            </tbody>
                        </table>
                    }.into_any()
                }
            }}
            <p><small>
                "Why the accession fallback exists: ZDS is a vendor-defined Z-segment from the IHE \
                 Radiology Technical Framework, not part of HL7 v2 proper, so many sites never populate \
                 it; and when the study UID is absent, archives such as dcm4che derive one from the \
                 requested procedure ID or the accession number, or generate a random one. The \
                 identifier that looks canonical may have been invented downstream. OMI^O23 carries \
                 the study UID in the standard IPC-3 instead, which is read first."
            </small></p>
        </div>
    }
}

#[component]
fn PairRow(
    name: &'static str,
    hl7_path: String,
    dicom_tag: &'static str,
    pair: Pair,
    fhir: &'static str,
    note: Option<String>,
) -> impl IntoView {
    let verdict = match pair.matches() {
        Some(true) => ("match", "="),
        Some(false) => ("mismatch", "≠"),
        None => ("", "–"),
    };
    view! {
        <tr>
            <td>{name}</td>
            <td>{hl7_path}</td>
            <td>{pair.dicom.clone().unwrap_or_else(|| "absent".into())}<br/><small>{dicom_tag}</small></td>
            <td>{pair.hl7.clone().unwrap_or_else(|| "absent".into())}</td>
            <td class=verdict.0>{verdict.1}</td>
            <td><small>{fhir}{note.map(|n| view! { <br/>{n} })}</small></td>
        </tr>
    }
}
