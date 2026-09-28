//! FHIR mapping benchmarks: `cargo bench -p dicomscope-core`.
//!
//! The mapping runs on every order/study pair loaded in the browser and
//! once per bundle in CI; these numbers say what a bundle costs and which
//! part of it. Inputs are the committed sample orders and a synthetic
//! series list; no DICOM file is parsed here (that is `dicom::StudySet`'s
//! cost, not the mapping's).

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use dicomscope_core::dicom::series::Slice;
use dicomscope_core::dicom::{Series, Study};
use dicomscope_core::fhir::{self, datetime, ids, patient, validate, FhirInput};
use dicomscope_core::link;
use hl7kit::order::Order;
use hl7kit::Message;
use std::hint::black_box;

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
        rows: 512,
        cols: 512,
        patient_name: Some("CompressedSamples^MR1^^Dr^PhD".into()),
        patient_birth_date: Some("19700101".into()),
        patient_sex: Some("O".into()),
        study_date: Some("20040826".into()),
        study_time: Some("185059.123456".into()),
        timezone_offset: Some("+0200".into()),
        study_description: Some("Head".into()),
    }
}

/// `n` series of `per_series` single-frame instances each.
fn series(n: usize, per_series: usize) -> Vec<Series> {
    (1..=n)
        .map(|s| Series {
            uid: format!("{UID}.{s}"),
            number: Some(s as i32),
            description: format!("Series {s}"),
            modality: if s % 2 == 0 { "CT" } else { "PT" }.into(),
            rows: 512,
            cols: 512,
            slices: (1..=per_series)
                .map(|i| Slice {
                    file: (s - 1) * per_series + i,
                    frame: 0,
                    instance_number: Some(i as i32),
                    position: Some(i as f64 * 2.5),
                    sop_instance_uid: format!("{UID}.{s}.{i}"),
                    sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
                })
                .collect(),
        })
        .collect()
}

fn bench_bundle(c: &mut Criterion) {
    let study = study();
    let mut group = c.benchmark_group("bundle");
    for name in ["order.hl7", "order-omi.hl7", "order-oru.hl7"] {
        let text = sample(name);
        let msg = Message::parse(&text).unwrap();
        let order = Order::extract(&msg);
        let linkage = link::resolve(&study, &order);
        let series = series(2, 3);
        let input = FhirInput {
            study: &study,
            series: &series,
            message: &msg,
            order: &order,
            linkage: &linkage,
        };
        group.bench_with_input(BenchmarkId::new("map", name), &input, |b, input| {
            b.iter(|| fhir::bundle(black_box(input)))
        });
        // What the CLI does end to end from text: parse, extract, link, map,
        // check, serialise.
        group.bench_with_input(BenchmarkId::new("text_to_json", name), &text, |b, text| {
            b.iter(|| {
                let msg = Message::parse(black_box(text.as_str())).unwrap();
                let order = Order::extract(&msg);
                let linkage = link::resolve(&study, &order);
                let input = FhirInput {
                    study: &study,
                    series: &series,
                    message: &msg,
                    order: &order,
                    linkage: &linkage,
                };
                let bundle = fhir::bundle(&input);
                assert!(validate::check(&bundle).is_empty());
                serde_json::to_string(&bundle).unwrap()
            })
        });
    }
    group.finish();
}

/// The ImagingStudy resource grows with the study; everything else is
/// constant. Throughput is instances per second.
fn bench_series_scaling(c: &mut Criterion) {
    let study = study();
    let text = sample("order.hl7");
    let msg = Message::parse(&text).unwrap();
    let order = Order::extract(&msg);
    let linkage = link::resolve(&study, &order);
    let mut group = c.benchmark_group("imaging_study_scaling");
    for (n, per) in [(1, 1), (2, 50), (10, 100), (20, 500)] {
        let series = series(n, per);
        let instances = (n * per) as u64;
        group.throughput(Throughput::Elements(instances));
        let input = FhirInput {
            study: &study,
            series: &series,
            message: &msg,
            order: &order,
            linkage: &linkage,
        };
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{n}x{per}")),
            &input,
            |b, input| b.iter(|| fhir::bundle(black_box(input))),
        );
    }
    group.finish();
}

