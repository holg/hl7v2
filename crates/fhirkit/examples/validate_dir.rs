//! Validate every FHIR R4 JSON resource in a folder and report timings.
//!
//! ```text
//! cargo run --profile native -p fhirkit --example validate_dir -- <folder> [--threads N]
//! cargo run --profile native -p fhirkit --example validate_dir -- <folder> --sweep ROUNDS
//! ```
//!
//! Files are read into memory first, so the timings are fhirkit's alone.
//! `--threads 0` (the default) uses one thread per core. Build with a speed
//! profile: the workspace's `release` profile optimises for size.
//!
//! `--sweep` times [`from_json_many`] for every thread count from 1 to the
//! number of cores. Each round tries every count once, in turn, so load from
//! other programs spreads over all counts instead of skewing a few; it
//! prints the median of the rounds per count. Close other busy programs
//! first.

use fhirkit::r4::Resource;
use fhirkit::{decode_json, from_json_many, FhirJson, Json, Severity};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first() else {
        eprintln!("usage: validate_dir <folder> [--threads N]");
        std::process::exit(2);
    };
    let threads: usize = args
        .iter()
        .position(|a| a == "--threads")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);

    let mut docs: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        // Skip hidden files: a package's `.index.json` is a catalogue.
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter(|t| t.contains("\"resourceType\""))
        .collect();
    docs.sort();
    let texts: Vec<&str> = docs.iter().map(String::as_str).collect();
    if let Some(rounds) = args
        .iter()
        .position(|a| a == "--sweep")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse::<usize>().ok())
    {
        return sweep(&texts, rounds.max(1));
    }
    let mb = texts.iter().map(|t| t.len()).sum::<usize>() as f64 / 1e6;

    // One thread, step by step.
    let t = Instant::now();
    let parsed: Vec<Json> = texts.iter().filter_map(|t| Json::parse(t).ok()).collect();
    let t_parse = t.elapsed();
    let t = Instant::now();
    let resources: Vec<Resource> = parsed
        .iter()
        .filter_map(|j| decode_json::<Resource>(j).0)
        .collect();
    let t_decode = t.elapsed();
    let t = Instant::now();
    let written: usize = resources
        .iter()
        .map(|r| r.encode().to_compact().len())
        .sum();
    let t_write = t.elapsed();

    // All at once, in parallel.
    let t = Instant::now();
    let many = from_json_many::<Resource>(&texts, threads);
    let t_many = t.elapsed();
    let invalid = many
        .iter()
        .filter(|r| match r {
            Ok((_, issues)) => issues.iter().any(|i| i.severity == Severity::Error),
            Err(_) => true,
        })
        .count();

    println!(
        "{} resources, {mb:.1} MB, {invalid} with errors",
        many.len()
    );
    println!(
        "  1 thread   JSON parse       {:>6.0} ms",
        t_parse.as_secs_f64() * 1e3
    );
    println!(
        "  1 thread   decode+validate  {:>6.0} ms",
        t_decode.as_secs_f64() * 1e3
    );
    println!(
        "  1 thread   write JSON       {:>6.0} ms  ({:.1} MB)",
        t_write.as_secs_f64() * 1e3,
        written as f64 / 1e6
    );
    let n = if threads == 0 {
        std::thread::available_parallelism().map_or(1, usize::from)
    } else {
        threads
    };
    println!(
        "  {n:>2} threads parse+validate   {:>6.0} ms  (from_json_many)",
        t_many.as_secs_f64() * 1e3
    );
}

fn sweep(texts: &[&str], rounds: usize) {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); cores];
    from_json_many::<Resource>(texts, cores); // warm up
    for _ in 0..rounds {
        for (i, slot) in times.iter_mut().enumerate() {
            let t = Instant::now();
            from_json_many::<Resource>(texts, i + 1);
            slot.push(t.elapsed().as_secs_f64() * 1e3);
        }
    }
    println!(
        "{} documents, {rounds} rounds, median ms per thread count:",
        texts.len()
    );
    let best = times
        .iter()
        .map(|t| median(t))
        .fold(f64::INFINITY, f64::min);
    for (i, t) in times.iter().enumerate() {
        let m = median(t);
        let lo = t.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = t.iter().copied().fold(0.0, f64::max);
        let mark = if m == best { "  <- fastest" } else { "" };
        println!("  {:>2} threads  {m:>6.0}  ({lo:.0}-{hi:.0}){mark}", i + 1);
    }
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[s.len() / 2]
}
