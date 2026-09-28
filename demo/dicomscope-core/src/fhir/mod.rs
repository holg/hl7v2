//! FHIR R4 output: one `Bundle` of type `collection` holding `Patient`,
//! `ServiceRequest` and `ImagingStudy` for the loaded study and order.
//!
//! Built as `serde_json::Value`, no typed FHIR crate, no terminology
//! lookup: everything is generated locally from the two inputs. Free of
//! Leptos and web-sys, so it is tested on the host. [`validate::check`]
//! is the in-process structural check every emitted bundle is held to.

pub mod datetime;
pub mod ids;
pub mod imaging_study;
pub mod patient;
pub mod service_request;
pub mod validate;

use crate::dicom::{Series, Study};
use crate::link::{LinkPath, Linkage};
use hl7kit::order::Order;
use hl7kit::Message;
use serde_json::{json, Value};

/// What the mapping reads.
pub struct FhirInput<'a> {
    pub study: &'a Study,
    pub series: &'a [Series],
    pub message: &'a Message,
    pub order: &'a Order,
    pub linkage: &'a Linkage,
}

/// The three resources, in bundle order, with their entry identities.
pub struct Resources {
    pub refs: ids::Refs,
    pub patient: Value,
    pub service_request: Value,
    pub imaging_study: Value,
}

pub fn resources(input: &FhirInput<'_>) -> Resources {
    // Entry identities are derived from the study, so the same study gives
    // the same URNs on every run and across the panel and the CLI.
    let seed = input
        .study
        .study_uid
        .as_deref()
        .or(input.order.study_uid.as_deref())
        .or(input.study.accession_number.as_deref())
        .or(input.study.patient_id.as_deref())
        .unwrap_or("unidentified");
    let refs = ids::Refs::for_study(seed);
    Resources {
        patient: patient::patient(input.study, Some(input.message)),
        service_request: service_request::service_request(input.message, input.order, &refs),
        imaging_study: imaging_study::imaging_study(
            input.study,
            input.series,
            input.linkage.path != LinkPath::None,
            &refs,
        ),
        refs,
    }
}

/// A `collection` bundle. Every entry carries a `fullUrl` (`urn:uuid:`),
/// which FHIR requires for any bundle that is not a transaction or batch;
/// references between the entries use the same URNs. The HL7 validator
/// rejects a collection without `fullUrl`, so this is not optional.
pub fn bundle(input: &FhirInput<'_>) -> Value {
    let r = resources(input);
    json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            { "fullUrl": r.refs.patient, "resource": r.patient },
            { "fullUrl": r.refs.service_request, "resource": r.service_request },
            { "fullUrl": r.refs.imaging_study, "resource": r.imaging_study },
        ],
    })
}

