//! `Patient` from the DICOM patient module first, PID second, element by
//! element. The imaged patient is the subject of the study, so DICOM wins
//! where both say something; where they disagree, the link panel already
//! shows the mismatch and this resource does not try to.
//!
//! Sources: HL7 "Version 2 to FHIR" IG, segment map PID → Patient
//! (PID-3 → identifier, PID-5 → name, PID-7 → birthDate, PID-8 → gender);
//! DICOM PS3.3 C.7.1.1 Patient module.

use super::datetime::{dicom_date, hl7_date};
use super::ids::{fhir_string, typed_identifier, Authority, PATIENT_ID};
use crate::dicom::Study;
use hl7kit::Message;
use serde_json::{json, Value};

pub fn patient(study: &Study, msg: Option<&Message>) -> Value {
    let mut p = json!({ "resourceType": "Patient", "id": PATIENT_ID });

    // identifier[MR]: (0010,0020), else PID-3.1; assigning authority only
    // from PID-3.4 (DICOM has Issuer of Patient ID, (0010,0021), which is a
    // bare name, so it becomes an assigner display too).
    let id_value = study
        .patient_id
        .as_deref()
        .and_then(fhir_string)
        .or_else(|| {
            msg.and_then(|m| m.get_decoded("PID-3.1"))
                .and_then(|v| fhir_string(&v))
        });
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

/// A FHIR `HumanName` from a caret-separated name, or `None` when there is
/// neither a family nor a given name (a prefix or suffix alone is not a
/// name). Components go through [`fhir_string`]; a sixth and later
/// component (HL7 XPN degree, name type code, and so on) is ignored.
pub fn parse_name(text: &str, order: NameOrder) -> Option<Value> {
    let first_group = text.split('=').next().unwrap_or("");
    let parts: Vec<Option<String>> = first_group.split('^').map(fhir_string).collect();
    let get = |i: usize| parts.get(i).cloned().flatten();
    let family = get(0);
    let mut given: Vec<String> = Vec::new();
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
    if let Some(f) = &family {
        name["family"] = json!(f);
    }
    if !given.is_empty() {
        name["given"] = json!(given);
    }
    if let Some(p) = &prefix {
        name["prefix"] = json!([p]);
    }
    if let Some(s) = &suffix {
        name["suffix"] = json!([s]);
    }
    let text: Vec<&str> = prefix
        .as_deref()
        .into_iter()
        .chain(given.iter().map(String::as_str))
        .chain(family.as_deref())
        .chain(suffix.as_deref())
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
        assert_eq!(gender(" M "), Some("male"), "DICOM padding");
        assert_eq!(gender("U"), None);
        assert_eq!(gender("A"), None, "HL7 ambiguous");
        assert_eq!(gender("N"), None, "HL7 not applicable");
        assert_eq!(gender("male"), None, "codes, not words");
        assert_eq!(gender("MF"), None);
        assert_eq!(gender(""), None);
        assert_eq!(gender("\"\""), None);
    }

    #[test]
    fn mononyms_and_partial_names() {
        let n = parse_name("Cher", NameOrder::Dicom).unwrap();
        assert_eq!(n["family"], "Cher");
        assert!(n.get("given").is_none());
        assert_eq!(n["text"], "Cher");

        let n = parse_name("^Madonna", NameOrder::Hl7).unwrap();
        assert!(n.get("family").is_none());
        assert_eq!(n["given"], json!(["Madonna"]));
        assert_eq!(n["text"], "Madonna");

        let n = parse_name("^^Middle", NameOrder::Dicom).unwrap();
        assert_eq!(
            n["given"],
            json!(["Middle"]),
            "a middle name alone is still a given name"
        );

        assert!(
            parse_name("^^^Dr^PhD", NameOrder::Dicom).is_none(),
            "titles alone are not a name"
        );
        assert!(parse_name("   ", NameOrder::Dicom).is_none());
        assert!(parse_name(" ^ ^ ", NameOrder::Hl7).is_none());
        assert!(
            parse_name("=山田^太郎", NameOrder::Dicom).is_none(),
            "only an ideographic group"
        );
    }

    #[test]
    fn prefix_and_suffix_swap_between_standards() {
        let d = parse_name("Doe^John^^Dr^Jr", NameOrder::Dicom).unwrap();
        assert_eq!(d["prefix"], json!(["Dr"]));
        assert_eq!(d["suffix"], json!(["Jr"]));
        assert_eq!(d["text"], "Dr John Doe Jr");
        let h = parse_name("Doe^John^^Jr^Dr", NameOrder::Hl7).unwrap();
        assert_eq!(h["prefix"], json!(["Dr"]));
        assert_eq!(h["suffix"], json!(["Jr"]));
        assert_eq!(h["text"], "Dr John Doe Jr");
        // Read with the wrong order the titles swap; the test pins that
        // the order flag is what decides, not the content.
        let wrong = parse_name("Doe^John^^Dr^Jr", NameOrder::Hl7).unwrap();
        assert_eq!(wrong["prefix"], json!(["Jr"]));
    }

    #[test]
    fn extra_components_padding_and_controls() {
        let n = parse_name("Doe^John^Q^Jr^Dr^MD^L", NameOrder::Hl7).unwrap();
        assert_eq!(
            n["suffix"],
            json!(["Jr"]),
            "XPN.6 degree and XPN.7 type are ignored"
        );
        let n = parse_name(" Doe ^ John \0", NameOrder::Dicom).unwrap();
        assert_eq!(n["family"], "Doe");
        assert_eq!(n["given"], json!(["John"]));
        assert_eq!(n["text"], "John Doe");
        let n = parse_name("Do\u{1}e^Jo\u{7f}hn", NameOrder::Dicom).unwrap();
        assert_eq!(n["family"], "Doe");
        assert_eq!(n["given"], json!(["John"]));
        let n = parse_name("Müller^Jürgen^^^Prof. Dr. med.", NameOrder::Dicom).unwrap();
        assert_eq!(n["suffix"], json!(["Prof. Dr. med."]));
        assert_eq!(n["text"], "Jürgen Müller Prof. Dr. med.");
    }

    #[test]
    fn ideographic_and_phonetic_groups_never_leak() {
        let n = parse_name("Yamada^Tarou=山田^太郎=やまだ^たろう", NameOrder::Dicom).unwrap();
        let s = n.to_string();
        assert!(!s.contains("山田") && !s.contains("やまだ"), "{s}");
        let n = parse_name("Wang^Xiaodong=王^小東", NameOrder::Dicom).unwrap();
        assert_eq!(n["text"], "Xiaodong Wang");
        let n = parse_name("Yamada^Tarou==", NameOrder::Dicom).unwrap();
        assert_eq!(n["family"], "Yamada");
    }

    fn msg(pid: &str) -> Message {
        Message::parse(format!(
            "MSH|^~\\&|RIS|HOSP|||20260905||ORM^O01|1|P|2.5.1\r{pid}\r"
        ))
        .unwrap()
    }

    #[test]
    fn dicom_wins_element_by_element_and_pid_fills_gaps() {
        let study = Study {
            patient_id: Some("D1".into()),
            patient_name: Some("Dicom^Name".into()),
            patient_birth_date: Some("19800102".into()),
            patient_sex: Some("F".into()),
            ..Study::default()
        };
        let m = msg("PID|1||H1^^^HOSP^MR||Hl7^Name||19900304|M");
        let p = patient(&study, Some(&m));
        assert_eq!(p["identifier"][0]["value"], "D1");
        assert_eq!(
            p["identifier"][0]["assigner"]["display"], "HOSP",
            "authority still from PID-3.4"
        );
        assert_eq!(p["name"][0]["family"], "Dicom");
        assert_eq!(p["birthDate"], "1980-01-02");
        assert_eq!(p["gender"], "female");

        let empty = Study::default();
        let p = patient(&empty, Some(&m));
        assert_eq!(p["identifier"][0]["value"], "H1");
        assert_eq!(p["name"][0]["family"], "Hl7");
        assert_eq!(p["birthDate"], "1990-03-04");
        assert_eq!(p["gender"], "male");

        // Partial DICOM: each element falls back on its own.
        let partial = Study {
            patient_sex: Some("O".into()),
            patient_birth_date: Some("19800230".into()),
            ..Study::default()
        };
        let p = patient(&partial, Some(&m));
        assert_eq!(p["gender"], "other");
        assert_eq!(
            p["birthDate"], "1990-03-04",
            "invalid DICOM DA falls through to PID-7"
        );
        assert_eq!(p["identifier"][0]["value"], "H1");
    }

    #[test]
    fn no_message_and_no_dicom_gives_a_bare_patient() {
        let p = patient(&Study::default(), None);
        assert_eq!(p, json!({ "resourceType": "Patient", "id": PATIENT_ID }));
        let p = patient(&Study::default(), Some(&msg("PID|1")));
        assert_eq!(p, json!({ "resourceType": "Patient", "id": PATIENT_ID }));
    }

    #[test]
    fn gender_priority_and_unknowns() {
        let m = msg("PID|1||H1||||19900304|M");
        let dicom_u = Study {
            patient_sex: Some("U".into()),
            ..Study::default()
        };
        assert!(
            patient(&dicom_u, Some(&m)).get("gender").is_none(),
            "DICOM `U` is a statement; PID-8 does not override it"
        );
        let dicom_none = Study::default();
        assert_eq!(patient(&dicom_none, Some(&m))["gender"], "male");
        let m = msg("PID|1||H1||||19900304|U");
        assert!(patient(&dicom_none, Some(&m)).get("gender").is_none());
        let m = msg("PID|1||H1||||19900304|\"\"");
        assert!(patient(&dicom_none, Some(&m)).get("gender").is_none());
    }

    #[test]
    fn pid3_assigning_authority_shapes() {
        let s = Study::default();
        let p = patient(&s, Some(&msg("PID|1||H1^^^HOSP^MR")));
        assert!(p["identifier"][0].get("system").is_none());
        assert_eq!(p["identifier"][0]["assigner"]["display"], "HOSP");

        let p = patient(&s, Some(&msg("PID|1||H1^^^HOSP&1.2.276.0.76.3.1.1&ISO^MR")));
        assert_eq!(p["identifier"][0]["system"], "urn:oid:1.2.276.0.76.3.1.1");
        assert_eq!(p["identifier"][0]["assigner"]["display"], "HOSP");

        let p = patient(
            &s,
            Some(&msg("PID|1||H1^^^&http://hospital.example/mrn&URI^MR")),
        );
        assert_eq!(p["identifier"][0]["system"], "http://hospital.example/mrn");
        assert!(p["identifier"][0].get("assigner").is_none());

        let p = patient(
            &s,
            Some(&msg(
                "PID|1||H1^^^&urn:uuid:6ba7b810-9dad-11d1-80b4-00c04fd430c8&UUID^MR",
            )),
        );
        assert_eq!(
            p["identifier"][0]["system"],
            "urn:uuid:6ba7b810-9dad-11d1-80b4-00c04fd430c8"
        );

        let p = patient(&s, Some(&msg("PID|1||H1^^^HOSP&garbage&ISO^MR")));
        assert!(
            p["identifier"][0].get("system").is_none(),
            "a bad OID is not a system"
        );
        assert_eq!(p["identifier"][0]["assigner"]["display"], "HOSP");

        let p = patient(&s, Some(&msg("PID|1||H1")));
        assert!(p["identifier"][0].get("system").is_none());
        assert!(p["identifier"][0].get("assigner").is_none());
        assert_eq!(p["identifier"][0]["type"]["coding"][0]["code"], "MR");
    }

    #[test]
    fn pid_values_are_decoded_and_cleaned() {
        let s = Study::default();
        let p = patient(
            &s,
            Some(&msg("PID|1||  \\F\\123 ^^^HOSP^MR||O\\E\\Brien^Se\\T\\an")),
        );
        assert_eq!(
            p["identifier"][0]["value"], "|123",
            "escape decoded, padding gone"
        );
        assert_eq!(p["name"][0]["family"], "O\\Brien");
        assert_eq!(p["name"][0]["given"], json!(["Se&an"]));
        let p = patient(&s, Some(&msg("PID|1||   ^^^HOSP^MR")));
        assert!(
            p.get("identifier").is_none(),
            "a blank PID-3.1 is no identifier"
        );
        let p = patient(&s, Some(&msg("PID|1||\"\"")));
        assert!(p.get("identifier").is_none(), "HL7 null is no identifier");
        let p = patient(&s, Some(&msg("PID|1||H1||^^^^")));
        assert!(p.get("name").is_none());
        let p = patient(&s, Some(&msg("PID|1||H1||Doe^Jane~Smith^Jane")));
        assert_eq!(
            p["name"].as_array().unwrap().len(),
            1,
            "only the first repetition"
        );
        assert_eq!(p["name"][0]["family"], "Doe");
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
