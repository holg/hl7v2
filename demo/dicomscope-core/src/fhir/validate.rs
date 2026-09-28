//! Structural checks on a generated bundle: the invariants FHIR R4 fixes
//! for any `collection` bundle and the ones this mapping promises, checked
//! in-process so a bad bundle is caught by `cargo test` and by the CLI
//! before it is handed to the HL7 validator. This is not a FHIR validator:
//! no profiles, no terminology, no cardinality tables. It is the set of
//! rules a generator can break by accident.
//!
//! Rules:
//! - `Bundle.type` is `collection`, `entry` is non-empty, every entry has
//!   a `fullUrl` of the form `urn:uuid:<lower-case RFC 9562 UUID>`, no two
//!   the same, and a `resource` with `resourceType` and `id`.
//! - Every `reference` resolves to a `fullUrl` in this bundle. Absolute
//!   URLs are accepted but this mapping never emits them.
//! - No empty string, empty array, empty object or `null` anywhere
//!   (FHIR JSON forbids all four).
//! - No string carries a control character other than tab, LF, CR.
//! - Every `system` and `fullUrl` is a URI; every `urn:oid:` value is a
//!   well-formed OID; every `urn:uuid:` value a UUID.
//! - `birthDate` is a FHIR `date`; `authoredOn` and `started` are FHIR
//!   `dateTime`s: a time of day always carries seconds and an offset.
//! - `gender`, `status` and `intent` are in their R4 value sets.
//! - `ImagingStudy.numberOfSeries` and `numberOfInstances` agree with the
//!   listed series and instances; each series' count with its instances.

use super::ids::{is_uid, is_uri};
use serde_json::Value;

/// One broken rule, with the JSON path it was found at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Every broken rule, in document order; empty when the bundle passes.
pub fn check(bundle: &Value) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut push = |path: String, message: String| out.push(Problem { path, message });

    if bundle["resourceType"] != "Bundle" {
        push("Bundle.resourceType".into(), "must be `Bundle`".into());
    }
    if bundle["type"] != "collection" {
        push("Bundle.type".into(), "must be `collection`".into());
    }
    let Some(entries) = bundle["entry"].as_array() else {
        push("Bundle.entry".into(), "missing".into());
        return out;
    };
    if entries.is_empty() {
        push("Bundle.entry".into(), "empty".into());
    }

    // fullUrls first, so references can be resolved in one pass.
    let mut full_urls: Vec<&str> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let path = format!("Bundle.entry[{i}].fullUrl");
        match e["fullUrl"].as_str() {
            Some(u) if is_uuid_urn(u) => {
                if full_urls.contains(&u) {
                    push(path, format!("duplicate fullUrl {u}"));
                }
                full_urls.push(u);
            }
            Some(u) => push(path, format!("not a urn:uuid: {u}")),
            None => push(path, "missing".into()),
        }
        let r = &e["resource"];
        let rpath = format!("Bundle.entry[{i}].resource");
        if !r.is_object() {
            push(rpath, "missing".into());
            continue;
        }
        if r["resourceType"].as_str().is_none_or(str::is_empty) {
            push(format!("{rpath}.resourceType"), "missing".into());
        }
        if r["id"].as_str().is_none_or(|id| !is_fhir_id(id)) {
            push(format!("{rpath}.id"), "missing or not an id".into());
        }
    }

    let mut ctx = Walk {
        full_urls: &full_urls,
        problems: Vec::new(),
    };
    for (i, e) in entries.iter().enumerate() {
        let r = &e["resource"];
        let rtype = r["resourceType"].as_str().unwrap_or("?");
        let path = format!("Bundle.entry[{i}].resource<{rtype}>");
        ctx.walk(&path, r, None);
        ctx.resource_rules(&path, rtype, r);
    }
    out.extend(ctx.problems);
    out
}

/// `check`, as a `Result` for callers that only want pass or fail.
pub fn ensure(bundle: &Value) -> Result<(), Vec<Problem>> {
    let problems = check(bundle);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

struct Walk<'a> {
    full_urls: &'a [&'a str],
    problems: Vec<Problem>,
}

