//! Identifier systems and constructors. Every string here is fixed by FHIR
//! R4 or by the HL7 "Version 2 to FHIR" implementation guide; nothing is
//! invented. Where a hospital's own namespace would be needed (accession,
//! MRN, order numbers) no system is emitted unless the message carries one.

use serde_json::{json, Value};

/// FHIR-defined system for DICOM UIDs (R4 `identifier-registry`), values
/// written as `urn:oid:<uid>`.
pub const DICOM_UID_SYSTEM: &str = "urn:dicom:uid";

/// HL7 v2 table 0203, identifier types: `MR`, `ACSN`, `PLAC`, `FILL`.
pub const V2_0203: &str = "http://terminology.hl7.org/CodeSystem/v2-0203";

/// DICOM controlled terminology, used for modality codes (`ImagingStudy.modality`).
pub const DCM: &str = "http://dicom.nema.org/resources/ontology/DCM";

/// System for `ImagingStudy.series.instance.sopClass`, per the R4
/// `ImagingStudy` definition (a `Coding` with `urn:oid:` codes).
pub const RFC3986: &str = "urn:ietf:rfc:3986";

/// MII Modul Bildgebung, ImagingStudy profile (canonical URL).
pub const MII_IMAGING_STUDY_PROFILE: &str = "https://www.medizininformatik-initiative.de/fhir/ext/modul-bildgebung/StructureDefinition/mii-pr-bildgebung-bildgebungsstudie";

/// The profile version that was read when writing this mapping.
pub const MII_IMAGING_STUDY_VERSION: &str = "2025.0.0-ballot";

pub const PATIENT_ID: &str = "patient-1";
pub const SERVICE_REQUEST_ID: &str = "servicerequest-1";
pub const IMAGING_STUDY_ID: &str = "imagingstudy-1";

/// The `fullUrl` of each entry. FHIR requires a `fullUrl` on every entry of
/// a bundle that is not a transaction or batch (the validator enforces it),
/// and references between entries must then be resolvable against those
/// URLs. `urn:uuid:` is the form for resources that live on no server.
///
/// The UUIDs are RFC 9562 version 8 (custom format) values derived from a
/// hash of the study identity and the resource type, so the same study
/// yields the same bundle every time and no random source is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refs {
    pub patient: String,
    pub service_request: String,
    pub imaging_study: String,
}

impl Refs {
    /// `seed` identifies the study: the Study Instance UID when known.
    pub fn for_study(seed: &str) -> Refs {
        Refs {
            patient: uuid_urn(&format!("dicomscope|Patient|{seed}")),
            service_request: uuid_urn(&format!("dicomscope|ServiceRequest|{seed}")),
            imaging_study: uuid_urn(&format!("dicomscope|ImagingStudy|{seed}")),
        }
    }
}

/// A `Reference` to an entry by its `fullUrl`.
pub fn reference(full_url: &str) -> Value {
    json!({ "reference": full_url })
}

/// `urn:uuid:` form of a version-8 UUID derived from `name`.
pub fn uuid_urn(name: &str) -> String {
    format!("urn:uuid:{}", uuid_v8(name))
}

/// RFC 9562 UUIDv8: 122 bits of application-defined content, here two
/// FNV-1a 64-bit hashes of the name with different offsets, with the
/// version nibble set to 8 and the variant bits to `10`.
fn uuid_v8(name: &str) -> String {
    let hi = fnv1a(name.as_bytes(), 0xcbf2_9ce4_8422_2325);
    let lo = fnv1a(name.as_bytes(), 0x84222325_cbf29ce4 ^ 0x5bd1_e995_9a2b_f347);
    let hi = (hi & 0xffff_ffff_ffff_0fff) | 0x0000_0000_0000_8000;
    let lo = (lo & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        hi >> 32,
        (hi >> 16) & 0xffff,
        hi & 0xffff,
        lo >> 48,
        lo & 0xffff_ffff_ffff
    )
}

fn fnv1a(bytes: &[u8], offset: u64) -> u64 {
    let mut h = offset;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `urn:oid:` form of a DICOM UID, trailing padding removed.
pub fn oid_urn(uid: &str) -> String {
    format!("urn:oid:{}", uid.trim_end_matches('\0').trim())
}

/// A DICOM UID as a FHIR identifier.
pub fn uid_identifier(uid: &str) -> Value {
    json!({ "system": DICOM_UID_SYSTEM, "value": oid_urn(uid) })
}

/// What PID-3.4 (assigning authority, HD) gives us: a `system` only when
/// it is a URI, else an `assigner.display`. Never a fabricated URI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Authority {
    pub system: Option<String>,
    pub assigner: Option<String>,
}

