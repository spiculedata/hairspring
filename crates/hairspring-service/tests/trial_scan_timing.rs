//! Timed reads over a run of 10,000 trials with 50 properties each, to decide whether the seed explorer needs a cache.
//!
//! Run with `cargo test --release -p hairspring-service --test trial_scan_timing -- --ignored --nocapture`. The tests
//! print what they measured and assert only the plan's targets (section 6): `fails=` under one second cold.
mod common;

use common::*;
use hairspring_engine::trial_log::ReportedVerdict;
use hairspring_props::AssertKind::Always;
use hairspring_service::{PropertyQuery, StripQuery, TrialQuery};
use std::time::Instant;

const TRIALS: u64 = 10_000;
/// Properties per trial unless `HAIRSPRING_BENCH_IDS` says otherwise.
const DEFAULT_IDS: usize = 50;
const BATCH: u64 = 256;
/// Every this-many-th seed fails the last property.
const FAIL_EVERY: u64 = 997;

fn id_count() -> usize {
    std::env::var("HAIRSPRING_BENCH_IDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_IDS)
}

fn ids() -> Vec<String> {
    (0..id_count())
        .map(|i| format!("bench.prop_{i:04}"))
        .collect()
}

fn big_run() -> (hairspring_service::Service, String) {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), TRIALS);
    let names = ids();
    let mut recorder = Recorder::new();
    let mut batch = Vec::new();
    for seed in 1..=TRIALS {
        let fails = u64::from(seed % FAIL_EVERY == 0);
        let specs: Vec<IdSpec<'_>> = names
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let falses = if i == id_count() - 1 { fails } else { 0 };
                (id.as_str(), Always, "m", 4 - falses, falses)
            })
            .collect();
        let verdict = if fails == 1 {
            ReportedVerdict::Violation
        } else {
            ReportedVerdict::Pass
        };
        batch.push(recorder.trial(seed, &props_multi(&specs), verdict, false));
        if batch.len() as u64 == BATCH {
            svc.ingest_trials(&job.job_id, &local(), &batch).unwrap();
            batch.clear();
        }
    }
    if !batch.is_empty() {
        svc.ingest_trials(&job.job_id, &local(), &batch).unwrap();
    }
    finish(&svc, &job, &local(), passed(TRIALS, Some(TRIALS)));
    (svc, job.run_id)
}

fn idx(svc: &hairspring_service::Service, run: &str, id: &str) -> String {
    svc.property_page(
        run,
        &PropertyQuery {
            q: Some(id.into()),
            ..Default::default()
        },
        None,
    )
    .unwrap()
    .unwrap()
    .properties[0]
        .idx
        .to_string()
}

fn timed<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    println!("{what}: {:?}", t.elapsed());
    out
}

#[test]
#[ignore = "timing; run in release with --ignored --nocapture"]
fn filters_over_ten_thousand_trials_with_fifty_properties() {
    let t = Instant::now();
    let (svc, run) = big_run();
    println!(
        "setup (ingest {TRIALS} trials x {} ids): {:?}",
        id_count(),
        t.elapsed()
    );

    let last = idx(&svc, &run, &format!("bench.prop_{:04}", id_count() - 1));
    let first = idx(&svc, &run, "bench.prop_0000");
    let q = |fails: Option<&str>| TrialQuery {
        fails: fails.map(str::to_owned),
        limit: Some(500),
        ..Default::default()
    };

    let all = timed("unfiltered page of 500", || {
        svc.trial_page(&run, &q(None)).unwrap().unwrap()
    });
    assert_eq!(all.trials.len(), 500);

    // A property that never fails matches nothing, so the scan reads every trial: the worst case.
    let none = timed(
        "fails=<never failing> (full scan of 10,000, no match)",
        || svc.trial_page(&run, &q(Some(&first))).unwrap().unwrap(),
    );
    assert!(none.trials.is_empty());
    println!("  scanned {} trials", none.scanned_trials);

    let some = timed("fails=<rarely failing> (full scan, 10 matches)", || {
        svc.trial_page(&run, &q(Some(&last))).unwrap().unwrap()
    });
    assert_eq!(some.trials.len() as u64, TRIALS / FAIL_EVERY);

    let warm = timed("fails=<rarely failing> again (no cache: same cost)", || {
        svc.trial_page(&run, &q(Some(&last))).unwrap().unwrap()
    });
    assert_eq!(warm.trials.len(), some.trials.len());

    let strip = timed("strip of 10,000", || {
        svc.strip(&run, &StripQuery::default()).unwrap().unwrap()
    });
    assert_eq!(strip.codes.len() as u64, TRIALS);
}
