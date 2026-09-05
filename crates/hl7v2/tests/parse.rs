use hl7v2::order::{Order, OrderField};
use hl7v2::{Encoding, Message, ParseError, Path, Warning};

const SAMPLE: &str = include_str!("fixtures/order.hl7");

fn msg(text: &str) -> Message {
    Message::parse(text).expect("parses")
}

#[test]
fn sample_fixture_uses_cr_terminators() {
    assert!(SAMPLE.contains('\r'));
    assert!(!SAMPLE.contains('\n'));
}

#[test]
fn msh_numbering_is_not_off_by_one() {
    let m = msg(SAMPLE);
    let msh = m.segment("MSH").unwrap();
    assert_eq!(msh.field(1).unwrap().value(), "|");
    assert_eq!(msh.field(2).unwrap().value(), "^~\\&");
    assert_eq!(msh.field(3).unwrap().value(), "RIS");
    assert_eq!(m.get("MSH-9"), Some("ORM^O01^ORM_O01"));
    assert_eq!(m.get("MSH-9.2"), Some("O01"));
    assert_eq!(m.get("MSH-10"), Some("MSG0001"));
    assert_eq!(m.get("MSH-12"), Some("2.5.1"));
    let mt = m.message_type().unwrap();
    assert_eq!(
        (mt.code, mt.trigger, mt.structure),
        ("ORM", "O01", "ORM_O01")
    );
    assert_eq!(m.control_id(), Some("MSG0001"));
    assert_eq!(m.version(), Some("2.5.1"));
    // MSH-2 is opaque: the component separator inside it must not split it.
    assert_eq!(msh.field(2).unwrap().component(1).unwrap().value(), "^~\\&");
    assert_eq!(msh.field(2).unwrap().repetition_count(), 1);
}

#[test]
fn other_segments_index_from_name() {
    let m = msg(SAMPLE);
    let pid = m.segment("PID").unwrap();
    assert_eq!(pid.field(1).unwrap().value(), "1");
    assert!(pid.field(2).unwrap().is_empty());
    assert_eq!(pid.field(3).unwrap().component(1).unwrap().value(), "4MR1");
    assert_eq!(m.get("PID-3.4"), Some("HOSP"));
    assert_eq!(m.get("PID-5.1"), Some("CompressedSamples"));
    assert_eq!(m.get("OBR-18"), Some("ACC-2026-0001"));
    assert_eq!(m.get("OBR-19"), Some("RP-2026-0001"));
}

#[test]
fn spans_point_back_into_raw_text() {
    let m = msg(SAMPLE);
    for path in ["MSH-9.1", "PID-3.1", "OBR-18", "ZDS-1.1", "OBR", "PID-3"] {
        let span = m.get_span(path).unwrap();
        assert_eq!(&m.raw()[span.range()], m.get(path).unwrap(), "{path}");
    }
    let seg = m.segment("ZDS").unwrap();
    assert!(seg.text().starts_with("ZDS|"));
    assert!(!seg.text().ends_with('\r'));
}

#[test]
fn line_endings_lf_and_crlf() {
    let cr = msg(SAMPLE);
    let lf = msg(&SAMPLE.replace('\r', "\n"));
    let crlf = msg(&SAMPLE.replace('\r', "\r\n"));
    for m in [&lf, &crlf] {
        assert_eq!(m.segment_count(), cr.segment_count());
        assert_eq!(m.get("ZDS-1.1"), cr.get("ZDS-1.1"));
        assert_eq!(m.get("OBR-19"), cr.get("OBR-19"));
        assert!(m.warnings().contains(&Warning::NonStandardTerminator));
    }
    assert!(cr.warnings().is_empty());
}

#[test]
fn blank_lines_bom_and_mllp_framing_are_tolerated() {
    let text = format!("\u{feff}\x0b{}\r\r\x1c\r", SAMPLE);
    let m = msg(&text);
    assert_eq!(m.segment_count(), 6);
    assert_eq!(m.segment_at(0).unwrap().name(), "MSH");
    assert!(m.warnings().contains(&Warning::ByteOrderMark));
    assert!(m.warnings().contains(&Warning::MllpFraming));
}

#[test]
fn absent_zds_gives_none() {
    let without: String = SAMPLE
        .split('\r')
        .filter(|l| !l.starts_with("ZDS"))
        .collect::<Vec<_>>()
        .join("\r");
    let m = msg(&without);
    assert_eq!(m.get("ZDS-1.1"), None);
    let order = Order::extract(&m);
    assert_eq!(order.study_uid, None);
    assert_eq!(order.patient_id.as_deref(), Some("4MR1"));
    assert!(order.spans.iter().all(|(_, f)| *f != OrderField::StudyUid));
}