impl Walk<'_> {
    fn problem(&mut self, path: &str, message: impl Into<String>) {
        self.problems.push(Problem {
            path: path.to_string(),
            message: message.into(),
        });
    }

    /// Recursive pass over one resource. `key` is the name the value sits
    /// under, for the keyword rules.
    fn walk(&mut self, path: &str, v: &Value, key: Option<&str>) {
        match v {
            Value::Null => self.problem(path, "null is not allowed in FHIR JSON"),
            Value::Bool(_) | Value::Number(_) => {}
            Value::String(s) => self.string(path, key, s),
            Value::Array(items) => {
                if items.is_empty() {
                    self.problem(path, "empty array");
                }
                for (i, item) in items.iter().enumerate() {
                    self.walk(&format!("{path}[{i}]"), item, key);
                }
            }
            Value::Object(map) => {
                if map.is_empty() {
                    self.problem(path, "empty object");
                }
                for (k, item) in map {
                    self.walk(&format!("{path}.{k}"), item, Some(k));
                }
            }
        }
    }

    fn string(&mut self, path: &str, key: Option<&str>, s: &str) {
        if s.is_empty() {
            self.problem(path, "empty string");
            return;
        }
        if s.chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
        {
            self.problem(path, "control character in string");
        }
        if s != s.trim() {
            self.problem(path, "leading or trailing whitespace");
        }
        match key {
            Some("system") | Some("fullUrl") | Some("profile") | Some("url") => {
                if !is_uri(s.split('|').next().unwrap_or(s)) {
                    self.problem(path, format!("not a URI: {s}"));
                }
            }
            Some("reference") => {
                // A `urn:uuid:` reference can only mean an entry of this
                // bundle; another absolute URI points at a server and is
                // left alone; anything relative has nothing to resolve
                // against.
                let resolves =
                    self.full_urls.contains(&s) || (!s.starts_with("urn:uuid:") && is_uri(s));
                if !resolves {
                    self.problem(path, format!("unresolved reference {s}"));
                }
            }
            Some("birthDate") => {
                if !is_fhir_date(s) {
                    self.problem(path, format!("not a FHIR date: {s}"));
                }
            }
            Some("authoredOn") | Some("started") => {
                if !is_fhir_datetime(s) {
                    self.problem(path, format!("not a FHIR dateTime: {s}"));
                }
            }
            Some("gender") => {
                if !["male", "female", "other", "unknown"].contains(&s) {
                    self.problem(path, format!("not an AdministrativeGender: {s}"));
                }
            }
            Some("intent") => {
                if !REQUEST_INTENT.contains(&s) {
                    self.problem(path, format!("not a RequestIntent: {s}"));
                }
            }
            Some("uid") if !is_uid(s) => {
                self.problem(path, format!("not a DICOM UID: {s}"));
            }
            _ => {}
        }
        if let Some(oid) = s.strip_prefix("urn:oid:") {
            if !is_uid(oid) {
                self.problem(path, format!("not an OID: {s}"));
            }
        }
        if s.starts_with("urn:uuid:") && !is_uuid_urn(s) {
            self.problem(path, format!("not a UUID: {s}"));
        }
    }

    fn resource_rules(&mut self, path: &str, rtype: &str, r: &Value) {
        let status_set: &[&str] = match rtype {
            "ServiceRequest" => REQUEST_STATUS,
            "ImagingStudy" => IMAGING_STUDY_STATUS,
            _ => &[],
        };
        if !status_set.is_empty() {
            match r["status"].as_str() {
                Some(s) if status_set.contains(&s) => {}
                Some(s) => self.problem(&format!("{path}.status"), format!("not allowed: {s}")),
                None => self.problem(&format!("{path}.status"), "missing (1..1)"),
            }
        }
        match rtype {
            "ServiceRequest" => {
                for required in ["intent", "subject"] {
                    if r.get(required).is_none() {
                        self.problem(&format!("{path}.{required}"), "missing (1..1)");
                    }
                }
            }
            "ImagingStudy" => {
                if r.get("subject").is_none() {
                    self.problem(&format!("{path}.subject"), "missing (1..1)");
                }
                self.imaging_study_counts(path, r);
            }
            _ => {}
        }
    }

    fn imaging_study_counts(&mut self, path: &str, r: &Value) {
        let series = r["series"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let listed_series = series.len() as u64;
        let mut listed_instances = 0u64;
        for (i, s) in series.iter().enumerate() {
            let n = s["instance"].as_array().map_or(0, Vec::len) as u64;
            listed_instances += n;
            if let Some(claimed) = s["numberOfInstances"].as_u64() {
                if claimed != n {
                    self.problem(
                        &format!("{path}.series[{i}].numberOfInstances"),
                        format!("{claimed} claimed, {n} listed"),
                    );
                }
            }
            if s.get("uid").is_none() {
                self.problem(&format!("{path}.series[{i}].uid"), "missing (1..1)");
            }
            for (j, inst) in s["instance"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .enumerate()
            {
                for required in ["uid", "sopClass"] {
                    if inst.get(required).is_none() {
                        self.problem(
                            &format!("{path}.series[{i}].instance[{j}].{required}"),
                            "missing (1..1)",
                        );
                    }
                }
                if let Some(code) = inst["sopClass"]["code"].as_str() {
                    if !code.starts_with("urn:oid:") {
                        self.problem(
                            &format!("{path}.series[{i}].instance[{j}].sopClass.code"),
                            format!("not urn:oid: {code}"),
                        );
                    }
                }
            }
        }
        for (element, listed) in [
            ("numberOfSeries", listed_series),
            ("numberOfInstances", listed_instances),
        ] {
            if let Some(claimed) = r[element].as_u64() {
                if claimed != listed {
                    self.problem(
                        &format!("{path}.{element}"),
                        format!("{claimed} claimed, {listed} listed"),
                    );
                }
            }
        }
    }
}

const REQUEST_STATUS: &[&str] = &[
    "draft",
    "active",
    "on-hold",
    "revoked",
    "completed",
    "entered-in-error",
    "unknown",
];
const REQUEST_INTENT: &[&str] = &[
    "proposal",
    "plan",
    "directive",
    "order",
    "original-order",
    "reflex-order",
    "filler-order",
    "instance-order",
    "option",
];
const IMAGING_STUDY_STATUS: &[&str] = &[
    "registered",
    "available",
    "cancelled",
    "entered-in-error",
    "unknown",
];

/// FHIR `id`: `[A-Za-z0-9\-\.]{1,64}`.
pub fn is_fhir_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

/// `urn:uuid:` followed by a lower-case `8-4-4-4-12` hex UUID.
pub fn is_uuid_urn(s: &str) -> bool {
    let Some(u) = s.strip_prefix("urn:uuid:") else {
        return false;
    };
    let groups: Vec<&str> = u.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8usize, 4, 4, 4, 12]).all(|(g, n)| {
            g.len() == n
                && g.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
}

/// FHIR `date`: `YYYY`, `YYYY-MM` or `YYYY-MM-DD`, a real calendar date.
pub fn is_fhir_date(s: &str) -> bool {
    date_parts(s).is_some()
}

/// FHIR `dateTime`: a `date`, or `YYYY-MM-DDThh:mm:ss[.fff...](Z|±hh:mm)`.
/// Seconds and an offset are mandatory when a time is present.
pub fn is_fhir_datetime(s: &str) -> bool {
    let Some((date, time)) = s.split_once('T') else {
        return is_fhir_date(s);
    };
    if date_parts(date) != Some(3) {
        return false;
    }
    let (clock, zone) = if let Some(c) = time.strip_suffix('Z') {
        (c, "Z")
    } else {
        match time.rfind(['+', '-']) {
            Some(i) => (&time[..i], &time[i..]),
            None => return false,
        }
    };
    let (hms, frac) = match clock.split_once('.') {
        Some((h, f)) => (h, Some(f)),
        None => (clock, None),
    };
    let fields: Vec<&str> = hms.split(':').collect();
    let [h, m, sec] = fields.as_slice() else {
        return false;
    };
    let two = |f: &str, max: u8| f.len() == 2 && f.parse::<u8>().is_ok_and(|v| v <= max);
    if !(two(h, 23) && two(m, 59) && two(sec, 60)) {
        return false;
    }
    if let Some(f) = frac {
        if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    zone == "Z" || {
        let Some((zh, zm)) = zone[1..].split_once(':') else {
            return false;
        };
        two(zh, 14) && two(zm, 59)
    }
}

/// Number of date parts (1..=3) when `s` is a valid FHIR `date`.
fn date_parts(s: &str) -> Option<usize> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let all_digits = |p: &str, n: usize| p.len() == n && p.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(parts[0], 4) || parts.iter().skip(1).any(|p| !all_digits(p, 2)) {
        return None;
    }
    let year: u16 = parts[0].parse().ok()?;
    if year == 0 {
        return None;
    }
    let month: u8 = match parts.get(1) {
        Some(m) => m.parse().ok().filter(|m| (1..=12).contains(m))?,
        None => return Some(1),
    };
    match parts.get(2) {
        Some(d) => {
            let day: u8 = d.parse().ok()?;
            (day >= 1 && day <= super::datetime::days_in_month(year, month)).then_some(3)
        }
        None => Some(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good() -> Value {
        let p = "urn:uuid:0a1b2c3d-4e5f-8a7b-9c8d-0e1f2a3b4c5d";
        let sr = "urn:uuid:1a1b2c3d-4e5f-8a7b-9c8d-0e1f2a3b4c5d";
        let is = "urn:uuid:2a1b2c3d-4e5f-8a7b-9c8d-0e1f2a3b4c5d";
        json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                { "fullUrl": p, "resource": {
                    "resourceType": "Patient", "id": "patient-1",
                    "identifier": [{ "system": "urn:oid:1.2.3", "value": "X" }],
                    "birthDate": "1970-01-01", "gender": "other"
                }},
                { "fullUrl": sr, "resource": {
                    "resourceType": "ServiceRequest", "id": "servicerequest-1",
                    "status": "active", "intent": "order",
                    "subject": { "reference": p },
                    "identifier": [{ "system": "urn:dicom:uid", "value": "urn:oid:1.2.3.4" }],
                    "authoredOn": "2026-09-05T11:30:00+02:00"
                }},
                { "fullUrl": is, "resource": {
                    "resourceType": "ImagingStudy", "id": "imagingstudy-1",
                    "status": "available",
                    "subject": { "reference": p },
                    "basedOn": [{ "reference": sr }],
                    "started": "2026-09-05",
                    "numberOfSeries": 1, "numberOfInstances": 2,
                    "series": [{
                        "uid": "1.2.3.4.1", "numberOfInstances": 2,
                        "instance": [
                            { "uid": "1.2.3.4.1.1", "sopClass": { "system": "urn:ietf:rfc:3986", "code": "urn:oid:1.2.840.10008.5.1.4.1.1.2" } },
                            { "uid": "1.2.3.4.1.2", "sopClass": { "system": "urn:ietf:rfc:3986", "code": "urn:oid:1.2.840.10008.5.1.4.1.1.2" } }
                        ]
                    }]
                }}
            ]
        })
    }

    fn messages(b: &Value) -> Vec<String> {
        check(b).iter().map(ToString::to_string).collect()
    }

    fn expect_problem(mut b: Value, mutate: impl FnOnce(&mut Value), needle: &str) {
        mutate(&mut b);
        let found = messages(&b);
        assert!(
            found.iter().any(|m| m.contains(needle)),
            "expected a problem containing {needle:?}, got {found:#?}"
        );
    }

    #[test]
    fn the_reference_bundle_passes() {
        assert_eq!(check(&good()), Vec::<Problem>::new());
        assert!(ensure(&good()).is_ok());
    }

    #[test]
    fn bundle_shape() {
        expect_problem(
            good(),
            |b| b["resourceType"] = json!("Patient"),
            "must be `Bundle`",
        );
        expect_problem(
            good(),
            |b| b["type"] = json!("transaction"),
            "must be `collection`",
        );
        expect_problem(good(), |b| b["entry"] = json!([]), "Bundle.entry: empty");
        expect_problem(
            good(),
            |b| {
                b.as_object_mut().unwrap().remove("entry");
            },
            "Bundle.entry: missing",
        );
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["id"] = json!("has space"),
            "not an id",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][0]["resource"]
                    .as_object_mut()
                    .unwrap()
                    .remove("resourceType");
            },
            "resourceType: missing",
        );
    }

    #[test]
    fn full_urls_and_references() {
        expect_problem(
            good(),
            |b| b["entry"][0]["fullUrl"] = json!("Patient/1"),
            "not a urn:uuid",
        );
        expect_problem(
            good(),
            |b| b["entry"][0]["fullUrl"] = json!("urn:uuid:0A1B2C3D-4e5f-8a7b-9c8d-0e1f2a3b4c5d"),
            "not a urn:uuid",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][0].as_object_mut().unwrap().remove("fullUrl");
            },
            "fullUrl: missing",
        );
        expect_problem(
            good(),
            |b| b["entry"][1]["fullUrl"] = b["entry"][0]["fullUrl"].clone(),
            "duplicate fullUrl",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][2]["resource"]["basedOn"][0]["reference"] =
                    json!("urn:uuid:ffffffff-ffff-8fff-8fff-ffffffffffff")
            },
            "unresolved reference",
        );
        expect_problem(
            good(),
            |b| b["entry"][1]["resource"]["subject"]["reference"] = json!("Patient/1"),
            "unresolved reference",
        );
        // An absolute URL is a legal reference even if this mapping never emits one.
        let mut b = good();
        b["entry"][1]["resource"]["subject"]["reference"] = json!("https://fhir.example/Patient/1");
        assert!(messages(&b).iter().all(|m| !m.contains("unresolved")));
    }

    #[test]
    fn empties_nulls_controls_and_padding() {
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["name"] = json!([]),
            "empty array",
        );
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["name"] = json!([{}]),
            "empty object",
        );
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["identifier"][0]["value"] = json!(""),
            "empty string",
        );
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["deceasedBoolean"] = json!(null),
            "null",
        );
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["identifier"][0]["value"] = json!("A\u{1}B"),
            "control character",
        );
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["identifier"][0]["value"] = json!(" X"),
            "whitespace",
        );
        let mut b = good();
        b["entry"][0]["resource"]["text"] =
            json!({ "status": "generated", "div": "<div>a\tb\nc</div>" });
        assert!(
            messages(&b).iter().all(|m| !m.contains("control")),
            "tab and LF are allowed"
        );
    }

    #[test]
    fn uris_oids_and_uuids() {
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["identifier"][0]["system"] = json!("HOSP"),
            "not a URI",
        );
        expect_problem(
            good(),
            |b| b["entry"][1]["resource"]["identifier"][0]["value"] = json!("urn:oid:not.an.oid"),
            "not an OID",
        );
        expect_problem(
            good(),
            |b| b["entry"][1]["resource"]["identifier"][0]["value"] = json!("urn:oid:"),
            "not an OID",
        );
        expect_problem(
            good(),
            |b| b["entry"][1]["resource"]["identifier"][0]["value"] = json!("urn:oid:1.2.3\u{0}"),
            "control character",
        );
        expect_problem(
            good(),
            |b| b["entry"][2]["resource"]["series"][0]["uid"] = json!("garbage"),
            "not a DICOM UID",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][2]["resource"]["series"][0]["instance"][0]["sopClass"]["code"] =
                    json!("1.2.840.10008.5.1.4.1.1.2")
            },
            "not urn:oid",
        );
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["identifier"][0]["value"] = json!("urn:uuid:nope"),
            "not a UUID",
        );
        expect_problem(
            good(),
            |b| b["entry"][2]["resource"]["meta"] = json!({ "profile": ["not a uri|1.0"] }),
            "not a URI",
        );
        let mut b = good();
        b["entry"][2]["resource"]["meta"] =
            json!({ "profile": ["https://example.org/StructureDefinition/x|1.0"] });
        assert!(messages(&b).is_empty(), "canonical|version is a URI");
    }

    #[test]
    fn dates_and_times() {
        for ok in ["1970", "1970-01", "1970-01-01", "2024-02-29"] {
            assert!(is_fhir_date(ok), "{ok}");
            assert!(is_fhir_datetime(ok), "{ok}");
        }
        for bad in [
            "",
            "197",
            "19700",
            "1970-1",
            "1970-01-1",
            "1970-13",
            "1970-00-01",
            "2023-02-29",
            "1970-01-01T",
            "0000-01-01",
            "1970/01/01",
            "1970-01-01-01",
        ] {
            assert!(!is_fhir_date(bad), "{bad}");
        }
        for ok in [
            "1970-01-01T00:00:00Z",
            "1970-01-01T23:59:60+00:00",
            "2026-09-05T11:30:00.5+02:00",
            "2026-09-05T11:30:00.123456-05:30",
            "2026-09-05T11:30:00+14:00",
            "2026-09-05T11:30:00-12:00",
        ] {
            assert!(is_fhir_datetime(ok), "{ok}");
        }
        for bad in [
            "1970-01-01T00:00",
            "1970-01-01T00:00:00",
            "1970-01-01T00:00:00+0100",
            "1970-01-01T00:00:00+01",
            "1970-01-01T24:00:00Z",
            "1970-01-01T00:60:00Z",
            "1970-01-01T00:00:61Z",
            "1970-01-01T00:00:00.Z",
            "1970-01-01T00:00:00.1aZ",
            "1970-01-01T00:00:00+15:00",
            "1970-01-01T00:00:00+01:60",
            "1970-01T00:00:00Z",
            "1970-01-01 00:00:00Z",
            "1970-01-01T0:00:00Z",
            "1970-01-01Tz",
            "T00:00:00Z",
        ] {
            assert!(!is_fhir_datetime(bad), "{bad}");
        }
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["birthDate"] = json!("1970-01-01T00:00:00Z"),
            "not a FHIR date",
        );
        expect_problem(
            good(),
            |b| b["entry"][1]["resource"]["authoredOn"] = json!("2026-09-05T11:30"),
            "not a FHIR dateTime",
        );
        expect_problem(
            good(),
            |b| b["entry"][2]["resource"]["started"] = json!("20260905"),
            "not a FHIR dateTime",
        );
    }

    #[test]
    fn value_sets_and_required_elements() {
        expect_problem(
            good(),
            |b| b["entry"][0]["resource"]["gender"] = json!("M"),
            "AdministrativeGender",
        );
        expect_problem(
            good(),
            |b| b["entry"][1]["resource"]["status"] = json!("cancelled"),
            "status: not allowed",
        );
        expect_problem(
            good(),
            |b| b["entry"][2]["resource"]["status"] = json!("revoked"),
            "status: not allowed",
        );
        expect_problem(
            good(),
            |b| b["entry"][1]["resource"]["intent"] = json!("wish"),
            "RequestIntent",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][1]["resource"]
                    .as_object_mut()
                    .unwrap()
                    .remove("status");
            },
            "status: missing",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][1]["resource"]
                    .as_object_mut()
                    .unwrap()
                    .remove("intent");
            },
            "intent: missing",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][1]["resource"]
                    .as_object_mut()
                    .unwrap()
                    .remove("subject");
            },
            "subject: missing",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][2]["resource"]
                    .as_object_mut()
                    .unwrap()
                    .remove("subject");
            },
            "subject: missing",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][2]["resource"]["series"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("uid");
            },
            "uid: missing",
        );
        expect_problem(
            good(),
            |b| {
                b["entry"][2]["resource"]["series"][0]["instance"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("sopClass");
            },
            "sopClass: missing",
        );
    }

    #[test]
    fn imaging_study_counts_must_agree() {
        expect_problem(
            good(),
            |b| b["entry"][2]["resource"]["numberOfSeries"] = json!(2),
            "numberOfSeries: 2 claimed, 1 listed",
        );
        expect_problem(
            good(),
            |b| b["entry"][2]["resource"]["numberOfInstances"] = json!(3),
            "numberOfInstances: 3 claimed, 2 listed",
        );
        expect_problem(
            good(),
            |b| b["entry"][2]["resource"]["series"][0]["numberOfInstances"] = json!(1),
            "series[0].numberOfInstances: 1 claimed, 2 listed",
        );
        // No series at all: the counts must say so.
        let mut b = good();
        let is = b["entry"][2]["resource"].as_object_mut().unwrap();
        is.remove("series");
        is["numberOfSeries"] = json!(0);
        is["numberOfInstances"] = json!(0);
        assert!(messages(&b).is_empty());
    }

    #[test]
    fn ids_and_uuid_urns() {
        assert!(is_fhir_id("patient-1"));
        assert!(is_fhir_id("A.b-1"));
        assert!(!is_fhir_id(""));
        assert!(!is_fhir_id("a b"));
        assert!(!is_fhir_id("a_b"));
        assert!(!is_fhir_id(&"x".repeat(65)));
        assert!(is_uuid_urn("urn:uuid:0a1b2c3d-4e5f-8a7b-9c8d-0e1f2a3b4c5d"));
        assert!(!is_uuid_urn("0a1b2c3d-4e5f-8a7b-9c8d-0e1f2a3b4c5d"));
        assert!(!is_uuid_urn("urn:uuid:0a1b2c3d4e5f8a7b9c8d0e1f2a3b4c5d"));
        assert!(!is_uuid_urn("urn:uuid:0a1b2c3d-4e5f-8a7b-9c8d-0e1f2a3b4c5"));
        assert!(!is_uuid_urn(
            "urn:uuid:0a1b2c3d-4e5f-8a7b-9c8d-0e1f2a3b4c5g"
        ));
        assert!(!is_uuid_urn("urn:uuid:"));
    }

    #[test]
    fn problems_display_with_their_path() {
        let mut b = good();
        b["entry"][0]["resource"]["gender"] = json!("x");
        let p = &check(&b)[0];
        assert_eq!(p.path, "Bundle.entry[0].resource<Patient>.gender");
        assert_eq!(
            p.to_string(),
            "Bundle.entry[0].resource<Patient>.gender: not an AdministrativeGender: x"
        );
        assert_eq!(ensure(&b), Err(vec![p.clone()]));
    }
}
