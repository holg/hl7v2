//! Every example in the official R4 examples package, read and written
//! back: no issues, and the JSON that comes out equals the JSON that went
//! in (key order aside).
//!
//! The package is not committed (192 MB). Point `FHIRKIT_R4_EXAMPLES` at
//! its unpacked `package/` folder, or unpack it to
//! `tmp/fhir/examples/package` at the repository root; without it the
//! test says so and passes.

use fhirkit::r4::Resource;
use fhirkit::{decode_json, Json};
use std::path::PathBuf;

fn examples_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FHIRKIT_R4_EXAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tmp/fhir/examples/package"
            ))
        });
    dir.is_dir().then_some(dir)
}

/// Examples in the official package that are themselves invalid R4, and
/// what fhirkit rightly says about them. Anything else is a failure.
const KNOWN_INVALID: &[(&str, &str)] = &[
    // Codes carrying a mis-encoded no-break space.
    ("Bundle-v2-valuesets.json", "is not a valid code"),
    ("CodeSystem-v2-0550.json", "is not a valid code"),
    // Items without the required linkId.
    (
        "Questionnaire-qs1.json",
        ".linkId: required element missing",
    ),
    // ImplementationGuide.name and .status are 1..1.
    ("ImplementationGuide-fhir.json", "required element missing"),
    ("ig-r4.json", "required element missing"),
    // SearchParameter.base is 1..*.
    (
        "SearchParameter-codesystem-extensions-CodeSystem-author.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-codesystem-extensions-CodeSystem-effective.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-codesystem-extensions-CodeSystem-end.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-codesystem-extensions-CodeSystem-keyword.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-codesystem-extensions-CodeSystem-workflow.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-valueset-extensions-ValueSet-author.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-valueset-extensions-ValueSet-effective.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-valueset-extensions-ValueSet-end.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-valueset-extensions-ValueSet-keyword.json",
        "SearchParameter.base: required",
    ),
    (
        "SearchParameter-valueset-extensions-ValueSet-workflow.json",
        "SearchParameter.base: required",
    ),
];

/// The input minus what fhirkit drops on purpose: `fhir_comments`.
fn without_comments(v: &Json) -> Json {
    match v {
        Json::Object(m) => Json::Object(
            m.iter()
                .filter(|(k, _)| k != "fhir_comments")
                .map(|(k, v)| (k.clone(), without_comments(v)))
                .collect(),
        ),
        Json::Array(a) => Json::Array(a.iter().map(without_comments).collect()),
        other => other.clone(),
    }
}

#[test]
fn every_r4_example_reads_cleanly_and_round_trips() {
    let Some(dir) = examples_dir() else {
        eprintln!("skipped: no R4 examples package (see the module docs)");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|e| e == "json")
                && !p.ends_with("package.json")
                && !p.ends_with(".index.json")
        })
        .collect();
    files.sort();
    let (mut read, mut with_issues, mut changed) = (0, Vec::new(), Vec::new());
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let Ok(json) = Json::parse(&text) else {
            with_issues.push(format!("{}: not JSON", path.display()));
            continue;
        };
        if json.get("resourceType").is_none() {
            continue;
        }
        read += 1;
        let (resource, issues) = decode_json::<Resource>(&json);
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if let Some((_, expect)) = KNOWN_INVALID.iter().find(|(f, _)| *f == name) {
            // The known problem must be found, and nothing else.
            assert!(
                !issues.is_empty(),
                "{name}: the known problem was not reported"
            );
            let other: Vec<String> = issues
                .iter()
                .map(ToString::to_string)
                .filter(|i| !i.contains(expect))
                .collect();
            assert!(other.is_empty(), "{name}: unexpected issues: {other:?}");
            continue;
        }
        if !issues.is_empty() {
            with_issues.push(format!(
                "{name}: {}",
                issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        if let Some(r) = resource {
            if !fhirkit::FhirJson::encode(&r).same_as(&without_comments(&json)) {
                changed.push(name);
            }
        }
    }
    eprintln!(
        "{read} examples read, {} with issues, {} changed on the way back",
        with_issues.len(),
        changed.len()
    );
    for w in with_issues.iter().take(40) {
        eprintln!("  issue  {w}");
    }
    for c in changed.iter().take(40) {
        eprintln!("  change {c}");
    }
    assert!(read > 2000, "only {read} examples found");
    assert!(with_issues.is_empty() && changed.is_empty());
}