#[test]
fn zds_takes_first_component_only() {
    let m = msg(SAMPLE);
    assert_eq!(
        m.get("ZDS-1"),
        Some("1.3.6.1.4.1.5962.1.2.4.20040826185059.5457^^Application^DICOM")
    );
    let order = Order::extract(&m);
    assert_eq!(
        order.study_uid.as_deref(),
        Some("1.3.6.1.4.1.5962.1.2.4.20040826185059.5457")
    );
    assert_eq!(order.accession.as_deref(), Some("ACC-2026-0001"));
    assert_eq!(order.procedure_id.as_deref(), Some("RP-2026-0001"));
    assert_eq!(order.spans.len(), 4);
}

#[test]
fn custom_encoding_characters() {
    let text = "MSH#!*?%#APP#FAC#APP2#FAC2#20260101##ADT!A01#1#P#2.3\rPID#1##ID1!!!HOSP*ID2!!!OTHER##Last!First%Middle";
    let m = msg(text);
    assert_eq!(m.encoding().field, b'#');
    assert_eq!(m.encoding().component, b'!');
    assert_eq!(m.encoding().repetition, b'*');
    assert_eq!(m.encoding().escape, b'?');
    assert_eq!(m.encoding().subcomponent, b'%');
    assert_eq!(m.get("MSH-1"), Some("#"));
    assert_eq!(m.get("MSH-2"), Some("!*?%"));
    assert_eq!(m.get("MSH-9.2"), Some("A01"));
    assert_eq!(m.get("PID-3.1"), Some("ID1"));
    assert_eq!(m.get("PID-3[2].1"), Some("ID2"));
    assert_eq!(m.get("PID-3[2].4"), Some("OTHER"));
    assert_eq!(m.get("PID-5.2.2"), Some("Middle"));
    assert!(m.warnings().is_empty());
}

#[test]
fn malformed_msh2_falls_back_to_defaults() {
    let text = "MSH|^^\\&|APP|FAC|||20260101||ADT^A01|1|P|2.3\rPID|1||X^Y";
    let m = msg(text);
    assert_eq!(m.encoding(), Encoding::STANDARD);
    assert!(matches!(m.warnings()[0], Warning::EncodingFallback { ref found } if found == "^^\\&"));
    assert_eq!(m.get("MSH-9.1"), Some("ADT"));
    assert_eq!(m.get("PID-3.2"), Some("Y"));
}

#[test]
fn empty_and_trailing_fields() {
    let m = msg("MSH|^~\\&|A|B|||||ADT^A01|1|P|2.3\rPID|||||\rNTE\rPV1|1|\"\"|");
    let pid = m.segment("PID").unwrap();
    assert_eq!(pid.field_count(), 5);
    assert!(pid.field(5).unwrap().is_empty());
    assert!(pid.field(6).is_none());
    assert_eq!(m.get("PID-5"), Some(""));
    assert_eq!(m.get("PID-6"), None);
    assert_eq!(m.get("PID-5.1"), Some(""));
    let nte = m.segment("NTE").unwrap();
    assert_eq!(nte.field_count(), 0);
    assert!(nte.field(1).is_none());
    let pv1 = m.segment("PV1").unwrap();
    assert!(pv1.field(2).unwrap().is_null());
    assert!(!pv1.field(1).unwrap().is_null());
    assert_eq!(pv1.field_count(), 3);
}

#[test]
fn repetitions_components_subcomponents() {
    let m = msg("MSH|^~\\&|A|B|||||ADT^A01|1|P|2.3\rPID|1||A^^^H1~B^^^H2&sub|");
    let f = m.segment("PID").unwrap().field(3).unwrap();
    assert_eq!(f.repetition_count(), 2);
    let reps: Vec<_> = f.repetitions().map(|r| r.value()).collect();
    assert_eq!(reps, ["A^^^H1", "B^^^H2&sub"]);
    let r2 = f.repetition(2).unwrap();
    assert_eq!(r2.component_count(), 4);
    let c4 = r2.component(4).unwrap();
    assert_eq!(c4.subcomponent_count(), 2);
    assert_eq!(c4.subcomponent(2).unwrap().value(), "sub");
    assert_eq!(m.get("PID-3[2].4.2"), Some("sub"));
    assert_eq!(m.get("PID-3[3]"), None);
    assert_eq!(m.get("PID-3[2]"), Some("B^^^H2&sub"));
    assert_eq!(f.component(1).unwrap().value(), "A");
}

