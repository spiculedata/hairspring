//! Conformance scenarios for the explorer's storage needs: the per-run digest allowance, the coverage series kept
//! with a finished run, and the query that names the worker behind each failing seed.
#![allow(dead_code)]
use super::trials::{batch, leased, row, trial};
use hairspring_store::*;

pub fn run_all_explorer(make: &dyn Fn() -> std::sync::Arc<dyn Store>) {
    reporters_name_the_worker_behind_each_seed(&*make());
    reporters_of_an_empty_or_unknown_request_is_empty(&*make());
    reporters_are_not_shared_between_runs(&*make());
    reporters_cover_requests_larger_than_one_statement(&*make());
    reporters_refuse_an_unbounded_request(&*make());
    a_digest_over_the_allowance_is_replaced_and_marked(&*make());
    replaced_digests_count_against_the_allowance(&*make());
    a_digest_over_the_allowance_without_a_replacement_rejects_the_batch(&*make());
    a_resent_batch_with_a_replaced_digest_is_a_duplicate(&*make());
    a_replacement_must_describe_the_same_trial(&*make());
    a_new_attempt_starts_the_digest_allowance_again(&*make());
    the_coverage_series_is_stored_with_the_finished_run(&*make());
    a_cancelled_run_keeps_no_coverage_series(&*make());
}

fn ingest(s: &dyn Store, job: &Job, worker: &str, b: &TrialBatch) -> IngestOutcome {
    s.ingest_trials(&job.id, worker, true, b, 20).unwrap()
}

fn applied(inserted: u32, duplicates: u32) -> IngestOutcome {
    IngestOutcome::Applied {
        inserted,
        duplicates,
    }
}

/// A trial whose digest is `len` bytes long and which offers `replacement` when the allowance is used up.
fn sized(seq: u64, len: usize) -> NewTrial {
    let mut t = trial(seq);
    t.digest = vec![1; len];
    t.on_digest_cap = Some(Box::new(replacement(seq)));
    t
}

/// What the service offers instead of a digest that does not fit: a two-byte stand-in, not rated.
fn replacement(seq: u64) -> NewTrial {
    NewTrial {
        digest: vec![7, 7],
        rated: false,
        claimable: false,
        max_idx: None,
        reason: "digest not stored: the run's digest allowance is used up".to_string(),
        on_digest_cap: None,
        ..trial(seq)
    }
}

fn capped_batch(trials: Vec<NewTrial>, cap: u64) -> TrialBatch {
    TrialBatch {
        digest_bytes_cap: cap,
        ..batch(vec![row(0, "a.b", true)], trials)
    }
}

/// One batch of trials numbered from 0 with digests of the given lengths, under a digest allowance of `cap` bytes.
pub fn batch_of_digests(lens: &[usize], cap: u64) -> TrialBatch {
    let trials = lens
        .iter()
        .enumerate()
        .map(|(i, len)| sized(i as u64, *len))
        .collect();
    capped_batch(trials, cap)
}

fn reporters_name_the_worker_behind_each_seed(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let mut again = trial(3);
    again.seed = trial(1).seed;
    let mut huge = trial(4);
    huge.seed = u64::MAX - 1;
    ingest(
        s,
        &job,
        "w1",
        &batch(
            vec![row(0, "a.b", true)],
            vec![trial(0), trial(1), trial(2), again, huge],
        ),
    );
    let got = s
        .reporters_of(
            &r.id,
            &[trial(1).seed, trial(2).seed, u64::MAX - 1, 999_999],
        )
        .unwrap();
    assert_eq!(got.len(), 3, "an unknown seed maps to nothing: {got:?}");
    assert_eq!(got[&trial(1).seed], "w1");
    assert_eq!(got[&trial(2).seed], "w1");
    assert_eq!(
        got[&(u64::MAX - 1)],
        "w1",
        "seeds above 2^63 keep their value"
    );
}

fn reporters_of_an_empty_or_unknown_request_is_empty(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    ingest(
        s,
        &job,
        "w1",
        &batch(vec![row(0, "a.b", true)], vec![trial(0)]),
    );
    assert!(s.reporters_of(&r.id, &[]).unwrap().is_empty());
    assert!(s
        .reporters_of("run_does_not_exist", &[trial(0).seed])
        .unwrap()
        .is_empty());
}

fn reporters_are_not_shared_between_runs(s: &dyn Store) {
    let (r1, j1) = leased(s, "w1");
    let (r2, j2) = leased(s, "w2");
    for (job, worker) in [(&j1, "w1"), (&j2, "w2")] {
        ingest(
            s,
            job,
            worker,
            &batch(vec![row(0, "a.b", true)], vec![trial(0)]),
        );
    }
    let seed = trial(0).seed;
    assert_eq!(s.reporters_of(&r1.id, &[seed]).unwrap()[&seed], "w1");
    assert_eq!(s.reporters_of(&r2.id, &[seed]).unwrap()[&seed], "w2");
}