/// Where each linkage row lands in the bundle, for the link panel.
pub fn fhir_element(field: hl7kit::order::OrderField) -> &'static str {
    use hl7kit::order::OrderField;
    match field {
        OrderField::StudyUid => {
            "ImagingStudy.identifier[urn:dicom:uid], ServiceRequest.identifier[urn:dicom:uid]"
        }
        OrderField::Accession => "ImagingStudy.identifier[ACSN], ServiceRequest.identifier[ACSN]",
        OrderField::PatientId => "Patient.identifier[MR]",
        OrderField::ProcedureId => "ServiceRequest.identifier (untyped: carried, not standardised)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dicom::series::Slice;
    use crate::link;

    const UID: &str = "1.3.6.1.4.1.5962.1.2.4.20040826185059.5457";

    fn sample(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../samples/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn study() -> Study {
        Study {
            patient_id: Some("4MR1".into()),
            accession_number: Some("ACC-2026-0001".into()),
            study_uid: Some(UID.into()),
            requested_procedure_id: Some("RP-2026-0001".into()),
            transfer_syntax: "1.2.840.10008.1.2.1".into(),
            modality: Some("MR".into()),
            rows: 64,
            cols: 64,
            patient_name: Some("CompressedSamples^MR1".into()),
            patient_birth_date: None,
            patient_sex: Some("O".into()),
            study_date: Some("20040826".into()),
            study_time: Some("185059".into()),
            timezone_offset: None,
            study_description: Some("Head".into()),
        }
    }

    fn slice(file: usize, frame: u32, n: i32, uid: &str) -> Slice {
        Slice {
            file,
            frame,
            instance_number: Some(n),
            position: Some(n as f64),
            sop_instance_uid: uid.into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.1.4".into(),
        }
    }

    fn series() -> Vec<Series> {
        vec![
            Series {
                uid: "1.2.3.1".into(),
                number: Some(1),
                description: "Localizer".into(),
                modality: "MR".into(),
                rows: 64,
                cols: 64,
                slices: vec![slice(0, 0, 1, "1.2.3.1.1")],
            },
            Series {
                uid: "1.2.3.2".into(),
                number: Some(2),
                description: "Axial".into(),
                modality: "MR".into(),
                rows: 64,
                cols: 64,
                // Two instances, the second multi-frame with two frames.
                slices: vec![
                    slice(1, 0, 1, "1.2.3.2.1"),
                    slice(2, 0, 2, "1.2.3.2.2"),
                    slice(2, 1, 2, "1.2.3.2.2"),
                ],
            },
        ]
    }

    fn build(msg_text: &str, study: &Study) -> (Value, Linkage) {
        let msg = Message::parse(msg_text).unwrap();
        let order = Order::extract(&msg);
        let linkage = link::resolve(study, &order);
        let series = series();
        let input = FhirInput {
            study,
            series: &series,
            message: &msg,
            order: &order,
            linkage: &linkage,
        };
        (bundle(&input), linkage)
    }

    fn entry<'a>(b: &'a Value, resource_type: &str) -> &'a Value {
        b["entry"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| &e["resource"])
            .find(|r| r["resourceType"] == resource_type)
            .unwrap()
    }

    #[test]
    fn orm_bundle_links_and_populates_core_elements() {
        let (b, linkage) = build(&sample("order.hl7"), &study());
        assert_eq!(linkage.path, LinkPath::StudyUid);
        assert_eq!(b["resourceType"], "Bundle");
        assert_eq!(b["type"], "collection");
        assert_eq!(b["entry"].as_array().unwrap().len(), 3);

        let p = entry(&b, "Patient");
        assert_eq!(p["id"], "patient-1");
        assert_eq!(p["identifier"][0]["type"]["coding"][0]["code"], "MR");
        assert_eq!(p["identifier"][0]["value"], "4MR1");
        assert_eq!(
            p["identifier"][0]["assigner"]["display"], "HOSP",
            "from PID-3.4"
        );
        assert!(p["identifier"][0].get("system").is_none());
        assert_eq!(p["name"][0]["family"], "CompressedSamples");
        assert_eq!(p["name"][0]["given"], json!(["MR1"]));
        assert_eq!(p["gender"], "other");
        // DICOM had no birth date; PID-7 supplies it.
        assert_eq!(p["birthDate"], "1970-01-01");

        // Every entry has a urn:uuid fullUrl and references use those URNs.
        let full_urls: Vec<String> = b["entry"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["fullUrl"].as_str().unwrap().to_string())
            .collect();
        assert!(full_urls.iter().all(|u| u.starts_with("urn:uuid:")));
        assert_eq!(full_urls.len(), 3);
        let sr = entry(&b, "ServiceRequest");
        assert_eq!(sr["subject"]["reference"], full_urls[0]);
        let ids = sr["identifier"].as_array().unwrap();
        assert!(ids
            .iter()
            .any(|i| i["system"] == "urn:dicom:uid" && i["value"] == format!("urn:oid:{UID}")));
        assert!(ids
            .iter()
            .any(|i| i["type"]["coding"][0]["code"] == "ACSN" && i["value"] == "ACC-2026-0001"));
        assert!(ids
            .iter()
            .any(|i| i["type"]["coding"][0]["code"] == "PLAC" && i["value"] == "ORD-2026-0001"));
        assert!(ids
            .iter()
            .any(|i| i["type"]["coding"][0]["code"] == "FILL" && i["value"] == "FIL-2026-0001"));
        assert!(ids
            .iter()
            .any(|i| i.get("type").is_none() && i["value"] == "RP-2026-0001"));
        assert!(ids.iter().all(|i| i
            .get("system")
            .map(|s| s.as_str() != Some(""))
            .unwrap_or(true)));
        assert_eq!(sr["code"]["text"], "MRI head without contrast");
        assert_eq!(
            sr["authoredOn"], "2026-09-05",
            "ORC-9 has no zone: date only"
        );

        let is = entry(&b, "ImagingStudy");
        let profile = is["meta"]["profile"][0].as_str().unwrap();
        assert_eq!(
            profile,
            format!(
                "{}|{}",
                ids::MII_IMAGING_STUDY_PROFILE,
                ids::MII_IMAGING_STUDY_VERSION
            )
        );
        assert_eq!(is["basedOn"][0]["reference"], full_urls[1]);
        assert_eq!(is["subject"]["reference"], full_urls[0]);
        assert_eq!(is["started"], "2004-08-26", "no (0008,0201): date only");
        assert_eq!(is["numberOfSeries"], 2);
        assert_eq!(is["numberOfInstances"], 3, "multi-frame counts once");
        assert_eq!(is["modality"][0]["code"], "MR");
        assert_eq!(is["modality"][0]["system"], ids::DCM);
        assert_eq!(is["series"][1]["numberOfInstances"], 2);
        assert_eq!(
            is["series"][1]["instance"][1]["sopClass"]["code"],
            "urn:oid:1.2.840.10008.5.1.4.1.1.4"
        );
        assert_eq!(
            is["series"][1]["instance"][1]["sopClass"]["system"],
            ids::RFC3986
        );
        assert_eq!(is["series"][1]["instance"][1]["number"], 2);
        assert_eq!(is["description"], "Head");
    }

    #[test]
    fn omi_bundle_reads_ipc_and_labels_the_source() {
        let (b, linkage) = build(&sample("order-omi.hl7"), &study());
        assert_eq!(linkage.path, LinkPath::StudyUid);
        let sr = entry(&b, "ServiceRequest");
        let ids = sr["identifier"].as_array().unwrap();
        assert!(ids.iter().any(|i| i["system"] == "urn:dicom:uid"));
        let acsn = ids
            .iter()
            .find(|i| i["type"]["coding"][0]["code"] == "ACSN")
            .unwrap();
        assert_eq!(acsn["value"], "ACC-2026-0003");
        assert_eq!(acsn["assigner"]["display"], "HOSP", "IPC-1.2 namespace");
        let msg = Message::parse(sample("order-omi.hl7")).unwrap();
        let order = Order::extract(&msg);
        assert_eq!(
            order.source_path(hl7kit::order::OrderField::StudyUid),
            Some("IPC-3.1")
        );
    }

    #[test]
    fn unlinked_study_has_no_based_on() {
        let other = Study {
            study_uid: Some("9.9.9".into()),
            accession_number: Some("OTHER".into()),
            ..study()
        };
        let (b, linkage) = build(&sample("order.hl7"), &other);
        assert_eq!(linkage.path, LinkPath::None);
        assert!(entry(&b, "ImagingStudy").get("basedOn").is_none());
    }

    #[test]
    fn started_carries_the_time_when_the_offset_is_known() {
        let with_zone = Study {
            timezone_offset: Some("+0200".into()),
            ..study()
        };
        let (b, _) = build(&sample("order.hl7"), &with_zone);
        assert_eq!(
            entry(&b, "ImagingStudy")["started"],
            "2004-08-26T18:50:59+02:00"
        );
    }

    #[test]
    fn gender_absent_and_unknown_are_omitted() {
        for sex in [None, Some("U".to_string())] {
            let s = Study {
                patient_sex: sex,
                ..study()
            };
            // The PID in the sample says `O`; DICOM wins when present, and
            // an absent DICOM value falls through to PID.
            let (b, _) = build(&sample("order.hl7"), &s);
            let p = entry(&b, "Patient");
            match s.patient_sex.as_deref() {
                None => assert_eq!(p["gender"], "other", "PID-8 fills in"),
                Some(_) => assert!(
                    p.get("gender").is_none(),
                    "DICOM `U` is omitted, not `unknown`"
                ),
            }
        }
    }

    #[test]
    fn fhir_elements_per_row() {
        use hl7kit::order::OrderField;
        assert!(fhir_element(OrderField::StudyUid).contains("urn:dicom:uid"));
        assert!(fhir_element(OrderField::ProcedureId).contains("not standardised"));
    }

    // ---- Bundle invariants ------------------------------------------------

    const SAMPLES: [&str; 6] = [
        "order.hl7",
        "order-omi.hl7",
        "order-oru.hl7",
        "order-mismatch.hl7",
        "anonymized-demo.hl7",
        "anonymized-demo-mismatch.hl7",
    ];

    fn assert_valid(b: &Value, what: &str) {
        let problems = validate::check(b);
        assert!(
            problems.is_empty(),
            "{what}: {}\n{}",
            problems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
            serde_json::to_string_pretty(b).unwrap()
        );
    }

    #[test]
    fn every_sample_order_gives_a_structurally_valid_bundle() {
        for name in SAMPLES {
            let (b, _) = build(&sample(name), &study());
            assert_valid(&b, name);
            // Linked or not, mismatch or not: still valid.
            let (b, _) = build(&sample(name), &Study::default());
            assert_valid(&b, &format!("{name} with an empty study"));
        }
    }

    #[test]
    fn references_resolve_to_full_urls_and_nothing_else() {
        let (b, _) = build(&sample("order.hl7"), &study());
        let full_urls: Vec<&str> = b["entry"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["fullUrl"].as_str().unwrap())
            .collect();
        assert!(full_urls.iter().all(|u| validate::is_uuid_urn(u)));
        let mut refs = Vec::new();
        collect_key(&b, "reference", &mut refs);
        assert_eq!(refs.len(), 3, "SR.subject, IS.subject, IS.basedOn");
        assert!(
            refs.iter().all(|r| full_urls.contains(&r.as_str())),
            "{refs:?}"
        );
        assert_eq!(
            b["entry"][0]["resource"]["resourceType"], "Patient",
            "entry order is fixed"
        );
        assert_eq!(b["entry"][1]["resource"]["resourceType"], "ServiceRequest");
        assert_eq!(b["entry"][2]["resource"]["resourceType"], "ImagingStudy");
    }

    fn collect_key(v: &Value, key: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, item) in m {
                    if k == key {
                        out.push(item.as_str().unwrap().to_string());
                    }
                    collect_key(item, key, out);
                }
            }
            Value::Array(a) => a.iter().for_each(|i| collect_key(i, key, out)),
            _ => {}
        }
    }

    #[test]
    fn bundle_is_deterministic_and_keyed_by_the_study() {
        let (a, _) = build(&sample("order.hl7"), &study());
        let (b, _) = build(&sample("order.hl7"), &study());
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            "byte-identical, key order included"
        );
        let other = Study {
            study_uid: Some("1.2.3.4.5".into()),
            ..study()
        };
        let (c, _) = build(&sample("order.hl7"), &other);
        assert_ne!(a["entry"][0]["fullUrl"], c["entry"][0]["fullUrl"]);
    }

    #[test]
    fn seed_falls_back_through_order_uid_accession_and_patient() {
        // The entry URNs must be stable for a study that lacks a UID, and
        // must not collide with those of a study that has one.
        let with_uid = Study {
            study_uid: Some("1.2.3".into()),
            ..Study::default()
        };
        let acc_only = Study {
            accession_number: Some("1.2.3".into()),
            ..Study::default()
        };
        let pid_only = Study {
            patient_id: Some("1.2.3".into()),
            ..Study::default()
        };
        let msg =
            Message::parse("MSH|^~\\&|A|B|||20260905||ORM^O01|1|P|2.5.1\rPID|1||P\r").unwrap();
        let order = Order::extract(&msg);
        let urns: Vec<String> = [&with_uid, &acc_only, &pid_only, &Study::default()]
            .into_iter()
            .map(|s| {
                let linkage = link::resolve(s, &order);
                let input = FhirInput {
                    study: s,
                    series: &[],
                    message: &msg,
                    order: &order,
                    linkage: &linkage,
                };
                resources(&input).refs.patient
            })
            .collect();
        // The seed is the same string `1.2.3` in the first three cases, so
        // they collide by design: the seed is an identity, not a source.
        assert_eq!(urns[0], urns[1]);
        assert_eq!(urns[1], urns[2]);
        assert_ne!(urns[2], urns[3], "`unidentified` is its own seed");
        assert_eq!(urns[3], ids::Refs::for_study("unidentified").patient);
    }

    #[test]
    fn hostile_inputs_never_panic_and_stay_valid() {
        let hostile = Study {
            patient_id: Some("\0\u{1}\u{2}".into()),
            accession_number: Some("  ".into()),
            study_uid: Some("not.a.uid.".into()),
            requested_procedure_id: Some("\"\"".into()),
            transfer_syntax: String::new(),
            modality: Some("?".into()),
            rows: 0,
            cols: 0,
            patient_name: Some("^^^^^^^^^^=山田=".into()),
            patient_birth_date: Some("20230229".into()),
            patient_sex: Some("X".into()),
            study_date: Some("9999-99-99".into()),
            study_time: Some("25:61:61".into()),
            timezone_offset: Some("+99:99".into()),
            study_description: Some("\u{7}\u{8}".into()),
        };
        let garbage_series = vec![Series {
            uid: "\0".into(),
            number: None,
            description: "\u{1}".into(),
            modality: "\u{1}".into(),
            rows: 0,
            cols: 0,
            slices: vec![slice(0, 0, 1, "")],
        }];
        let msg = Message::parse(
            "MSH|^~\\&|A|B|||20260905||ORM^O01|1|P|2.5.1\r\
             PID|1||\"\"^^^\"\"&\"\"&\"\"^MR||\"\"||\"\"|\"\"\r\
             ORC|\"\"|\"\"|\"\"||||||\"\"|||\"\"\r\
             OBR|1|||\"\"^\"\"^\"\"||||||||||||\"\"||\"\"|\"\"\r\
             ZDS|\"\"\r",
        )
        .unwrap();
        let order = Order::extract(&msg);
        let linkage = link::resolve(&hostile, &order);
        let input = FhirInput {
            study: &hostile,
            series: &garbage_series,
            message: &msg,
            order: &order,
            linkage: &linkage,
        };
        let b = bundle(&input);
        assert_valid(&b, "hostile");
        let p = entry(&b, "Patient");
        assert_eq!(p, &json!({ "resourceType": "Patient", "id": "patient-1" }));
        let is = entry(&b, "ImagingStudy");
        assert_eq!(is["numberOfSeries"], 0);
        assert!(is.get("identifier").is_none());
        assert!(is.get("started").is_none());
    }

    // ---- Property tests --------------------------------------------------

    mod props {
        use super::super::*;
        use crate::link;
        use proptest::prelude::*;

        fn is_iso_date_or_datetime(s: &str) -> bool {
            validate::is_fhir_datetime(s)
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(512))]

            /// Anything at all in: `None` or a valid FHIR `dateTime` out.
            #[test]
            fn hl7_datetime_total_and_valid(s in "\\PC{0,24}") {
                if let Some(out) = datetime::hl7_datetime(&s) {
                    prop_assert!(is_iso_date_or_datetime(&out), "{s:?} -> {out}");
                }
                if let Some(out) = datetime::hl7_date(&s) {
                    prop_assert!(validate::is_fhir_date(&out), "{s:?} -> {out}");
                }
            }

            /// Digit-heavy DTM-shaped input, to exercise the deep paths.
            #[test]
            fn hl7_datetime_on_dtm_shapes(
                digits in "[0-9]{0,16}",
                frac in proptest::option::of("\\.[0-9]{0,5}"),
                zone in proptest::option::of("[+-][0-9]{0,5}"),
            ) {
                let zone_ok = zone.as_deref().and_then(datetime::zone).is_some();
                let s = format!("{digits}{}{}", frac.unwrap_or_default(), zone.unwrap_or_default());
                if let Some(out) = datetime::hl7_datetime(&s) {
                    prop_assert!(is_iso_date_or_datetime(&out), "{s:?} -> {out}");
                    // A DTM with a valid zone and at least HHMM keeps its time.
                    if zone_ok && digits.len() >= 12 {
                        prop_assert!(out.contains('T'), "{s:?} -> {out}: time dropped");
                    }
                }
            }

            #[test]
            fn dicom_datetime_total_and_valid(
                da in "\\PC{0,12}",
                tm in proptest::option::of("\\PC{0,16}"),
                off in proptest::option::of("\\PC{0,8}"),
            ) {
                if let Some(out) = datetime::dicom_datetime(&da, tm.as_deref(), off.as_deref()) {
                    prop_assert!(is_iso_date_or_datetime(&out), "{da:?} {tm:?} {off:?} -> {out}");
                    // The date alone must be valid, with or without a time.
                    prop_assert!(validate::is_fhir_date(&out[..10]), "{out}");
                }
                if let Some(out) = datetime::dicom_date(&da) {
                    prop_assert_eq!(out.len(), 10);
                }
                let _ = datetime::zone(off.as_deref().unwrap_or(""));
            }

            /// Valid calendar dates round-trip through both parsers.
            #[test]
            fn valid_dates_round_trip(y in 1u16..=9999, m in 1u8..=12, d in 1u8..=31) {
                let s = format!("{y:04}{m:02}{d:02}");
                let expect = format!("{y:04}-{m:02}-{d:02}");
                let real = d <= datetime::days_in_month(y, m);
                let expect = real.then_some(expect);
                prop_assert_eq!(&datetime::dicom_date(&s), &expect);
                prop_assert_eq!(&datetime::hl7_datetime(&s), &expect);
                prop_assert_eq!(&datetime::hl7_date(&s), &expect);
            }

            /// Every valid offset normalises; everything else is refused.
            #[test]
            fn zones_round_trip(sign in "[+-]", h in 0u8..=99, m in 0u8..=99) {
                let s = format!("{sign}{h:02}{m:02}");
                let out = datetime::zone(&s);
                if h <= 14 && m <= 59 {
                    prop_assert_eq!(out, Some(format!("{sign}{h:02}:{m:02}")));
                } else {
                    prop_assert_eq!(out, None);
                }
            }

            /// `Refs` are deterministic, type-distinct, well-formed URNs.
            #[test]
            fn refs_are_deterministic_and_distinct(seed in "\\PC{0,80}") {
                let a = ids::Refs::for_study(&seed);
                let b = ids::Refs::for_study(&seed);
                prop_assert_eq!(&a, &b);
                prop_assert_ne!(&a.patient, &a.service_request);
                prop_assert_ne!(&a.patient, &a.imaging_study);
                prop_assert_ne!(&a.service_request, &a.imaging_study);
                for u in [&a.patient, &a.service_request, &a.imaging_study] {
                    prop_assert!(validate::is_uuid_urn(u), "{u}");
                    let bare = &u["urn:uuid:".len()..];
                    prop_assert_eq!(&bare[14..15], "8", "version nibble in {}", u);
                    prop_assert!(matches!(&bare[19..20], "8" | "9" | "a" | "b"), "variant in {u}");
                }
            }

            /// Seeds that differ give different URNs (no trivial collision
            /// such as ignoring the tail of the seed).
            #[test]
            fn distinct_seeds_give_distinct_refs(a in "[0-9.]{1,64}", b in "[0-9.]{1,64}") {
                prop_assume!(a != b);
                prop_assert_ne!(ids::Refs::for_study(&a).patient, ids::Refs::for_study(&b).patient);
            }

            /// `is_uid` agrees with a direct reading of PS3.5 9.1.
            #[test]
            fn is_uid_matches_the_grammar(s in "[0-9.\\x00 a]{0,70}") {
                let core = s.trim_end_matches('\0').trim();
                let expect = !core.is_empty()
                    && core.len() <= 64
                    && core.contains('.')
                    && core.split('.').all(|c| {
                        !c.is_empty()
                            && c.bytes().all(|b| b.is_ascii_digit())
                            && (c == "0" || !c.starts_with('0'))
                    });
                prop_assert_eq!(ids::is_uid(&s), expect, "{:?}", s);
            }

            /// Names: never panic, never emit an empty element, the text is
            /// the parts joined, and the ideographic group never leaks.
            #[test]
            fn parse_name_total(s in "[\\PC=^\\\\ ]{0,40}") {
                for order in [patient::NameOrder::Dicom, patient::NameOrder::Hl7] {
                    let Some(n) = patient::parse_name(&s, order) else { continue };
                    let obj = n.as_object().unwrap();
                    prop_assert!(obj.contains_key("family") || obj.contains_key("given"));
                    for (k, v) in obj {
                        match v {
                            Value::String(t) => prop_assert!(!t.is_empty() && t == t.trim(), "{k}: {t:?}"),
                            Value::Array(a) => {
                                prop_assert!(!a.is_empty(), "{k}");
                                for item in a {
                                    let t = item.as_str().unwrap();
                                    prop_assert!(!t.is_empty() && t == t.trim(), "{k}: {t:?}");
                                    prop_assert!(!t.contains('^') && !t.contains('='), "{k}: {t:?}");
                                }
                            }
                            _ => prop_assert!(false, "unexpected {k}"),
                        }
                    }
                    if let Some((head, tail)) = s.split_once('=') {
                        // Any character that occurs only after the `=` must
                        // not reach the output; `text` carries every part
                        // (the join adds spaces, so those are exempt).
                        let text = n["text"].as_str().unwrap();
                        for c in tail.chars().filter(|c| *c != ' ' && !head.contains(*c)) {
                            prop_assert!(!text.contains(c), "{s:?}: {c:?} leaked into {text}");
                        }
                    }
                }
            }

            /// `fhir_string` output is always clean or absent.
            #[test]
            fn fhir_string_is_clean(s in "\\PC{0,32}") {
                if let Some(out) = ids::fhir_string(&s) {
                    prop_assert!(!out.is_empty());
                    prop_assert_eq!(out.as_str(), out.trim());
                    prop_assert!(out.chars().all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r')));
                    prop_assert_ne!(out.as_str(), "\"\"");
                }
            }

            /// Any HD gives a system that is a URI, or none.
            #[test]
            fn authority_system_is_always_a_uri(hd in "\\PC{0,48}") {
                let a = ids::Authority::from_hd(Some(&hd));
                if let Some(sys) = &a.system {
                    prop_assert!(ids::is_uri(sys), "{hd:?} -> {sys}");
                }
                if let Some(d) = &a.assigner {
                    prop_assert!(!d.is_empty() && d == d.trim());
                }
            }

            /// A whole bundle from a fuzzed study header stays valid.
            #[test]
            fn fuzzed_study_header_gives_a_valid_bundle(
                patient_id in proptest::option::of("\\PC{0,12}"),
                accession in proptest::option::of("\\PC{0,12}"),
                study_uid in proptest::option::of("[0-9.a]{0,20}"),
                name in proptest::option::of("[\\PC^=]{0,24}"),
                birth in proptest::option::of("[0-9.]{0,10}"),
                sex in proptest::option::of("[MFOUmfx ]{0,2}"),
                date in proptest::option::of("[0-9.]{0,10}"),
                time in proptest::option::of("[0-9.:]{0,12}"),
                offset in proptest::option::of("[+-]?[0-9]{0,5}"),
                desc in proptest::option::of("\\PC{0,12}"),
                modality in "[A-Z?a]{0,3}",
            ) {
                let s = Study {
                    patient_id,
                    accession_number: accession,
                    study_uid,
                    requested_procedure_id: None,
                    transfer_syntax: String::new(),
                    modality: Some(modality),
                    rows: 0,
                    cols: 0,
                    patient_name: name,
                    patient_birth_date: birth,
                    patient_sex: sex,
                    study_date: date,
                    study_time: time,
                    timezone_offset: offset,
                    study_description: desc,
                };
                let msg = Message::parse(super::sample("order.hl7")).unwrap();
                let order = Order::extract(&msg);
                let linkage = link::resolve(&s, &order);
                let series = super::series();
                let input = FhirInput {
                    study: &s,
                    series: &series,
                    message: &msg,
                    order: &order,
                    linkage: &linkage,
                };
                let b = bundle(&input);
                let problems = validate::check(&b);
                prop_assert!(problems.is_empty(), "{:#?}\n{}", problems, serde_json::to_string_pretty(&b).unwrap());
            }
        }
    }
}
