//! The non-interactive subcommands. Each reads its inputs through the same
//! [`Session`] the viewer uses, prints text or JSON, and returns the exit
//! status: 0 for success, 3 when `link` finds no link; errors become 1 in
//! `main`.

use crate::cli::{
    DocsArgs, FhirArgs, FhirResource, GpuArgs, InfoArgs, LinkArgs, OrderArgs, RenderArgs,
    SliceArgs, TagsArgs, WorklistArgs,
};
use crate::gpu::{self, ImagePlugin, Look, WgpuPlugin};
use crate::term;
use dicomscope_core::dicom::TagRow;
use dicomscope_core::link::{Linkage, Pair};
use dicomscope_core::session::{DocumentContent, Session};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// `println!` that ends the program quietly when standard output is
/// closed, as in `dicomscope-tui tags scan.dcm | head`, instead of
/// panicking on the broken pipe.
macro_rules! out {
    ($($arg:tt)*) => {
        emit(format_args!($($arg)*))
    };
}

fn emit(args: std::fmt::Arguments) {
    let mut o = std::io::stdout().lock();
    if let Err(e) = o.write_fmt(args).and_then(|()| o.write_all(b"\n")) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
    }
}

/// Exit status of `link` for a study and order that do not link, whose
/// keys conflict, or whose patient IDs differ.
pub const EXIT_NO_LINK: u8 = 3;

/// A command's result: an exit status, or an error message for status 1.
pub type Outcome = Result<u8, String>;

fn strings(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.display().to_string()).collect()
}

/// Open a study, failing with the session's own message.
pub fn open_study(paths: &[PathBuf]) -> Result<Session, String> {
    let mut s = Session::new();
    s.open_study_paths(&strings(paths));
    match (&s.set, s.error.take()) {
        (Some(_), None) => Ok(s),
        (_, Some(e)) => Err(e),
        (None, None) => Err("no study opened".into()),
    }
}

/// Go to a 1-based (series, slice), checking both exist.
fn goto(s: &mut Session, at: SliceArgs) -> Result<(), String> {
    let set = s.set.as_ref().ok_or("no study")?;
    let series = at.series.checked_sub(1).ok_or("series counts from 1")?;
    let slice = at.slice.checked_sub(1).ok_or("slice counts from 1")?;
    let Some(sr) = set.series.get(series) else {
        return Err(format!(
            "series {} does not exist; the study has {}",
            at.series,
            set.series.len()
        ));
    };
    if slice >= sr.slices.len() {
        return Err(format!(
            "slice {} does not exist; series {} has {}",
            at.slice,
            at.series,
            sr.slices.len()
        ));
    }
    s.show_slice(series, slice, true);
    match s.error.take() {
        Some(e) if s.frame.is_none() && s.tags.is_empty() => Err(e),
        _ => Ok(()),
    }
}

fn open_order(s: &mut Session, path: &Path) -> Result<(), String> {
    s.open_hl7(&path.display().to_string());
    match s.error.take() {
        Some(e) => Err(e),
        None if s.hl7.is_none() => Err(format!("{}: not read", path.display())),
        None => Ok(()),
    }
}

fn print_json(v: &Value) -> Outcome {
    out!(
        "{}",
        serde_json::to_string_pretty(v).map_err(|e| e.to_string())?
    );
    Ok(0)
}

fn opt(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("—")
}

// --- info ---