fn reporters_cover_requests_larger_than_one_statement(s: &dyn Store) {
    const TRIALS: u64 = 600;
    let (r, job) = leased(s, "w1");
    for start in (0..TRIALS).step_by(MAX_TRIALS_PER_BATCH) {
        let end = (start + MAX_TRIALS_PER_BATCH as u64).min(TRIALS);
        let catalog = if start == 0 {
            vec![row(0, "a.b", true)]
        } else {
            vec![]
        };
        ingest(
            s,
            &job,
            "w1",
            &batch(catalog, (start..end).map(trial).collect()),
        );
    }
    let mut wanted: Vec<u64> = (0..TRIALS).map(|q| trial(q).seed).collect();
    wanted.extend(5_000_000..5_000_700);
    let got = s.reporters_of(&r.id, &wanted).unwrap();
    assert_eq!(got.len() as u64, TRIALS);
}

fn reporters_refuse_an_unbounded_request(s: &dyn Store) {
    let (r, _job) = leased(s, "w1");
    let seeds: Vec<u64> = (0..=MAX_REPORTER_SEEDS as u64).collect();
    assert!(matches!(
        s.reporters_of(&r.id, &seeds),
        Err(StoreError::LimitExceeded { .. })
    ));
}

fn a_digest_over_the_allowance_is_replaced_and_marked(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    // Two 4-byte digests fit in 10 bytes; the third would make 12.
    let out = ingest(
        s,
        &job,
        "w1",
        &capped_batch(vec![sized(0, 4), sized(1, 4), sized(2, 4)], 10),
    );
    assert_eq!(out, applied(3, 0), "the run is not refused");
    let kept = s.get_trial(&r.id, 1).unwrap().unwrap();
    assert_eq!(kept.digest, vec![1; 4]);
    let capped = s.get_trial(&r.id, 2).unwrap().unwrap();
    assert_eq!(
        capped.digest,
        vec![7, 7],
        "the row holds the offered stand-in"
    );
    assert!(!capped.rated && !capped.claimable);
    assert!(capped.reason.contains("allowance"));
    assert_eq!(
        (capped.seq, capped.seed),
        (2, trial(2).seed),
        "the trial itself is still recorded"
    );
    assert_eq!(
        s.run_digest_use(&r.id).unwrap(),
        DigestUse {
            bytes: 10,
            capped_trials: 1
        },
        "two 4-byte digests and the 2-byte stand-in"
    );
}

fn replaced_digests_count_against_the_allowance(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    // 4 + 4 stored, the third is replaced by 2 bytes (10 used), so a fourth 4-byte digest no longer fits.
    ingest(
        s,
        &job,
        "w1",
        &capped_batch(vec![sized(0, 4), sized(1, 4), sized(2, 4)], 10),
    );
    ingest(s, &job, "w1", &capped_batch(vec![sized(3, 4)], 10));
    assert_eq!(s.get_trial(&r.id, 3).unwrap().unwrap().digest, vec![7, 7]);
    assert_eq!(s.run_digest_use(&r.id).unwrap().capped_trials, 2);
    // Another run has its own allowance.
    let (r2, job2) = leased(s, "w1");
    ingest(s, &job2, "w1", &capped_batch(vec![sized(0, 4)], 10));
    assert_eq!(s.get_trial(&r2.id, 0).unwrap().unwrap().digest, vec![1; 4]);
    assert_eq!(
        s.run_digest_use(&r2.id).unwrap(),
        DigestUse {
            bytes: 4,
            capped_trials: 0
        }
    );
}

fn a_digest_over_the_allowance_without_a_replacement_rejects_the_batch(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let mut bare = trial(1);
    bare.digest = vec![1; 8];
    let out = ingest(s, &job, "w1", &capped_batch(vec![sized(0, 4), bare], 10));
    assert!(matches!(out, IngestOutcome::Rejected(_)), "{out:?}");
    assert_eq!(
        s.count_trials(&r.id).unwrap(),
        0,
        "a rejected batch writes nothing"
    );
}

