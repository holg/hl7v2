//! dicomscope: a browser-only DICOM viewer that links a study to its HL7 v2
//! order. This is the demo application for the `hl7kit` crate.
//!
//! No hand-written JavaScript, no network I/O after page load, no `unwrap()`
//! on user data. See the README for the guarantees and how to verify them.

#![forbid(unsafe_code)]
#![warn(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// The domain modules are consumed by the browser app only; on the host they
// exist to be tested.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code, unused_imports))]

mod dicom;
mod error;
mod fhir;
mod link;
mod measure;
#[cfg(not(target_arch = "wasm32"))]
mod order_gen;
mod thumbnail;
mod view;

#[cfg(target_arch = "wasm32")]
mod app;
mod render;
#[cfg(target_arch = "wasm32")]
mod ui;

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}

/// On the host the binary is a command-line companion built on the same
/// modules the browser uses:
///
/// * `dicomscope <file.dcm | folder | study.zip>...` checks that every input
///   loads, scans into series and decodes, and prints what it found;
/// * `dicomscope order <folder | study.zip | file.dcm> [options]` writes an
///   HL7 ORM^O01 that matches the study, then parses it back with the
///   `hl7kit` crate and reports the linkage. That is how the sample orders in
///   `samples/` are produced, and it is a round trip through both crates.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let subcommand = match args.first().map(String::as_str) {
        Some("order") => Some(order_command as fn(&[String]) -> Result<(), String>),
        Some("pack") => Some(pack_command as fn(&[String]) -> Result<(), String>),
        Some("fhir") => Some(fhir_command as fn(&[String]) -> Result<(), String>),
        _ => None,
    };
    if let Some(run) = subcommand {
        args.remove(0);
        match run(&args) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
    let paths = args;
    if paths.is_empty() {
        eprintln!("dicomscope is a browser application; build it with `trunk build --release`.");
        eprintln!("Host usage:");
        eprintln!("  dicomscope <file.dcm | folder | study.zip>...   check inputs with the browser's code path");
        eprintln!("  dicomscope order <folder | study.zip | file.dcm> [--patient-id ID] [--accession ACC]");
        eprintln!("                   [--procedure-id ID] [--control-id ID] [-o out.hl7]");
        eprintln!("                                                  write a matching HL7 ORM^O01 and verify the link");
        eprintln!("  dicomscope fhir <folder | study.zip | file.dcm> <order.hl7> [-o bundle.json]");
        eprintln!("                                                  emit the FHIR R4 bundle the browser would build");
        eprintln!("  dicomscope pack <folder | study.zip> -o out.zip [--series N,N] [--every K]");
        eprintln!("                                                  rewrite as a clean deflated zip of image instances only");
        eprintln!("  DICOMSCOPE_TAGS=1 prints every tag of checked files");
        std::process::exit(2);
    }
    let mut failures = 0;
    for path in &paths {
        let is_dir = std::path::Path::new(path).is_dir();
        let is_zip = std::fs::read(path)
            .map(|b| dicom::series::is_zip(&b))
            .unwrap_or(false);
        let result = if is_dir || is_zip {
            check_set(path)
        } else {
            check(path)
        };
        match result {
            Ok(line) => println!("ok    {path}: {line}"),
            Err(e) => {
                failures += 1;
                println!("error {path}: {e}");
            }
        }
    }
    if failures > 0 {
        std::process::exit(1);
    }
}