/// `info`: the study, its series, documents and skipped files.
pub fn info(a: &InfoArgs) -> Outcome {
    let s = open_study(&a.study)?;
    let set = s.set.as_ref().ok_or("no study")?;
    let st = s.study.clone().unwrap_or_default();
    if a.format.json {
        return print_json(&json!({
            "study": {
                "studyInstanceUid": st.study_uid,
                "patientId": st.patient_id,
                "patientName": st.patient_name,
                "birthDate": st.patient_birth_date,
                "sex": st.patient_sex,
                "accessionNumber": st.accession_number,
                "requestedProcedureId": st.requested_procedure_id,
                "studyDate": st.study_date,
                "studyTime": st.study_time,
                "description": st.study_description,
                "modality": st.modality,
                "transferSyntax": st.transfer_syntax,
            },
            "series": set.series.iter().enumerate().map(|(i, sr)| json!({
                "index": i + 1,
                "number": sr.number,
                "uid": sr.uid,
                "modality": sr.modality,
                "description": sr.description,
                "rows": sr.rows,
                "columns": sr.cols,
                "slices": sr.slices.len(),
            })).collect::<Vec<_>>(),
            "documents": set.documents.iter().enumerate().map(|(i, d)| json!({
                "index": i + 1,
                "kind": d.kind.label(),
                "title": d.title,
                "seriesNumber": d.series_number,
            })).collect::<Vec<_>>(),
            "files": set.files.len(),
            "skipped": set.skipped.iter().map(|k| json!({"name": k.name, "reason": k.reason})).collect::<Vec<_>>(),
            "ignoredMetadata": set.ignored_metadata,
        }));
    }
    out!("Study            {}", opt(&st.study_uid));
    out!(
        "Patient          {}  (ID {}, born {}, sex {})",
        opt(&st.patient_name),
        opt(&st.patient_id),
        opt(&st.patient_birth_date),
        opt(&st.patient_sex)
    );
    out!("Accession        {}", opt(&st.accession_number));
    out!("Procedure ID     {}", opt(&st.requested_procedure_id));
    out!(
        "Date             {} {}",
        opt(&st.study_date),
        opt(&st.study_time)
    );
    out!("Description      {}", opt(&st.study_description));
    out!("Transfer syntax  {}", st.transfer_syntax);
    out!("");
    out!("Series ({}):", set.series.len());
    for (i, sr) in set.series.iter().enumerate() {
        out!(
            "  {:>3}. {:<48} {:>5} x {:<5}",
            i + 1,
            sr.label(),
            sr.cols,
            sr.rows
        );
    }
    if !set.documents.is_empty() {
        out!("");
        out!("Documents ({}):", set.documents.len());
        for (i, d) in set.documents.iter().enumerate() {
            out!("  {:>3}. {}", i + 1, d.label());
        }
    }
    if !set.skipped.is_empty() {
        out!("");
        out!(
            "Skipped ({} of {} files):",
            set.skipped.len(),
            set.files.len()
        );
        for k in set.skipped.iter().take(10) {
            out!("  {}: {}", k.name, k.reason);
        }
        if set.skipped.len() > 10 {
            out!("  ... and {} more", set.skipped.len() - 10);
        }
    }
    Ok(0)
}

// --- tags ---

/// Tag rows whose tag, keyword, VR or value contains `needle`, ignoring
/// case. Every row stands alone; sequence parents are not pulled in.
pub fn filter_tags<'a>(rows: &'a [TagRow], needle: Option<&str>) -> Vec<&'a TagRow> {
    let needle = needle.map(str::to_lowercase);
    rows.iter()
        .filter(|r| {
            needle.as_deref().is_none_or(|n| {
                [&r.tag, &r.keyword, &r.vr, &r.value]
                    .iter()
                    .any(|f| f.to_lowercase().contains(n))
            })
        })
        .collect()
}

/// `tags`: one slice's DICOM header.
pub fn tags(a: &TagsArgs) -> Outcome {
    let mut s = open_study(&a.study)?;
    goto(&mut s, a.at)?;
    let rows = filter_tags(&s.tags, a.grep.as_deref());
    if a.format.json {
        return print_json(&Value::Array(
            rows.iter()
                .map(|r| {
                    json!({"tag": r.tag, "keyword": r.keyword, "vr": r.vr, "value": r.value, "depth": r.depth})
                })
                .collect(),
        ));
    }
    for r in rows {
        let indent = "  ".repeat(r.depth);
        out!(
            "{indent}{} {:<2} {:<36} {}",
            r.tag,
            r.vr,
            r.keyword,
            r.value
        );
    }
    Ok(0)
}

// --- render ---