#[test]
fn segment_occurrences() {
    let m =
        msg("MSH|^~\\&|A|B|||||ORU^R01|1|P|2.3\rOBX|1|NM|A||1\rOBX|2|NM|B||2\rOBX|3|ST|C||three");
    assert_eq!(m.segments_named("OBX").count(), 3);
    assert_eq!(m.get("OBX-5"), Some("1"));
    assert_eq!(m.get("OBX[3]-5"), Some("three"));
    assert_eq!(m.get("OBX[4]-5"), None);
    assert_eq!(m.get("obx[2]-3"), Some("B"));
    let path = Path::parse("OBX[2]-5").unwrap();
    assert_eq!(m.resolve(&path).map(|s| s.slice(m.raw())), Some("2"));
}

#[test]
fn escapes_decode_on_demand() {
    let m = msg("MSH|^~\\&|A|B|||||ADT^A01|1|P|2.3\rPID|1||X||Smith \\T\\ Jones\\S\\Jr|\rNTE|1||line one\\.br\\line two");
    assert_eq!(m.get("PID-5"), Some("Smith \\T\\ Jones\\S\\Jr"));
    assert_eq!(m.get_decoded("PID-5").unwrap(), "Smith & Jones^Jr");
    assert_eq!(
        m.segment("NTE").unwrap().field(3).unwrap().decoded(),
        "line one\nline two"
    );
}

#[test]
fn order_values_are_decoded_and_trimmed() {
    let m = msg("MSH|^~\\&|A|B|||||ORM^O01|1|P|2.3\rPID|1|| 4MR1 ^^^H\rOBR|1|||||||||||||||||A\\S\\1|   \rZDS|1.2.3^^App^DICOM");
    let o = Order::extract(&m);
    assert_eq!(o.patient_id.as_deref(), Some("4MR1"));
    assert_eq!(o.accession.as_deref(), Some("A^1"));
    assert_eq!(o.procedure_id, None);
    assert_eq!(o.study_uid.as_deref(), Some("1.2.3"));
    assert_eq!(o.get(OrderField::StudyUid), Some("1.2.3"));
    assert_eq!(o.spans.len(), 3);
}

#[test]
fn errors_name_the_problem() {
    assert_eq!(Message::parse("").unwrap_err(), ParseError::Empty);
    assert_eq!(Message::parse("\r\n  \r\n").unwrap_err(), ParseError::Empty);
    assert_eq!(
        Message::parse("PID|1||X").unwrap_err(),
        ParseError::MissingMsh {
            found: "PID|1||X".into()
        }
    );
    assert!(matches!(
        Message::parse("MSH"),
        Err(ParseError::MalformedMsh { .. })
    ));
    assert!(matches!(
        Message::parse("MSHA^~\\&|X"),
        Err(ParseError::MalformedMsh { .. })
    ));
    let bad = Message::parse("MSH|^~\\&|A|B|||||ADT^A01|1|P|2.3\rPID|1\rthis is not a segment|x");
    let bad = bad.unwrap_err();
    assert_eq!(
        bad,
        ParseError::MalformedSegment {
            line: 3,
            text: "this is not a segment|x".into()
        }
    );
    assert_eq!(
        bad.to_string(),
        "line 3: not a segment: \"this is not a segment|x\""
    );
    assert!(matches!(
        Message::parse_bytes(b"MSH|^~\\&|\xff"),
        Err(ParseError::InvalidUtf8 { valid_up_to: 9 })
    ));
    let lossy =
        Message::parse_lossy(b"MSH|^~\\&|A|B|||||ADT^A01|1|P|2.3\rPID|1||X||M\xfcller").unwrap();
    assert_eq!(lossy.get("PID-5"), Some("M\u{fffd}ller"));
}

#[test]
fn minimal_and_odd_messages_do_not_panic() {
    for text in [
        "MSH|",
        "MSH|^~\\&",
        "MSH|^~\\&|",
        "MSH|^~\\&\rPID",
        "MSH|^~\\&|||||||||||\r\r\r",
        "MSH|^~\\&|A\rZZZ|~~~^^^&&&",
        "MSH|\u{e9}",
    ] {
        let _ = Message::parse(text);
    }
    let m = msg("MSH|^~\\&|A\rZZZ|~~~^^^&&&");
    assert_eq!(
        m.segment("ZZZ")
            .unwrap()
            .field(1)
            .unwrap()
            .repetition_count(),
        4
    );
    assert_eq!(m.get("ZZZ-1[4].4.4"), Some(""));
    assert_eq!(m.get("ZZZ-1[5]"), None);
}