impl Authority {
    /// From an HD value (`namespace^universal id^universal id type`); the
    /// universal ID is preferred when it is a URI, then the namespace.
    pub fn from_hd(hd: Option<&str>) -> Authority {
        let Some(hd) = hd.map(str::trim).filter(|s| !s.is_empty()) else {
            return Authority::default();
        };
        let mut parts = hd.split('^');
        let namespace = parts.next().unwrap_or("").trim();
        let universal = parts.next().unwrap_or("").trim();
        let universal_type = parts.next().unwrap_or("").trim();
        if is_uri(universal) {
            return Authority {
                system: Some(universal.to_string()),
                assigner: None,
            };
        }
        if universal_type.eq_ignore_ascii_case("ISO") && !universal.is_empty() && is_oid(universal)
        {
            return Authority {
                system: Some(format!("urn:oid:{universal}")),
                assigner: (!namespace.is_empty()).then(|| namespace.to_string()),
            };
        }
        if is_uri(namespace) {
            return Authority {
                system: Some(namespace.to_string()),
                assigner: None,
            };
        }
        Authority {
            system: None,
            assigner: (!namespace.is_empty()).then(|| namespace.to_string()),
        }
    }
}

/// An identifier with a v2-0203 type code, and a system or assigner only
/// when the source supplies one.
pub fn typed_identifier(code: &str, display: &str, value: &str, authority: &Authority) -> Value {
    let mut id = json!({
        "type": { "coding": [{ "system": V2_0203, "code": code, "display": display }] },
        "value": value,
    });
    if let Some(system) = &authority.system {
        id["system"] = json!(system);
    }
    if let Some(assigner) = &authority.assigner {
        id["assigner"] = json!({ "display": assigner });
    }
    id
}

/// An identifier with neither type nor system: carried, not standardised.
pub fn plain_identifier(value: &str) -> Value {
    json!({ "value": value })
}

/// A conservative URI check: a scheme followed by `:` and no whitespace.
/// `urn:oid:1.2`, `http://x`, `urn:uuid:…` pass; `HOSP` and `RIS` do not.
pub fn is_uri(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    let scheme_ok = scheme
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic())
        .unwrap_or(false)
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.');
    scheme_ok && !rest.is_empty() && !s.chars().any(char::is_whitespace)
}

fn is_oid(s: &str) -> bool {
    !s.is_empty()
        && s.split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && s.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_check() {
        assert!(is_uri("urn:oid:1.2.3"));
        assert!(is_uri("http://hospital.example/mrn"));
        assert!(!is_uri("HOSP"));
        assert!(!is_uri("RIS SYSTEM"));
        assert!(!is_uri(":x"));
        assert!(!is_uri("a b:c"));
    }

    #[test]
    fn authority_never_fabricates_a_system() {
        assert_eq!(Authority::from_hd(None), Authority::default());
        assert_eq!(Authority::from_hd(Some("")), Authority::default());
        let plain = Authority::from_hd(Some("HOSP"));
        assert_eq!(plain.system, None);
        assert_eq!(plain.assigner.as_deref(), Some("HOSP"));
        let uri = Authority::from_hd(Some("HOSP^http://hospital.example/mrn^URI"));
        assert_eq!(uri.system.as_deref(), Some("http://hospital.example/mrn"));
        assert_eq!(uri.assigner, None);
        let iso = Authority::from_hd(Some("HOSP^1.2.276.0.76.3.1.1^ISO"));
        assert_eq!(iso.system.as_deref(), Some("urn:oid:1.2.276.0.76.3.1.1"));
        assert_eq!(iso.assigner.as_deref(), Some("HOSP"));
        let bad_iso = Authority::from_hd(Some("HOSP^notanoid^ISO"));
        assert_eq!(bad_iso.system, None);
        assert_eq!(bad_iso.assigner.as_deref(), Some("HOSP"));
    }

    #[test]
    fn uuids_are_stable_well_formed_and_distinct() {
        let r = Refs::for_study("1.2.3");
        assert_eq!(r, Refs::for_study("1.2.3"), "deterministic");
        assert_ne!(r.patient, r.service_request);
        assert_ne!(r.patient, Refs::for_study("1.2.4").patient);
        let u = r.patient.strip_prefix("urn:uuid:").unwrap();
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('8'), "version 8: {u}");
        assert!(
            matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b')),
            "variant 10: {u}"
        );
        assert!(u.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert_eq!(reference(&r.patient)["reference"], r.patient);
    }

    #[test]
    fn identifiers() {
        assert_eq!(uid_identifier("1.2.3\0")["value"], "urn:oid:1.2.3");
        let id = typed_identifier(
            "MR",
            "Medical record number",
            "4MR1",
            &Authority::from_hd(Some("HOSP")),
        );
        assert_eq!(id["type"]["coding"][0]["code"], "MR");
        assert_eq!(id["value"], "4MR1");
        assert!(id.get("system").is_none(), "no fabricated system");
        assert_eq!(id["assigner"]["display"], "HOSP");
        assert_eq!(plain_identifier("RP-1"), json!({ "value": "RP-1" }));
    }
}