/// `render`: one slice through the GPU plugin, to PNG or inline.
pub fn render(a: &RenderArgs, g: GpuArgs) -> Outcome {
    let mut s = open_study(&a.study)?;
    goto(&mut s, a.at)?;
    let img = s.image().ok_or_else(|| {
        s.error
            .clone()
            .unwrap_or_else(|| "this slice has no displayable image".into())
    })?;
    if let Some(w) = a.window {
        s.window = w;
    }
    s.enhance = a.enhance;
    let oriented = dicomscope_core::view::Viewport {
        rotation: a.rotate,
        flip_h: a.flip_h,
        flip_v: a.flip_v,
        smooth: !a.sharp,
        ..Default::default()
    };
    let size = a.size.unwrap_or_else(|| oriented.display_size(img));
    s.set_canvas(size);
    s.view = oriented.fit(size, img).zoom_about(
        a.zoom.clamp(0.01, 100.0),
        size.0 as f32 / 2.0,
        size.1 as f32 / 2.0,
    );
    let mut plugin = WgpuPlugin::new(g.backend).map_err(|e| e.to_string())?;
    let look = Look { invert: a.invert };
    let picture = plugin
        .render(&mut s, size, look)
        .map_err(|e| e.to_string())?;
    if let Some(path) = &a.output {
        picture
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!(
            "{}: {}x{} via {}",
            path.display(),
            size.0,
            size.1,
            plugin.describe()
        );
    }
    if a.inline {
        term::print_inline(picture, a.protocol).map_err(|e| e.to_string())?;
    }
    Ok(0)
}

// --- order ---

/// `order`: the parsed order and its linkage identifiers.
pub fn order(a: &OrderArgs) -> Outcome {
    let mut s = Session::new();
    open_order(&mut s, &a.order)?;
    let h = s.hl7.as_ref().ok_or("no order")?;
    let o = &h.order;
    let wl_ok = matches!(s.worklist, Some(Ok(_)));
    if a.format.json {
        let mut v = json!({
            "summary": h.summary,
            "patientId": o.patient_id,
            "patientAuthority": h.patient_authority,
            "accession": o.accession,
            "requestedProcedureId": o.procedure_id,
            "studyInstanceUid": o.study_uid,
            "studyInstanceUidSource": o.study_uid_source.map(|s| s.path()),
            "warnings": h.warnings,
            "worklistItem": wl_ok,
        });
        if a.raw {
            v["raw"] = Value::String(h.raw.clone());
        }
        return print_json(&v);
    }
    out!("{}", h.summary);
    out!("");
    out!(
        "Patient ID      {}{}",
        opt(&o.patient_id),
        h.patient_authority
            .as_deref()
            .map(|a| format!("  (authority {a})"))
            .unwrap_or_default()
    );
    out!("Accession       {}", opt(&o.accession));
    out!("Procedure ID    {}", opt(&o.procedure_id));
    out!(
        "Study UID       {}{}",
        opt(&o.study_uid),
        o.study_uid_source
            .map(|s| format!("  (from {})", s.path()))
            .unwrap_or_default()
    );
    out!(
        "Worklist item   {}",
        match &s.worklist {
            Some(Ok(_)) => "can be built (see `worklist`)".to_string(),
            Some(Err(e)) => format!("not possible: {e}"),
            None => "—".to_string(),
        }
    );
    if !h.warnings.is_empty() {
        out!("");
        out!("Warnings:");
        for w in &h.warnings {
            out!("  - {w}");
        }
    }
    if a.raw {
        out!("");
        for seg in h.raw.split(['\r', '\n']).filter(|l| !l.is_empty()) {
            out!("{seg}");
        }
    }
    Ok(0)
}

// --- link ---

fn pair_json(p: &Pair) -> Value {
    json!({"dicom": p.dicom, "hl7": p.hl7, "matches": p.matches()})
}

/// `true` when the study and order link without a key conflict.
pub fn is_linked(l: &Linkage) -> bool {
    l.path != dicomscope_core::link::LinkPath::None && l.key_conflict().is_none()
}

/// Linked, no key conflict, and the patient IDs do not disagree: what
/// `link` exits 0 for.
pub fn is_clean(l: &Linkage) -> bool {
    is_linked(l) && !l.linked_with_patient_mismatch()
}