fn a_resent_batch_with_a_replaced_digest_is_a_duplicate(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let b = capped_batch(vec![sized(0, 4), sized(1, 8)], 10);
    assert_eq!(ingest(s, &job, "w1", &b), applied(2, 0));
    assert_eq!(
        ingest(s, &job, "w1", &b),
        applied(0, 2),
        "a lost reply must not turn a stored batch into a conflict"
    );
    assert_eq!(s.count_trials(&r.id).unwrap(), 2);
    assert_eq!(
        s.run_digest_use(&r.id).unwrap(),
        DigestUse {
            bytes: 6,
            capped_trials: 1
        },
        "a duplicate uses no allowance"
    );
    // Different content under the same sequence number is still refused.
    let mut changed = sized(1, 8);
    changed.seed += 1;
    changed.on_digest_cap = Some(Box::new(NewTrial {
        seed: changed.seed,
        ..replacement(1)
    }));
    assert!(matches!(
        ingest(s, &job, "w1", &capped_batch(vec![changed], 10)),
        IngestOutcome::Rejected(_)
    ));
}

fn a_replacement_must_describe_the_same_trial(s: &dyn Store) {
    let (_r, job) = leased(s, "w1");
    let mut other_seq = sized(0, 4);
    other_seq.on_digest_cap = Some(Box::new(replacement(9)));
    let mut other_seed = sized(1, 4);
    other_seed.on_digest_cap = Some(Box::new(NewTrial {
        seed: 12345,
        ..replacement(1)
    }));
    let mut nested = sized(2, 4);
    nested.on_digest_cap = Some(Box::new(NewTrial {
        on_digest_cap: Some(Box::new(replacement(2))),
        ..replacement(2)
    }));
    let mut oversized = sized(3, 4);
    oversized.on_digest_cap = Some(Box::new(NewTrial {
        digest: vec![0; MAX_DIGEST_BLOB_BYTES + 1],
        ..replacement(3)
    }));
    for bad in [other_seq, other_seed, nested, oversized] {
        let out = ingest(s, &job, "w1", &capped_batch(vec![bad], 1_000));
        assert!(matches!(out, IngestOutcome::Rejected(_)), "{out:?}");
    }
}

fn a_new_attempt_starts_the_digest_allowance_again(s: &dyn Store) {
    let r = s
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 7,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let j = s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    s.lease_job("w1", 10, 1_000).unwrap().unwrap();
    s.ingest_trials(
        &j.id,
        "w1",
        true,
        &capped_batch(vec![sized(0, 8), sized(1, 8)], 10),
        1_100,
    )
    .unwrap();
    assert_eq!(s.run_digest_use(&r.id).unwrap().capped_trials, 1);
    s.lease_job("w2", 10, 100_000).unwrap().unwrap();
    let out = s
        .ingest_trials(
            &j.id,
            "w2",
            true,
            &capped_batch(vec![sized(0, 8)], 10),
            100_002,
        )
        .unwrap();
    assert_eq!(out, applied(1, 0));
    assert_eq!(
        s.run_digest_use(&r.id).unwrap(),
        DigestUse {
            bytes: 8,
            capped_trials: 0
        },
        "the earlier attempt's rows and its use of the allowance are gone"
    );
}

fn finish(s: &dyn Store, job: &Job, coverage: Option<&str>) -> FinishOutcome {
    let result = JobResult {
        state: RunState::Passed,
        steps: 1,
        trials: 1,
        error: None,
        failure: None,
        event_payload_json: "{}".to_string(),
        stats: RunStats {
            coverage_json: coverage.map(str::to_owned),
            ..RunStats::default()
        },
    };
    s.finish_job(&job.id, "w1", &result, 50).unwrap()
}

fn the_coverage_series_is_stored_with_the_finished_run(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    assert_eq!(s.run_coverage_json(&r.id).unwrap(), None);
    ingest(
        s,
        &job,
        "w1",
        &batch(vec![row(0, "a.b", true)], vec![trial(0)]),
    );
    assert_eq!(
        s.run_coverage_json(&r.id).unwrap(),
        None,
        "not before it ends"
    );
    let series = "{\"points\":[]}";
    assert_eq!(finish(s, &job, Some(series)), FinishOutcome::Applied);
    assert_eq!(s.run_coverage_json(&r.id).unwrap().as_deref(), Some(series));
    assert_eq!(s.run_coverage_json("run_does_not_exist").unwrap(), None);
    let (r2, job2) = leased(s, "w1");
    assert_eq!(finish(s, &job2, None), FinishOutcome::Applied);
    assert_eq!(s.run_coverage_json(&r2.id).unwrap(), None);
}

fn a_cancelled_run_keeps_no_coverage_series(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    assert!(s.cancel_run(&r.id, 30).unwrap());
    assert_eq!(
        finish(s, &job, Some("{\"points\":[]}")),
        FinishOutcome::RunAlreadyFinal
    );
    assert_eq!(s.run_coverage_json(&r.id).unwrap(), None);
}