fn bench_validate(c: &mut Criterion) {
    let study = study();
    let text = sample("order.hl7");
    let msg = Message::parse(&text).unwrap();
    let order = Order::extract(&msg);
    let linkage = link::resolve(&study, &order);
    let mut group = c.benchmark_group("validate");
    for (n, per) in [(2, 3), (10, 100)] {
        let series = series(n, per);
        let input = FhirInput {
            study: &study,
            series: &series,
            message: &msg,
            order: &order,
            linkage: &linkage,
        };
        let bundle = fhir::bundle(&input);
        group.throughput(Throughput::Elements((n * per) as u64));
        group.bench_with_input(
            BenchmarkId::new("check", format!("{n}x{per}")),
            &bundle,
            |b, bundle| b.iter(|| validate::check(black_box(bundle))),
        );
        group.bench_with_input(
            BenchmarkId::new("serialize_pretty", format!("{n}x{per}")),
            &bundle,
            |b, bundle| b.iter(|| serde_json::to_string_pretty(black_box(bundle)).unwrap()),
        );
    }
    group.finish();
}

fn bench_primitives(c: &mut Criterion) {
    let mut group = c.benchmark_group("primitives");
    for (label, input) in [
        ("hl7_datetime/full", "20260905113000.1234+0200"),
        ("hl7_datetime/date", "20260905"),
        ("hl7_datetime/garbage", "not a date at all, really"),
    ] {
        group.bench_function(label, |b| {
            b.iter(|| datetime::hl7_datetime(black_box(input)))
        });
    }
    group.bench_function("dicom_datetime/full", |b| {
        b.iter(|| {
            datetime::dicom_datetime(
                black_box("20151207"),
                black_box(Some("073153.250000")),
                black_box(Some("-0500")),
            )
        })
    });
    group.bench_function("parse_name/dicom", |b| {
        b.iter(|| {
            patient::parse_name(
                black_box("Doe^Jane^Marie^Dr^PhD"),
                patient::NameOrder::Dicom,
            )
        })
    });
    group.bench_function("parse_name/ideographic", |b| {
        b.iter(|| {
            patient::parse_name(
                black_box("Yamada^Tarou=山田^太郎=やまだ^たろう"),
                patient::NameOrder::Dicom,
            )
        })
    });
    group.bench_function("refs_for_study", |b| {
        b.iter(|| ids::Refs::for_study(black_box(UID)))
    });
    group.bench_function("is_uid", |b| b.iter(|| ids::is_uid(black_box(UID))));
    group.bench_function("fhir_string", |b| {
        b.iter(|| ids::fhir_string(black_box("  MRI head without contrast \0")))
    });
    group.bench_function("authority_from_hd", |b| {
        b.iter(|| ids::Authority::from_hd(black_box(Some("HOSP&1.2.276.0.76.3.1.1&ISO"))))
    });
    group.bench_function("is_fhir_datetime", |b| {
        b.iter(|| validate::is_fhir_datetime(black_box("2026-09-05T11:30:00.1234+02:00")))
    });
    group.finish();
}

/// The parse step the mapping depends on, for proportion.
fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("hl7_parse");
    for name in ["order.hl7", "order-omi.hl7", "order-oru.hl7"] {
        let text = sample(name);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("parse_and_extract", name),
            &text,
            |b, text| {
                b.iter_batched(
                    || text.clone(),
                    |t| {
                        let msg = Message::parse(t).unwrap();
                        Order::extract(&msg)
                    },
                    BatchSize::SmallInput,
                )
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_bundle,
    bench_series_scaling,
    bench_validate,
    bench_primitives,
    bench_parse
);
criterion_main!(benches);