/// `link`: the study against the order; [`EXIT_NO_LINK`] unless
/// [`is_clean`].
pub fn link(a: &LinkArgs) -> Outcome {
    let (study, order) = a.inputs.split()?;
    let mut s = open_study(&study)?;
    open_order(&mut s, &order)?;
    let l = s.linkage.clone().ok_or("no linkage computed")?;
    let chain = s.chain.clone();
    let status = if is_clean(&l) { 0 } else { EXIT_NO_LINK };
    if a.format.json {
        print_json(&json!({
            "linked": is_linked(&l),
            "clean": status == 0,
            "path": l.path.label(),
            "patientMatch": l.patient_match,
            "patientMismatch": l.linked_with_patient_mismatch(),
            "keyConflict": l.key_conflict().map(|k| k.explanation()),
            "studyInstanceUid": pair_json(&l.study_uid),
            "accessionNumber": pair_json(&l.accession),
            "patientId": pair_json(&l.patient_id),
            "requestedProcedureId": pair_json(&l.procedure_id),
            "chain": chain.as_ref().map(|c| json!({
                "rows": c.rows.iter().map(|r| json!({
                    "name": r.name,
                    "order": r.order,
                    "worklist": r.worklist,
                    "image": r.image,
                    "orderToWorklist": r.order_to_worklist(),
                    "worklistToImage": r.worklist_to_image(),
                })).collect::<Vec<_>>(),
                "findings": c.findings.iter().map(|f| f.explanation()).collect::<Vec<_>>(),
            })),
        }))?;
        return Ok(status);
    }
    out!("{}", link_text(&l, chain.as_ref()).trim_end());
    Ok(status)
}

fn mark(m: Option<bool>) -> &'static str {
    match m {
        Some(true) => "=",
        Some(false) => "≠",
        None => "·",
    }
}

/// The linkage and chain as text, shared by `link` and the viewer.
pub fn link_text(l: &Linkage, chain: Option<&dicomscope_core::link::Chain>) -> String {
    let mut t = String::new();
    let verdict = if is_linked(l) { "LINKED" } else { "NOT LINKED" };
    t.push_str(&format!("{verdict} via {}\n", l.path.label()));
    if l.linked_with_patient_mismatch() {
        t.push_str("  ! linked, but the patient IDs differ\n");
    }
    if let Some(k) = l.key_conflict() {
        t.push_str(&format!("  ! {}\n", k.explanation()));
    }
    t.push_str("\nDICOM study against HL7 order\n");
    for (name, p) in [
        ("Study Instance UID", &l.study_uid),
        ("Accession Number", &l.accession),
        ("Patient ID", &l.patient_id),
        ("Requested Procedure ID", &l.procedure_id),
    ] {
        compare_row(
            &mut t,
            name,
            &[("DICOM", &p.dicom), ("HL7", &p.hl7)],
            &[p.matches()],
        );
    }
    if let Some(c) = chain {
        t.push_str("\nOrder → worklist → image\n");
        for r in &c.rows {
            compare_row(
                &mut t,
                r.name,
                &[
                    ("order", &r.order),
                    ("worklist", &r.worklist),
                    ("image", &r.image),
                ],
                &[r.order_to_worklist(), r.worklist_to_image()],
            );
        }
        for f in &c.findings {
            t.push_str(&format!("  ! {}\n", f.explanation()));
        }
    }
    t
}

/// Widest value that still goes side by side with its neighbours.
const CELL: usize = 24;

/// One compared identifier: `name  a = b = c` when every value fits a
/// cell, else the marks on the name line and one value per line below,
/// so a 64-character UID never pushes a mark off screen.
fn compare_row(
    t: &mut String,
    name: &str,
    cells: &[(&str, &Option<String>)],
    marks: &[Option<bool>],
) {
    let fits = cells.iter().all(|(_, v)| opt(v).chars().count() <= CELL);
    if fits {
        t.push_str(&format!("  {name:<24}"));
        for (i, (_, v)) in cells.iter().enumerate() {
            match marks.get(i) {
                Some(m) => t.push_str(&format!(" {:<CELL$} {}", opt(v), mark(*m))),
                None => t.push_str(&format!(" {}", opt(v))),
            }
        }
    } else {
        let marks: Vec<&str> = marks.iter().map(|m| mark(*m)).collect();
        t.push_str(&format!("  {name:<24} {}", marks.join(" ")));
        for (label, v) in cells {
            t.push_str(&format!("\n      {label:<9}{}", opt(v)));
        }
    }
    t.push('\n');
}

// --- fhir ---

