//! One stored digest that cannot be read must not take the whole trial page down with it.
mod common;

use common::*;
use hairspring_props::AssertKind;
use hairspring_service::TrialQuery;
use hairspring_store::{NewTrial, TrialBatch, TrialVerdict, MAX_KEPT_BYTES_PER_RUN};

const GOOD_SEED: u64 = 1;
const CORRUPT_SEED: u64 = 2;

fn corrupt_trial(seq: u64) -> NewTrial {
    NewTrial {
        seq,
        seed: CORRUPT_SEED,
        verdict: TrialVerdict::Pass,
        reason: String::new(),
        exit_code: Some(0),
        signal: None,
        timed_out: false,
        degraded: false,
        perturbing: false,
        report_missing: false,
        rated: true,
        claimable: true,
        behaviour_hash: None,
        schedule_fp: None,
        syscalls: 1,
        work_ops: 1,
        elapsed_ms: 1,
        max_idx: None,
        digest: vec![9, 9, 9],
        detail: None,
        output: None,
    }
}

/// A run holding one good trial (seq 0) and one whose digest is garbage (seq 1).
fn run_with_a_corrupt_digest() -> (hairspring_service::Service, String) {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 2);
    let good = Recorder::new().pass(GOOD_SEED, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job.job_id, &local(), &[good]).unwrap();
    let batch = TrialBatch {
        catalog: Vec::new(),
        trials: vec![corrupt_trial(1)],
        kept_bytes_cap: MAX_KEPT_BYTES_PER_RUN,
    };
    store
        .ingest_trials(
            &job.job_id,
            &local(),
            true,
            &batch,
            hairspring_store::now_ms(),
        )
        .unwrap();
    (svc, job.run_id)
}

#[test]
fn an_unreadable_digest_leaves_its_row_listed_with_no_failing_ids() {
    let (svc, run_id) = run_with_a_corrupt_digest();
    let page = svc
        .trial_page(&run_id, &TrialQuery::default())
        .expect("the page is served")
        .unwrap();
    let seeds: Vec<&str> = page.trials.iter().map(|t| t.seed.as_str()).collect();
    assert_eq!(seeds, ["1", "2"]);
    assert_eq!(page.trials[1].failing_count, 0);
    assert!(page.trials[1].guest.failing_ids.is_empty());
}

#[test]
fn a_digest_filter_skips_a_row_whose_digest_cannot_be_read() {
    let (svc, run_id) = run_with_a_corrupt_digest();
    let q = TrialQuery {
        hits: Some("0".into()),
        ..Default::default()
    };
    let page = svc.trial_page(&run_id, &q).expect("served").unwrap();
    let seeds: Vec<&str> = page.trials.iter().map(|t| t.seed.as_str()).collect();
    assert_eq!(seeds, ["1"], "the unreadable row matches no digest filter");
}
