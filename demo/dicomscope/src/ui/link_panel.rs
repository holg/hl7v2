//! Which path linked, the compared values, and the patient-mismatch warning.

use crate::link::{LinkPath, Linkage, Pair};
use leptos::prelude::*;

#[component]
pub fn LinkPanel(linkage: Signal<Option<Linkage>>) -> impl IntoView {
    view! {
        <div>
            {move || match linkage.get() {
                None => view! {
                    <p class="banner info">"Load both a DICOM file and an HL7 order to compare their identifiers."</p>
                }.into_any(),
                Some(l) => view! {
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
                            "Neither the Study Instance UID nor the Accession Number matches."</p>
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
                    <table>
                        <thead><tr><th>"Identifier"</th><th>"DICOM"</th><th>"HL7"</th><th></th></tr></thead>
                        <tbody>
                            <PairRow name="Study Instance UID" dicom_tag="(0020,000D)" hl7_path="ZDS-1.1" pair=l.study_uid.clone() />
                            <PairRow name="Accession Number" dicom_tag="(0008,0050)" hl7_path="OBR-18" pair=l.accession.clone() />
                            <PairRow name="Patient ID" dicom_tag="(0010,0020)" hl7_path="PID-3.1" pair=l.patient_id.clone() />
                            <PairRow name="Requested Procedure ID" dicom_tag="(0040,1001)" hl7_path="OBR-19" pair=l.procedure_id.clone() />
                        </tbody>
                    </table>
                }.into_any(),
            }}
            <p><small>
                "Why the accession fallback exists: ZDS is a vendor-defined Z-segment from the IHE \
                 Radiology Technical Framework, not part of HL7 v2 proper, so many sites never populate \
                 it; and when the study UID is absent, archives such as dcm4che derive one from the \
                 requested procedure ID or the accession number, or generate a random one. The \
                 identifier that looks canonical may have been invented downstream."
            </small></p>
        </div>
    }
}

#[component]
fn PairRow(
    name: &'static str,
    dicom_tag: &'static str,
    hl7_path: &'static str,
    pair: Pair,
) -> impl IntoView {
    let verdict = match pair.matches() {
        Some(true) => ("match", "="),
        Some(false) => ("mismatch", "≠"),
        None => ("", "–"),
    };
    view! {
        <tr>
            <td>{name}<br/><small>{dicom_tag} " / " {hl7_path}</small></td>
            <td>{pair.dicom.clone().unwrap_or_else(|| "absent".into())}</td>
            <td>{pair.hl7.clone().unwrap_or_else(|| "absent".into())}</td>
            <td class=verdict.0>{verdict.1}</td>
        </tr>
    }
}