/// Scan a folder or zip the way the browser does and describe the series.
#[cfg(not(target_arch = "wasm32"))]
fn check_set(path: &str) -> Result<String, String> {
    let set = dicom::StudySet::scan(collect_inputs(path)?);
    if set.is_empty() {
        return Err(format!(
            "no displayable image among {} file(s); first reason: {}",
            set.skipped.len(),
            set.skipped
                .first()
                .map(|s| format!("{}: {}", s.name, s.reason))
                .unwrap_or_default()
        ));
    }
    let mut out = format!(
        "{} file(s), {} series, {} slice(s), {} skipped, study {:?}",
        set.files.len(),
        set.series.len(),
        set.slice_count(),
        set.skipped.len(),
        set.study_uid.as_deref().unwrap_or("")
    );
    for s in &set.series {
        let first = &s.slices[0];
        let last = &s.slices[s.slices.len() - 1];
        out.push_str(&format!(
            "\n        {} {}x{}  first {:?} @ {:?}  last {:?} @ {:?}",
            s.label(),
            s.cols,
            s.rows,
            set.files[first.file].name,
            first.position,
            set.files[last.file].name,
            last.position
        ));
    }
    for sk in &set.skipped {
        out.push_str(&format!("\n        skipped {}: {}", sk.name, sk.reason));
    }
    Ok(out)
}
#[cfg(not(target_arch = "wasm32"))]
fn check(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let obj = dicom::load(&bytes).map_err(|e| e.to_string())?;
    let study = dicom::Study::from_object(&obj);
    let rows = dicom::tag_rows(&obj);
    if std::env::var_os("DICOMSCOPE_TAGS").is_some() {
        for r in &rows {
            println!(
                "      {}{} {} {} {}",
                "  ".repeat(r.depth),
                r.tag,
                r.keyword,
                r.vr,
                r.value
            );
        }
    }
    let tags = rows.len();
    if let Some(kind) = dicom::sr::document_kind(&obj) {
        let title = dicom::sr::document_title(&obj);
        return Ok(match kind {
            dicom::sr::DocumentKind::EncapsulatedPdf => {
                let (bytes, mime) = dicom::sr::encapsulated_document(&obj)
                    .ok_or("Encapsulated PDF without an Encapsulated Document element")?;
                format!(
                    "{} {title:?}: {} bytes of {mime}, tags {tags}",
                    kind.label(),
                    bytes.len()
                )
            }
            _ => {
                let lines = dicom::sr::render_sr(&obj);
                format!(
                    "{} {title:?}: {} content items, tags {tags}\n{}",
                    kind.label(),
                    lines.len().saturating_sub(1),
                    dicom::sr::sr_to_text(&lines)
                        .lines()
                        .map(|l| format!("        {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        });
    }
    let frame = dicom::decode_frame(&obj, 0).map_err(|e| {
        format!(
            "{e} [ts {} {}]",
            study.transfer_syntax,
            dicom::transfer_syntax::name(&study.transfer_syntax)
        )
    })?;
    let mut line = format!(
        "{} ({}) {}x{} {}-bit {}{} range {:.0}..{:.0} rescale {}/{} window {} tags {} patient {:?} accession {:?} procedure {:?} study {:?}",
        dicom::transfer_syntax::name(&study.transfer_syntax),
        study.transfer_syntax,
        frame.width,
        frame.height,
        frame.bits_stored,
        frame.photometric,
        if frame.pixels.is_color() { " (colour)" } else { "" },
        frame.value_range.0,
        frame.value_range.1,
        frame.rescale.0,
        frame.rescale.1,
        frame
            .default_window
            .map(|(c, w)| format!("{c:.0}/{w:.0}"))
            .unwrap_or_else(|| "none".into()),
        tags,
        study.patient_id.as_deref().unwrap_or(""),
        study.accession_number.as_deref().unwrap_or(""),
        study.requested_procedure_id.as_deref().unwrap_or(""),
        study.study_uid.as_deref().unwrap_or(""),
    );
    if frame.frame_count > 1 {
        line.push_str(&format!(" (frame 1 of {})", frame.frame_count));
    }
    Ok(line)
}

/// `dicomscope order <path> [--patient-id ID] [--accession ACC]
/// [--procedure-id ID] [--control-id ID] [-o out.hl7]`
///
/// Reads one instance header from the study (streaming the first `.dcm`
/// entry out of a zip, so a 500 MB archive is not loaded), builds the order
/// with the `hl7kit` builder, parses it back, extracts the order fields and
/// resolves the linkage against the study. The message goes to `-o` or
/// stdout; the verification goes to stderr.
#[cfg(not(target_arch = "wasm32"))]
fn order_command(args: &[String]) -> Result<(), String> {
    let mut path = None;
    let mut out = None;
    let mut details = order_gen::OrderDetails::default();
    let mut i = 0;
    while i < args.len() {
        let next = |i: usize| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "--patient-id" => {
                details.patient_id = Some(next(i)?);
                i += 1;
            }
            "--accession" => {
                details.accession = Some(next(i)?);
                i += 1;
            }
            "--procedure-id" => {
                details.procedure_id = Some(next(i)?);
                i += 1;
            }
            "--control-id" => {
                details.control_id = Some(next(i)?);
                i += 1;
            }
            "-o" | "--out" => {
                out = Some(next(i)?);
                i += 1;
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            p if path.is_none() => path = Some(p.to_string()),
            extra => return Err(format!("unexpected argument {extra}")),
        }
        i += 1;
    }
    let path = path.ok_or("order needs a folder, zip or DICOM file")?;

    let (name, bytes) = first_instance(&path)?;
    let obj = dicom::load::load_header(&bytes).map_err(|e| format!("{name}: {e}"))?;
    let study = dicom::Study::from_object(&obj);
    use dicom::study::string;
    use dicom_dictionary_std::tags;
    details.patient_name = details
        .patient_name
        .or_else(|| string(&obj, tags::PATIENT_NAME));
    details.birth_date = string(&obj, tags::PATIENT_BIRTH_DATE);
    details.sex = string(&obj, tags::PATIENT_SEX);
    details.study_datetime = string(&obj, tags::STUDY_DATE).map(|d| {
        let t = string(&obj, tags::STUDY_TIME).unwrap_or_default();
        let t: String = t
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(6)
            .collect();
        format!("{d}{t}")
    });
    details.procedure_description =
        string(&obj, tags::STUDY_DESCRIPTION).or_else(|| string(&obj, tags::PROTOCOL_NAME));

    let text = order_gen::order_message(&study, &details);

    // Round trip: what we wrote must parse, and the link must resolve.
    let msg = hl7kit::Message::parse(&text)
        .map_err(|e| format!("generated message does not parse: {e}"))?;
    if !msg.warnings().is_empty() {
        return Err(format!(
            "generated message has warnings: {:?}",
            msg.warnings()
        ));
    }
    let order = hl7kit::order::Order::extract(&msg);
    let linkage = link::resolve(&study, &order);
    eprintln!(
        "read {name}: patient {:?}, accession {:?}, procedure {:?}, study {:?}",
        study.patient_id.as_deref().unwrap_or(""),
        study.accession_number.as_deref().unwrap_or(""),
        study.requested_procedure_id.as_deref().unwrap_or(""),
        study.study_uid.as_deref().unwrap_or("")
    );
    eprintln!(
        "generated {} bytes, {} segments, {}; link: {}; patient match: {}",
        text.len(),
        msg.segment_count(),
        msg.message_type()
            .map(|t| format!("{}^{}", t.code, t.trigger))
            .unwrap_or_default(),
        linkage.path.label(),
        match linkage.patient_match {
            Some(true) => "yes",
            Some(false) => "NO (mismatch)",
            None => "not comparable",
        }
    );
    if linkage.path == link::LinkPath::None {
        return Err("the generated order does not link back to the study".into());
    }

    match out {
        Some(file) => {
            std::fs::write(&file, text.as_bytes()).map_err(|e| format!("{file}: {e}"))?;
            eprintln!("wrote {file}");
        }
        None => {
            use std::io::Write;
            std::io::stdout()
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The first DICOM instance in a folder, zip or single file, read without
/// loading anything else.
#[cfg(not(target_arch = "wasm32"))]
fn first_instance(path: &str) -> Result<(String, Vec<u8>), String> {
    let root = std::path::Path::new(path);
    if root.is_dir() {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut children: Vec<_> = std::fs::read_dir(&dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect();
            children.sort();
            children.reverse(); // so the stack pops in sorted order
            for child in children {
                if child.is_dir() {
                    stack.push(child);
                } else if let Ok(bytes) = std::fs::read(&child) {
                    if dicom::load::load_header(&bytes).is_ok() {
                        return Ok((child.display().to_string(), bytes));
                    }
                }
            }
        }
        return Err(format!("{path}: no DICOM instance found"));
    }
    let file = std::fs::File::open(root).map_err(|e| format!("{path}: {e}"))?;
    let mut magic = [0u8; 4];
    {
        use std::io::Read;
        let mut probe = &file;
        probe
            .read_exact(&mut magic)
            .map_err(|e| format!("{path}: {e}"))?;
    }
    if dicom::series::is_zip(&magic) {
        let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{path}: {e}"))?;
        let mut names: Vec<(usize, String)> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| (i, e.name().to_string())))
            .filter(|(_, n)| !n.ends_with('/'))
            .collect();
        names.sort_by(|a, b| a.1.cmp(&b.1));
        for (i, name) in names {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            use std::io::Read;
            entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if dicom::load::load_header(&bytes).is_ok() {
                return Ok((format!("{path}/{name}"), bytes));
            }
        }
        return Err(format!("{path}: no DICOM instance in the archive"));
    }
    let bytes = std::fs::read(root).map_err(|e| format!("{path}: {e}"))?;
    Ok((path.to_string(), bytes))
}

/// Every file under a folder (recursively, sorted), or the single file at
/// `path`, as scan inputs. A zip is handed over as one input; the scanner
/// reads it entry by entry.
#[cfg(not(target_arch = "wasm32"))]
fn collect_inputs(path: &str) -> Result<Vec<dicom::FileEntry>, String> {
    let root = std::path::Path::new(path);
    if !root.is_dir() {
        return Ok(vec![dicom::FileEntry::new(
            root.display().to_string(),
            std::fs::read(root).map_err(|e| format!("{path}: {e}"))?,
        )]);
    }
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        children.sort();
        children.reverse();
        for child in children {
            if child.is_dir() {
                stack.push(child);
            } else if let Ok(bytes) = std::fs::read(&child) {
                entries.push(dicom::FileEntry::new(
                    child
                        .strip_prefix(root)
                        .unwrap_or(&child)
                        .display()
                        .to_string(),
                    bytes,
                ));
            }
        }
    }
    Ok(entries)
}

/// `dicomscope pack <folder | study.zip> -o out.zip [--series N,N] [--every K]`
///
/// Writes a clean, deflated zip containing only the image instances the
/// scanner accepted: no OS metadata, no DICOMDIR, no stray files. Entries are
/// named `series-<number>/<original file name>` in slice order. `--series`
/// keeps only the listed series numbers; `--every K` keeps every K-th slice
/// of each series (the first is always kept). Pixel data is copied as is.
#[cfg(not(target_arch = "wasm32"))]
fn pack_command(args: &[String]) -> Result<(), String> {
    let mut path = None;
    let mut out = None;
    let mut series_filter: Option<Vec<i32>> = None;
    let mut every = 1usize;
    let mut i = 0;
    while i < args.len() {
        let next = |i: usize| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "-o" | "--out" => {
                out = Some(next(i)?);
                i += 1;
            }
            "--series" => {
                let list = next(i)?;
                series_filter = Some(
                    list.split(',')
                        .map(|s| {
                            s.trim()
                                .parse::<i32>()
                                .map_err(|_| format!("bad series number {s:?}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                );
                i += 1;
            }
            "--every" => {
                every = next(i)?
                    .parse::<usize>()
                    .ok()
                    .filter(|k| *k >= 1)
                    .ok_or("--every needs a positive integer")?;
                i += 1;
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            p if path.is_none() => path = Some(p.to_string()),
            extra => return Err(format!("unexpected argument {extra}")),
        }
        i += 1;
    }
    let path = path.ok_or("pack needs a folder or zip")?;
    let out = out.ok_or("pack needs -o <out.zip>")?;

    let set = dicom::StudySet::scan(collect_inputs(&path)?);
    if set.is_empty() {
        return Err(format!("{path}: no displayable DICOM image found"));
    }
    eprintln!(
        "scanned {}: {} series, {} slices, {} skipped, {} metadata entries ignored",
        path,
        set.series.len(),
        set.slice_count(),
        set.skipped.len(),
        set.ignored_metadata
    );

    let file = std::fs::File::create(&out).map_err(|e| format!("{out}: {e}"))?;
    let mut writer = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .large_file(true);
    let mut written = 0usize;
    let mut bytes_in = 0usize;
    for series in &set.series {
        if let Some(keep) = &series_filter {
            if !series.number.map(|n| keep.contains(&n)).unwrap_or(false) {
                continue;
            }
        }
        let dir = match series.number {
            Some(n) => format!("series-{n:03}"),
            None => "series-unnumbered".to_string(),
        };
        // Multi-frame files appear once per frame in `slices`; write each
        // file once, in slice order, thinning by `every`.
        let mut seen = std::collections::BTreeSet::new();
        let mut kept = 0usize;
        for (i, slice) in series.slices.iter().enumerate() {
            if i % every != 0 || !seen.insert(slice.file) {
                continue;
            }
            let entry = &set.files[slice.file];
            let base = entry.name.rsplit(['/', '\\']).next().unwrap_or(&entry.name);
            let bytes = set.bytes(slice.file)?;
            bytes_in += bytes.len();
            writer
                .start_file(format!("{dir}/{base}"), opts)
                .map_err(|e| format!("{out}: {e}"))?;
            use std::io::Write;
            writer
                .write_all(&bytes)
                .map_err(|e| format!("{out}: {e}"))?;
            written += 1;
            kept += 1;
        }
        eprintln!("  {} -> {kept} file(s) in {dir}/", series.label());
    }
    let mut inner = writer.finish().map_err(|e| format!("{out}: {e}"))?;
    use std::io::Write;
    inner.flush().map_err(|e| format!("{out}: {e}"))?;
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "wrote {out}: {written} file(s), {:.1} MB in, {:.1} MB out",
        bytes_in as f64 / 1e6,
        size as f64 / 1e6
    );
    if written == 0 {
        return Err("nothing matched the filters; the zip is empty".into());
    }
    Ok(())
}

/// `dicomscope fhir <study> <order.hl7> [-o bundle.json]`
///
/// Exactly what the browser does after both files are loaded: scan the
/// study, read the study header from its first slice, parse the order,
/// resolve the linkage, and emit the FHIR R4 bundle. The bundle goes to
/// `-o` or stdout; what was linked and from where goes to stderr, so the
/// JSON can be piped into a validator.
#[cfg(not(target_arch = "wasm32"))]
fn fhir_command(args: &[String]) -> Result<(), String> {
    let mut positional = Vec::new();
    let mut out = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                out = Some(args.get(i + 1).cloned().ok_or("-o needs a file name")?);
                i += 1;
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            p => positional.push(p.to_string()),
        }
        i += 1;
    }
    let [study_path, order_path] = positional.as_slice() else {
        return Err("fhir needs <study> and <order.hl7>".into());
    };

    let set = dicom::StudySet::scan(collect_inputs(study_path)?);
    let Some((_, first)) = set.slice(0, 0) else {
        return Err(format!("{study_path}: no image series found"));
    };
    let bytes = set.bytes(first.file)?;
    let obj = dicom::load(&bytes).map_err(|e| e.to_string())?;
    let study = dicom::Study::from_object(&obj);

    let text = std::fs::read(order_path).map_err(|e| format!("{order_path}: {e}"))?;
    let msg = hl7kit::Message::parse_lossy(&text).map_err(|e| format!("{order_path}: {e}"))?;
    let order = hl7kit::order::Order::extract(&msg);
    let linkage = link::resolve(&study, &order);
    let input = fhir::FhirInput {
        study: &study,
        series: &set.series,
        message: &msg,
        order: &order,
        linkage: &linkage,
    };
    let bundle = fhir::bundle(&input);
    let json = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;

    eprintln!(
        "study {:?}: {} series, {} slices; order {}: study UID from {}, accession from {}",
        set.study_uid.as_deref().unwrap_or(""),
        set.series.len(),
        set.slice_count(),
        msg.message_type()
            .map(|t| format!("{}^{}", t.code, t.trigger))
            .unwrap_or_default(),
        order
            .source_path(hl7kit::order::OrderField::StudyUid)
            .unwrap_or("nowhere"),
        order
            .source_path(hl7kit::order::OrderField::Accession)
            .unwrap_or("nowhere"),
    );
    eprintln!(
        "link: {}; patient match: {}; basedOn {}; {} bytes of JSON",
        linkage.path.label(),
        match linkage.patient_match {
            Some(true) => "yes",
            Some(false) => "NO (mismatch)",
            None => "not comparable",
        },
        if linkage.path == link::LinkPath::None {
            "omitted"
        } else {
            "asserted"
        },
        json.len()
    );
    for w in &order.warnings {
        eprintln!("order warning: {w}");
    }
    match out {
        Some(file) => {
            std::fs::write(&file, json.as_bytes()).map_err(|e| format!("{file}: {e}"))?;
            eprintln!("wrote {file}");
        }
        None => println!("{json}"),
    }
    Ok(())
}
