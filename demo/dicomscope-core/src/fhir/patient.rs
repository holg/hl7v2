//! `Patient` from the DICOM patient module first, PID second, element by
//! element. The imaged patient is the subject of the study, so DICOM wins
//! where both say something; where they disagree, the link panel already
//! shows the mismatch and this resource does not try to.
//!
//! Sources: HL7 "Version 2 to FHIR" IG, segment map PID → Patient
//! (PID-3 → identifier, PID-5 → name, PID-7 → birthDate, PID-8 → gender);
//! DICOM PS3.3 C.7.1.1 Patient module.

use super::datetime::{dicom_date, hl7_date};
use super::ids::{typed_identifier, Authority, PATIENT_ID};
use crate::dicom::Study;
use hl7kit::Message;
use serde_json::{json, Value};

pub fn patient(study: &Study, msg: Option<&Message>) -> Value {
    let mut p = json!({ "resourceType": "Patient", "id": PATIENT_ID });

    // identifier[MR]: (0010,0020), else PID-3.1; assigning authority only
    // from PID-3.4 (DICOM has Issuer of Patient ID, (0010,0021), which is a
    // bare name, so it becomes an assigner display too).
    let hl7_pid = msg.and_then(|m| m.get_decoded("PID-3.1").map(|v| v.trim().to_string()));
    let id_value = study
        .patient_id
        .clone()
        .or(hl7_pid)
        .filter(|s| !s.is_empty());
    if let Some(value) = id_value {
        let authority = Authority::from_hd(msg.and_then(|m| m.get("PID-3.4")));
        p["identifier"] = json!([typed_identifier(
            "MR",
            "Medical record number",
            &value,
            &authority
        )]);
    }

    // name: (0010,0010) PN `Family^Given^Middle^Prefix^Suffix`, first `=`
    // group only (alphabetic; ideographic and phonetic groups are dropped);
    // else PID-5 XPN `Family^Given^Middle^Suffix^Prefix`. Note the swapped
    // prefix and suffix positions between the two standards.
    let name = match &study.patient_name {
        Some(pn) => parse_name(pn, NameOrder::Dicom),
        None => msg
            .and_then(|m| m.get_decoded("PID-5").map(|v| v.into_owned()))
            .and_then(|xpn| parse_name(&xpn, NameOrder::Hl7)),
    };
    if let Some(name) = name {
        p["name"] = json!([name]);
    }

    // birthDate: (0010,0030) DA, else the date part of PID-7.
    let birth = study
        .patient_birth_date
        .as_deref()
        .and_then(dicom_date)
        .or_else(|| msg.and_then(|m| m.get("PID-7")).and_then(hl7_date));
    if let Some(b) = birth {
        p["birthDate"] = json!(b);
    }

    // gender: (0010,0040) else PID-8; M/F/O map, U and absent are omitted
    // (absent is absent; `unknown` would claim knowledge of ignorance).
    let sex = study
        .patient_sex
        .clone()
        .or_else(|| msg.and_then(|m| m.get("PID-8").map(|s| s.to_string())));
    if let Some(g) = sex.as_deref().and_then(gender) {
        p["gender"] = json!(g);
    }
    p
}

pub fn gender(code: &str) -> Option<&'static str> {
    match code.trim().to_ascii_uppercase().as_str() {
        "M" => Some("male"),
        "F" => Some("female"),
        "O" => Some("other"),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub enum NameOrder {
    /// DICOM PN: 4 = prefix, 5 = suffix.
    Dicom,
    /// HL7 XPN: 4 = suffix, 5 = prefix.
    Hl7,
}

/// A FHIR `HumanName` from a caret-separated name, or `None` when empty.
pub fn parse_name(text: &str, order: NameOrder) -> Option<Value> {
    let first_group = text.split('=').next().unwrap_or("");
    let parts: Vec<&str> = first_group.split('^').map(str::trim).collect();
    let get = |i: usize| parts.get(i).copied().filter(|s| !s.is_empty());
    let family = get(0);
    let mut given: Vec<&str> = Vec::new();
    given.extend(get(1));
    given.extend(get(2));
    let (prefix, suffix) = match order {
        NameOrder::Dicom => (get(3), get(4)),
        NameOrder::Hl7 => (get(4), get(3)),
    };
    if family.is_none() && given.is_empty() {
        return None;
    }
    let mut name = json!({});
    if let Some(f) = family {
        name["family"] = json!(f);
    }
    if !given.is_empty() {
        name["given"] = json!(given);
    }
    if let Some(p) = prefix {
        name["prefix"] = json!([p]);
    }
    if let Some(s) = suffix {
        name["suffix"] = json!([s]);
    }
    let text: Vec<&str> = prefix
        .into_iter()
        .chain(given.iter().copied())
        .chain(family)
        .chain(suffix)
        .collect();
    name["text"] = json!(text.join(" "));
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genders() {
        assert_eq!(gender("M"), Some("male"));
        assert_eq!(gender("f"), Some("female"));
        assert_eq!(gender("O"), Some("other"));
        assert_eq!(gender("U"), None);
        assert_eq!(gender(""), None);
    }

    #[test]
    fn names() {
        let n = parse_name("Doe^Jane^Marie^Dr^PhD", NameOrder::Dicom).unwrap();
        assert_eq!(n["family"], "Doe");
        assert_eq!(n["given"], json!(["Jane", "Marie"]));
        assert_eq!(n["prefix"], json!(["Dr"]));
        assert_eq!(n["suffix"], json!(["PhD"]));
        assert_eq!(n["text"], "Dr Jane Marie Doe PhD");
        let h = parse_name("Doe^Jane^^Jr^Dr", NameOrder::Hl7).unwrap();
        assert_eq!(h["prefix"], json!(["Dr"]));
        assert_eq!(h["suffix"], json!(["Jr"]));
        // Ideographic and phonetic groups are dropped.
        let i = parse_name("Yamada^Tarou=山田^太郎=やまだ^たろう", NameOrder::Dicom).unwrap();
        assert_eq!(i["family"], "Yamada");
        assert_eq!(i["given"], json!(["Tarou"]));
        assert!(parse_name("^^^", NameOrder::Dicom).is_none());
        assert!(parse_name("", NameOrder::Hl7).is_none());
        assert_eq!(
            parse_name("Anonymized^^", NameOrder::Dicom).unwrap()["text"],
            "Anonymized"
        );
    }
}