/// `fhir`: one FHIR R4 resource, or the Bundle.
pub fn fhir(a: &FhirArgs) -> Outcome {
    let (study, order) = a.inputs.split()?;
    let mut s = open_study(&study)?;
    open_order(&mut s, &order)?;
    let f = s
        .fhir
        .as_ref()
        .ok_or("no FHIR output: study or order missing")?;
    let text = match a.resource {
        FhirResource::Bundle => &f.bundle,
        FhirResource::Patient => &f.patient,
        FhirResource::ServiceRequest => &f.service_request,
        FhirResource::ImagingStudy => &f.imaging_study,
    };
    match &a.output {
        Some(p) => {
            std::fs::write(p, format!("{text}\n")).map_err(|e| format!("{}: {e}", p.display()))?
        }
        None => out!("{text}"),
    }
    Ok(0)
}

// --- worklist ---

/// `worklist`: the Modality Worklist item, printed or written as Part 10.
pub fn worklist(a: &WorklistArgs) -> Outcome {
    let mut s = Session::new();
    open_order(&mut s, &a.order)?;
    let wl = match s.worklist.take() {
        Some(Ok(w)) => w,
        Some(Err(e)) => return Err(format!("no worklist item: {e}")),
        None => return Err("no worklist item".into()),
    };
    if let Some(p) = &a.output {
        std::fs::write(p, &wl.bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        eprintln!("{}: {} bytes", p.display(), wl.bytes.len());
    }
    if a.format.json {
        return print_json(&Value::Array(
            wl.rows
                .iter()
                .map(|r| json!({"tag": r.tag, "keyword": r.keyword, "vr": r.vr, "value": r.value, "depth": r.depth}))
                .collect(),
        ));
    }
    if a.output.is_none() {
        for r in &wl.rows {
            out!(
                "{}{} {:<2} {:<36} {}",
                "  ".repeat(r.depth),
                r.tag,
                r.vr,
                r.keyword,
                r.value
            );
        }
    }
    Ok(0)
}

// --- docs ---

/// `docs`: list documents, print a report or save a PDF.
pub fn docs(a: &DocsArgs) -> Outcome {
    let mut s = open_study(&a.study)?;
    let docs = s
        .set
        .as_ref()
        .map(|x| x.documents.clone())
        .unwrap_or_default();
    let Some(n) = a.show else {
        if a.format.json {
            return print_json(&Value::Array(
                docs.iter()
                    .enumerate()
                    .map(|(i, d)| json!({"index": i + 1, "kind": d.kind.label(), "title": d.title, "seriesNumber": d.series_number}))
                    .collect(),
            ));
        }
        if docs.is_empty() {
            out!("no documents in this study");
        }
        for (i, d) in docs.iter().enumerate() {
            out!("{:>3}. {}", i + 1, d.label());
        }
        return Ok(0);
    };
    let i = n
        .checked_sub(1)
        .filter(|i| *i < docs.len())
        .ok_or_else(|| format!("document {n} does not exist; the study has {}", docs.len()))?;
    s.open_document(i);
    match s.document.take().map(|(_, c)| c) {
        Some(DocumentContent::Report { title, text }) => match &a.output {
            Some(p) => std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))?,
            None => out!("{title}\n\n{text}"),
        },
        Some(DocumentContent::Pdf { title, bytes, mime }) => {
            let p = a.output.as_ref().ok_or_else(|| {
                format!(
                    "{title} is a {mime} of {} bytes; give -o FILE to save it",
                    bytes.len()
                )
            })?;
            std::fs::write(p, &bytes).map_err(|e| format!("{}: {e}", p.display()))?;
            eprintln!("{}: {} bytes ({mime})", p.display(), bytes.len());
        }
        Some(DocumentContent::Failed(e)) => return Err(e),
        None => return Err("document not opened".into()),
    }
    Ok(0)
}

// --- gpu ---

/// `gpu`: the adapters per backend, and the one `--backend` picks.
pub fn gpu_list(g: GpuArgs) -> Outcome {
    let all = gpu::adapters();
    if all.is_empty() {
        out!("no GPU adapter found on any backend");
    }
    for a in &all {
        out!(
            "{:<8} {:<40} {:?}  {} {}",
            format!("{:?}", a.backend),
            a.name,
            a.device_type,
            a.driver,
            a.driver_info
        );
    }
    match WgpuPlugin::new(g.backend) {
        Ok(p) => out!("\n--backend {:?} uses: {}", g.backend, p.describe()),
        Err(e) => out!("\n--backend {:?}: {e}", g.backend),
    }
    Ok(0)
}
