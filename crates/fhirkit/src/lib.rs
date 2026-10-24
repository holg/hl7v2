//! FHIR R4 for Rust.
//!
//! Typed models for every R4 resource and datatype, generated from the
//! official specification package, read from and written to JSON without
//! losing anything (decimal precision, extensions on primitives, key
//! order), and checked while reading: every problem comes back as an
//! [`Issue`] with the element's path, not as the first error only.
//!
//! ```
//! use fhirkit::r4::{Patient, Resource};
//!
//! let json = r#"{
//!   "resourceType": "Patient",
//!   "id": "example",
//!   "name": [{"family": "Chalmers", "given": ["Peter", "James"]}],
//!   "birthDate": "1974-12-25",
//!   "_birthDate": {"extension": [{"url": "http://hl7.org/fhir/StructureDefinition/patient-birthTime",
//!                                  "valueDateTime": "1974-12-25T14:35:45-05:00"}]}
//! }"#;
//!
//! let patient: Patient = fhirkit::from_json_strict(json).unwrap();
//! assert_eq!(patient.name[0].family.as_ref().unwrap().value.as_deref(), Some("Chalmers"));
//! assert_eq!(patient.birth_date.as_ref().unwrap().extension.len(), 1);
//!
//! // Any resource, dispatched on resourceType; problems are reported with paths.
//! let (resource, issues) = fhirkit::from_json::<Resource>(r#"{"resourceType": "Patient", "gender": "x y ", "foo": 1}"#).unwrap();
//! assert_eq!(resource.unwrap().resource_type(), "Patient");
//! let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
//! assert_eq!(paths, ["Patient.gender", "Patient.foo"]);
//! ```
//!
//! Resources come in five Cargo features, as the specification groups
//! them: `foundation`, `base`, `clinical`, `financial`, `specialized`
//! (all on by default). Datatypes are always included.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod decode;
pub mod json;
pub mod primitive;
#[allow(missing_docs)]
pub mod r4;

pub use decode::{Decoder, FhirJson, Issue, Severity};
pub use json::{Json, JsonError};
pub use primitive::{Decimal, Kind, Primitive};

/// Why a document could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not JSON.
    Json(JsonError),
    /// JSON, but with problems (only from [`from_json_strict`]).
    Invalid(Vec<Issue>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Json(e) => write!(f, "not JSON: {e}"),
            Error::Invalid(issues) => {
                write!(f, "{} problem(s)", issues.len())?;
                for i in issues {
                    write!(f, "\n  {i}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

/// What reading gives back: the value, if it was usable at all, and every
/// problem found on the way.
pub type Decoded<T> = (Option<T>, Vec<Issue>);

/// Read a value from FHIR JSON. Returns what could be read and every
/// problem found; `None` when the document was not usable at all.
pub fn from_json<T: FhirJson>(text: &str) -> Result<Decoded<T>, JsonError> {
    let json = Json::parse(text)?;
    Ok(decode_json(&json))
}

/// Read a value from parsed JSON, as [`from_json`].
pub fn decode_json<T: FhirJson>(json: &Json) -> Decoded<T> {
    let root = json
        .get("resourceType")
        .and_then(Json::as_str)
        .unwrap_or(T::TYPE_NAME);
    let mut d = Decoder::new(root);
    let v = T::decode(json, &mut d);
    (v, d.issues)
}

/// Read a value from FHIR JSON, failing on any error-level issue.
pub fn from_json_strict<T: FhirJson>(text: &str) -> Result<T, Error> {
    let (v, issues) = from_json::<T>(text).map_err(Error::Json)?;
    let errors: Vec<Issue> = issues
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    match v {
        Some(v) if errors.is_empty() => Ok(v),
        _ => Err(Error::Invalid(errors)),
    }
}

/// Write a value as compact FHIR JSON.
pub fn to_json<T: FhirJson>(value: &T) -> String {
    value.encode().to_compact()
}

/// Write a value as FHIR JSON indented by two spaces.
pub fn to_json_pretty<T: FhirJson>(value: &T) -> String {
    value.encode().to_pretty()
}

/// The README's examples, compiled and run as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

/// Read many documents at once, on up to `threads` threads (`0` means one
/// per available core). Results come back in input order, each as
/// [`from_json`] would return it.
///
/// Threads take the next document from a shared counter, so a few very
/// large Bundles cannot pile up on one thread. On WebAssembly, which has no
/// threads in the browser, this reads on the calling thread.
pub fn from_json_many<T: FhirJson + Send>(
    texts: &[&str],
    threads: usize,
) -> Vec<Result<Decoded<T>, JsonError>> {
    parallel_map(texts, threads, |t| from_json::<T>(t))
}

#[cfg(not(target_arch = "wasm32"))]
fn parallel_map<I: Sync, R: Send>(
    items: &[I],
    threads: usize,
    f: impl Fn(&I) -> R + Sync,
) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let threads = if threads == 0 {
        std::thread::available_parallelism().map_or(1, usize::from)
    } else {
        threads
    }
    .min(items.len());
    if threads <= 1 {
        return items.iter().map(&f).collect();
    }
    let next = AtomicUsize::new(0);
    let (f, next) = (&f, &next);
    let mut out: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(move || {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        mine.push((i, f(item)));
                    }
                    mine
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| match w.join() {
                Ok(v) => v,
                // A panic in `f` is a bug; carry it to the caller.
                Err(p) => std::panic::resume_unwind(p),
            })
            .collect()
    });
    out.sort_unstable_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

#[cfg(target_arch = "wasm32")]
fn parallel_map<I, R>(items: &[I], _threads: usize, f: impl Fn(&I) -> R) -> Vec<R> {
    items.iter().map(f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r4::Resource;

    #[test]
    fn many_documents_come_back_in_order_with_their_issues() {
        let docs: Vec<String> = (0..200)
            .map(|i| match i % 3 {
                0 => format!(r#"{{"resourceType":"Patient","id":"p{i}"}}"#),
                1 => format!(r#"{{"resourceType":"Patient","id":"p{i}","gender":"x y "}}"#),
                _ => "not json".to_string(),
            })
            .collect();
        let refs: Vec<&str> = docs.iter().map(String::as_str).collect();
        for threads in [1, 4, 0] {
            let out = from_json_many::<Resource>(&refs, threads);
            assert_eq!(out.len(), 200);
            for (i, r) in out.iter().enumerate() {
                match i % 3 {
                    0 => {
                        let (r, issues) = r.as_ref().unwrap();
                        assert_eq!(r.as_ref().unwrap().id(), Some(format!("p{i}").as_str()));
                        assert!(issues.is_empty());
                    }
                    1 => assert_eq!(r.as_ref().unwrap().1.len(), 1),
                    _ => assert!(r.is_err()),
                }
            }
        }
    }
}
