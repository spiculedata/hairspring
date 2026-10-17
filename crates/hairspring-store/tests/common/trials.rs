//! Conformance scenarios for schema v2: run statistics, the run catalog, trial rows and worker trust.
#![allow(dead_code)]
use hairspring_store::*;

fn new_run(s: &dyn Store, target: &str, now: i64) -> RunRecord {
    s.create_run(
        &NewRun {
            target: target.into(),
            backend: "hermit".into(),
            seed: 7,
            config_json: "{}".into(),
        },
        now,
    )
    .unwrap()
}

/// A run with a leased job, ready for ingest.
fn leased(s: &dyn Store, worker: &str) -> (RunRecord, Job) {
    let r = new_run(s, "t", 1);
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    let job = s.lease_job(worker, 600, 10).unwrap().unwrap();
    s.mark_running(&r.id, 10).unwrap();
    (r, job)
}

fn row(idx: u32, id: &str, decided: bool) -> CatalogRowRecord {
    CatalogRowRecord {
        idx,
        prop_id: id.to_string(),
        kind: "always".to_string(),
        must_hit: true,
        msg: format!("message for {id}"),
        loc_file: Some("src/a.c".to_string()),
        loc_line: Some(12),
        loc_func: Some("fn_a".to_string()),
        decided,
    }
}

fn trial(seq: u64) -> NewTrial {
    NewTrial {
        seq,
        seed: 100 + seq,
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
        behaviour_hash: Some("abc123".to_string()),
        schedule_fp: Some(format!("fp{}", seq % 3)),
        syscalls: 1000 + seq,
        work_ops: 5,
        elapsed_ms: 12,
        max_idx: Some(0),
        digest: vec![1, 0, 0, 0],
        detail: None,
        output: None,
    }
}

fn batch(catalog: Vec<CatalogRowRecord>, trials: Vec<NewTrial>) -> TrialBatch {
    TrialBatch {
        catalog,
        trials,
        kept_bytes_cap: MAX_KEPT_BYTES_PER_RUN,
    }
}

fn applied(inserted: u32, duplicates: u32) -> IngestOutcome {
    IngestOutcome::Applied {
        inserted,
        duplicates,
    }
}

pub fn run_all_trials(make: &dyn Fn() -> std::sync::Arc<dyn Store>) {
    new_runs_have_zero_statistics_and_the_search_mode(&*make());
    finish_job_records_statistics_and_property_rows(&*make());
    ingest_stores_trials_and_catalog_and_reads_them_back(&*make());
    ingest_keeps_seeds_above_two_to_the_fifty_three(&*make());
    ingest_is_idempotent_on_sequence_number(&*make());
    ingest_requires_the_lease_and_an_open_run(&*make());
    ingest_refuses_digests_that_point_past_the_catalog(&*make());
    ingest_enforces_the_catalog_rules(&*make());
    ingest_enforces_batch_and_field_limits(&*make());
    a_new_attempt_replaces_the_previous_attempts_rows(&*make());
    kept_detail_goes_to_blobs_under_a_per_run_cap(&*make());
    worker_trust_defaults_to_untrusted_and_is_reported_per_run(&*make());
    trial_digests_page_in_sequence_order(&*make());
    strip_rows_carry_the_verdict_and_flags_in_sequence_order(&*make());
    hostile_text_in_trials_and_catalog_is_stored_verbatim(&*make());
    runs_do_not_see_each_others_rows(&*make());
    finishing_a_later_attempt_removes_the_earlier_attempts_rows(&*make());
    worker_secret_hashes_are_stored_apart_from_the_worker_record(&*make());
    a_worker_and_its_secret_hash_are_registered_in_one_step(&*make());
    a_new_attempt_collects_the_blobs_only_its_discarded_rows_used(&*make());
}

fn new_runs_have_zero_statistics_and_the_search_mode(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    assert_eq!(
        (
            r.inconclusive,
            r.distinct_schedules,
            r.work,
            r.catalog_size,
            r.ingest_incomplete
        ),
        (0, 0, 0, 0, false)
    );
    assert_eq!(r.mode, "search");
    assert!(!r.stopped_at_first_failure);
    assert!(r.property_summary_json.is_none() && r.build_id.is_none());
}

fn property_row(idx: u32, status: &str) -> RunPropertyRow {
    RunPropertyRow {
        idx,
        status: status.to_string(),
        trials: 10,
        trials_rated: 9,
        trials_unreported: 1,
        trials_degraded: 0,
        trials_perturbing: 0,
        trials_conflicted: 0,
        trials_inconclusive_with_failures: 0,
        trials_claimable: 9,
        trials_hit: 8,
        trials_true: 7,
        trials_false: 1,
        evals: 100,
        evals_true: 99,
        evals_false: 1,
        rate_basis: Some("trials_false".to_string()),
        rate_k: Some(1),
        rate_n: Some(9),
        first_failure_seed: Some(u64::MAX - 5),
    }
}

fn finish_with_stats(s: &dyn Store, job: &Job, worker: &str, stats: RunStats) -> FinishOutcome {
    let result = JobResult {
        state: RunState::Passed,
        steps: 10,
        trials: 10,
        error: None,
        failure: None,
        event_payload_json: "{}".to_string(),
        stats,
    };
    s.finish_job(&job.id, worker, &result, 50).unwrap()
}

fn finish_job_records_statistics_and_property_rows(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    s.ingest_trials(
        &job.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], vec![trial(0)]),
        20,
    )
    .unwrap();
    let stats = RunStats {
        inconclusive: 2,
        distinct_schedules: 3,
        work: 4000,
        stopped_at_first_failure: true,
        ingest_incomplete: true,
        property_summary_json: Some("{\"trials\":10}".to_string()),
        properties: vec![property_row(0, "violation")],
    };
    assert_eq!(
        finish_with_stats(s, &job, "w1", stats),
        FinishOutcome::Applied
    );
    let g = s.get_run(&r.id).unwrap().unwrap();
    assert_eq!(
        (
            g.inconclusive,
            g.distinct_schedules,
            g.work,
            g.catalog_size,
            g.stopped_at_first_failure,
            g.ingest_incomplete
        ),
        (2, 3, 4000, 1, true, true)
    );
    assert_eq!(g.property_summary_json.as_deref(), Some("{\"trials\":10}"));
    assert_eq!(
        s.list_run_properties(&r.id).unwrap(),
        vec![property_row(0, "violation")]
    );
}

fn ingest_stores_trials_and_catalog_and_reads_them_back(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let mut t1 = trial(1);
    t1.verdict = TrialVerdict::Violation;
    t1.reason = "always 'a.b' was false".to_string();
    t1.exit_code = None;
    t1.signal = Some(11);
    let outcome = s
        .ingest_trials(
            &job.id,
            "w1",
            true,
            &batch(vec![row(0, "a.b", true)], vec![trial(0), t1.clone()]),
            20,
        )
        .unwrap();
    assert_eq!(outcome, applied(2, 0));
    assert_eq!(s.count_trials(&r.id).unwrap(), 2);
    assert_eq!(s.get_run(&r.id).unwrap().unwrap().catalog_size, 1);
    assert_eq!(s.get_catalog(&r.id).unwrap(), vec![row(0, "a.b", true)]);
    let back = s.get_trial(&r.id, 1).unwrap().unwrap();
    assert_eq!(
        (back.seed, back.verdict, back.reason.as_str(), back.signal),
        (
            101,
            TrialVerdict::Violation,
            "always 'a.b' was false",
            Some(11)
        )
    );
    assert_eq!(back.exit_code, None);
    assert_eq!(
        (back.worker_id.as_str(), back.worker_trusted, back.attempt),
        ("w1", true, 1)
    );
    assert_eq!(back.digest, t1.digest);
    assert_eq!(back.kept, KeptState::NotKept);
    assert!(s.get_trial(&r.id, 9).unwrap().is_none());
    let listed = s.list_trials(&r.id, None, 10).unwrap();
    assert_eq!(listed.iter().map(|t| t.seq).collect::<Vec<_>>(), vec![0, 1]);
}

fn ingest_keeps_seeds_above_two_to_the_fifty_three(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let mut t = trial(0);
    t.seed = u64::MAX - 1;
    s.ingest_trials(
        &job.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], vec![t]),
        20,
    )
    .unwrap();
    assert_eq!(s.get_trial(&r.id, 0).unwrap().unwrap().seed, u64::MAX - 1);
}

fn ingest_is_idempotent_on_sequence_number(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let b = batch(vec![row(0, "a.b", true)], vec![trial(0), trial(1)]);
    assert_eq!(
        s.ingest_trials(&job.id, "w1", true, &b, 20).unwrap(),
        applied(2, 0)
    );
    assert_eq!(
        s.ingest_trials(&job.id, "w1", true, &b, 21).unwrap(),
        applied(0, 2)
    );
    assert_eq!(s.count_trials(&r.id).unwrap(), 2);
    assert_eq!(s.get_catalog(&r.id).unwrap().len(), 1);
    let mut changed = trial(1);
    changed.seed += 1;
    let again = batch(vec![], vec![changed]);
    assert!(matches!(
        s.ingest_trials(&job.id, "w1", true, &again, 22).unwrap(),
        IngestOutcome::Rejected(_)
    ));
    let mut other_digest = trial(1);
    other_digest.digest = vec![1, 0, 0, 1];
    assert!(matches!(
        s.ingest_trials(&job.id, "w1", true, &batch(vec![], vec![other_digest]), 23)
            .unwrap(),
        IngestOutcome::Rejected(_)
    ));
    assert_eq!(
        s.get_trial(&r.id, 1).unwrap().unwrap().seed,
        101,
        "first write wins"
    );
}

fn ingest_requires_the_lease_and_an_open_run(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let b = batch(vec![row(0, "a.b", true)], vec![trial(0)]);
    assert_eq!(
        s.ingest_trials(&job.id, "intruder", true, &b, 20).unwrap(),
        IngestOutcome::NotLeased
    );
    assert_eq!(
        s.ingest_trials("job_nope", "w1", true, &b, 20).unwrap(),
        IngestOutcome::NotLeased
    );
    assert_eq!(
        s.count_trials(&r.id).unwrap(),
        0,
        "nothing written for a non-owner"
    );
    s.cancel_run(&r.id, 25).unwrap();
    assert_eq!(
        s.ingest_trials(&job.id, "w1", true, &b, 30).unwrap(),
        IngestOutcome::RunNotOpen
    );
    assert_eq!(s.count_trials(&r.id).unwrap(), 0);
}

fn ingest_refuses_digests_that_point_past_the_catalog(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let mut t = trial(0);
    t.max_idx = Some(1);
    let out = s
        .ingest_trials(
            &job.id,
            "w1",
            true,
            &batch(vec![row(0, "a.b", true)], vec![t]),
            20,
        )
        .unwrap();
    assert!(matches!(out, IngestOutcome::Rejected(_)), "{out:?}");
    assert_eq!(s.count_trials(&r.id).unwrap(), 0);
    assert!(
        s.get_catalog(&r.id).unwrap().is_empty(),
        "the whole batch is refused"
    );
    let mut none = trial(0);
    none.max_idx = None;
    assert_eq!(
        s.ingest_trials(&job.id, "w1", true, &batch(vec![], vec![none]), 21)
            .unwrap(),
        applied(1, 0),
        "a digest with no ids needs no catalog"
    );
}

fn ingest_enforces_the_catalog_rules(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let no_trials = |c| batch(c, vec![]);
    let go = |c| {
        s.ingest_trials(&job.id, "w1", true, &no_trials(c), 20)
            .unwrap()
    };
    assert!(
        matches!(go(vec![row(1, "a.b", true)]), IngestOutcome::Rejected(_)),
        "a gap"
    );
    assert_eq!(
        go(vec![row(0, "a.b", false), row(1, "a.c", true)]),
        applied(0, 0)
    );
    assert!(
        matches!(go(vec![row(2, "a.b", true)]), IngestOutcome::Rejected(_)),
        "a duplicate id"
    );
    let mut decide = row(0, "a.b", true);
    decide.kind = "sometimes".to_string();
    decide.must_hit = true;
    assert_eq!(
        go(vec![decide.clone()]),
        applied(0, 0),
        "an undecided row may be decided"
    );
    assert_eq!(s.get_catalog(&r.id).unwrap()[0].kind, "sometimes");
    assert_eq!(
        go(vec![decide.clone()]),
        applied(0, 0),
        "a repeated row is a no-op"
    );
    let mut different = decide;
    different.kind = "reachable".to_string();
    assert!(
        matches!(go(vec![different]), IngestOutcome::Rejected(_)),
        "a decided row cannot change"
    );
    let mut renamed = row(1, "a.other", true);
    renamed.decided = true;
    assert!(matches!(go(vec![renamed]), IngestOutcome::Rejected(_)));
    assert_eq!(s.get_run(&r.id).unwrap().unwrap().catalog_size, 2);
}

fn ingest_enforces_batch_and_field_limits(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let go = |b: TrialBatch| s.ingest_trials(&job.id, "w1", true, &b, 20).unwrap();
    let too_many: Vec<_> = (0..=MAX_TRIALS_PER_BATCH as u64).map(trial).collect();
    assert!(matches!(
        go(batch(vec![row(0, "a.b", true)], too_many)),
        IngestOutcome::Rejected(_)
    ));
    let mut big_digest = trial(0);
    big_digest.digest = vec![0; MAX_DIGEST_BLOB_BYTES + 1];
    assert!(matches!(
        go(batch(vec![row(0, "a.b", true)], vec![big_digest])),
        IngestOutcome::Rejected(_)
    ));
    let mut long_reason = trial(0);
    long_reason.reason = "r".repeat(MAX_TRIAL_REASON_CHARS + 1);
    assert!(matches!(
        go(batch(vec![row(0, "a.b", true)], vec![long_reason])),
        IngestOutcome::Rejected(_)
    ));
    let mut huge_detail = trial(0);
    huge_detail.detail = Some(vec![b'x'; MAX_KEPT_DETAIL_BYTES + 1]);
    assert!(matches!(
        go(batch(vec![row(0, "a.b", true)], vec![huge_detail])),
        IngestOutcome::Rejected(_)
    ));
    let mut huge_output = trial(0);
    huge_output.output = Some(vec![b'x'; MAX_KEPT_OUTPUT_BYTES + 1]);
    assert!(matches!(
        go(batch(vec![row(0, "a.b", true)], vec![huge_output])),
        IngestOutcome::Rejected(_)
    ));
    let rows: Vec<_> = (0..=MAX_CATALOG_ROWS as u32)
        .map(|i| row(i, &format!("id.{i}"), true))
        .collect();
    assert!(matches!(
        go(batch(rows, vec![])),
        IngestOutcome::Rejected(_)
    ));
    let mut bad_verdict_seq = trial(0);
    bad_verdict_seq.seq = i64::MAX as u64 + 1;
    assert!(matches!(
        go(batch(vec![row(0, "a.b", true)], vec![bad_verdict_seq])),
        IngestOutcome::Rejected(_)
    ));
    assert_eq!(s.count_trials(&r.id).unwrap(), 0);
    assert!(s.get_catalog(&r.id).unwrap().is_empty());
}

fn a_new_attempt_replaces_the_previous_attempts_rows(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    let j = s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    s.lease_job("w1", 10, 1_000).unwrap().unwrap();
    s.ingest_trials(
        &j.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], vec![trial(0), trial(1)]),
        1_100,
    )
    .unwrap();
    assert_eq!(s.count_trials(&r.id).unwrap(), 2);
    // The lease expires and the job is leased again, by another worker.
    let second = s.lease_job("w2", 10, 100_000).unwrap().unwrap();
    assert_eq!((second.id.as_str(), second.attempts), (j.id.as_str(), 2));
    assert_eq!(
        s.ingest_trials(&j.id, "w1", true, &batch(vec![], vec![trial(2)]), 100_001)
            .unwrap(),
        IngestOutcome::NotLeased,
        "the first worker lost the lease"
    );
    let out = s
        .ingest_trials(
            &j.id,
            "w2",
            false,
            &batch(vec![row(0, "x.y", true)], vec![trial(0)]),
            100_002,
        )
        .unwrap();
    assert_eq!(out, applied(1, 0));
    assert_eq!(
        s.count_trials(&r.id).unwrap(),
        1,
        "the first attempt's rows are gone"
    );
    assert_eq!(s.get_catalog(&r.id).unwrap()[0].prop_id, "x.y");
    let t = s.get_trial(&r.id, 0).unwrap().unwrap();
    assert_eq!(
        (t.worker_id.as_str(), t.worker_trusted, t.attempt),
        ("w2", false, 2)
    );
}

fn kept_detail_goes_to_blobs_under_a_per_run_cap(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let mut kept = trial(0);
    kept.detail = Some(b"{\"v\":1}".to_vec());
    kept.output = Some(b"{\"stdout\":\"hi\"}".to_vec());
    s.ingest_trials(
        &job.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], vec![kept]),
        20,
    )
    .unwrap();
    let t = s.get_trial(&r.id, 0).unwrap().unwrap();
    assert_eq!(t.kept, KeptState::Kept);
    let detail = s
        .get_blob(t.detail_ref.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(detail, b"{\"v\":1}");
    let output = s
        .get_blob(t.output_ref.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(output, b"{\"stdout\":\"hi\"}");

    // With a small allowance, the trial that does not fit is marked, not silently dropped.
    let small = |t: NewTrial| TrialBatch {
        catalog: vec![],
        trials: vec![t],
        kept_bytes_cap: (MAX_KEPT_DETAIL_BYTES * 2) as u64,
    };
    let big = |seq| NewTrial {
        detail: Some(vec![b'a'; MAX_KEPT_DETAIL_BYTES]),
        ..trial(seq)
    };
    for seq in 1..=2 {
        s.ingest_trials(&job.id, "w1", true, &small(big(seq)), 21)
            .unwrap();
    }
    assert_eq!(
        s.get_trial(&r.id, 1).unwrap().unwrap().kept,
        KeptState::Kept
    );
    assert_eq!(
        s.get_trial(&r.id, 2).unwrap().unwrap().kept,
        KeptState::DroppedCap,
        "the first kept trial and one full-size detail already fill the allowance"
    );
    let capped = s.get_trial(&r.id, 2).unwrap().unwrap();
    assert!(capped.detail_ref.is_none() && capped.output_ref.is_none());
    assert_eq!(
        capped.verdict,
        TrialVerdict::Pass,
        "the row itself is still stored"
    );
}

fn worker_trust_defaults_to_untrusted_and_is_reported_per_run(s: &dyn Store) {
    let w = s.register_worker("remote", "{}", 1).unwrap();
    assert!(!w.trusted);
    assert_eq!(s.worker_trusted(&w.id).unwrap(), Some(false));
    assert_eq!(s.worker_trusted("wrk_unknown").unwrap(), None);
    assert!(s.set_worker_trusted("wrk_unknown", true).is_err());
    s.set_worker_trusted(&w.id, true).unwrap();
    assert!(s.list_workers().unwrap()[0].trusted);

    let (r, job) = leased(s, "w1");
    s.ingest_trials(
        &job.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], vec![trial(0), trial(1)]),
        20,
    )
    .unwrap();
    let contributors = s.run_contributors(&r.id).unwrap();
    assert_eq!(
        contributors,
        vec![Contributor {
            worker_id: "w1".to_string(),
            trusted: true,
            trials: 2
        }]
    );
    let (r2, job2) = leased(s, "w2");
    s.ingest_trials(
        &job2.id,
        "w2",
        false,
        &batch(vec![row(0, "a.b", true)], vec![trial(0)]),
        20,
    )
    .unwrap();
    assert!(!s.run_contributors(&r2.id).unwrap()[0].trusted);
}

fn trial_digests_page_in_sequence_order(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let trials: Vec<_> = (0..7).map(trial).collect();
    s.ingest_trials(
        &job.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], trials),
        20,
    )
    .unwrap();
    let first = s.list_trial_digests(&r.id, None, 3).unwrap();
    assert_eq!(
        first.iter().map(|t| t.seq).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    let next = s.list_trial_digests(&r.id, Some(2), 3).unwrap();
    assert_eq!(
        next.iter().map(|t| t.seq).collect::<Vec<_>>(),
        vec![3, 4, 5]
    );
    assert_eq!(next[0].seed, 103);
    let last = s.list_trial_digests(&r.id, Some(5), 3).unwrap();
    assert_eq!(last.len(), 1);
    assert!(s.list_trial_digests(&r.id, Some(6), 3).unwrap().is_empty());
}

fn strip_rows_carry_the_verdict_and_flags_in_sequence_order(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let mut degraded = trial(1);
    degraded.degraded = true;
    let mut unreported = trial(2);
    unreported.report_missing = true;
    let mut unrated = trial(3);
    unrated.rated = false;
    let mut failed = trial(4);
    failed.verdict = TrialVerdict::Violation;
    let trials = vec![trial(0), degraded, unreported, unrated, failed];
    s.ingest_trials(
        &job.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], trials),
        20,
    )
    .unwrap();
    let rows = s.list_trial_strip_rows(&r.id, None, 10).unwrap();
    let got: Vec<_> = rows
        .iter()
        .map(|t| {
            (
                t.seq,
                t.verdict,
                t.rated,
                t.degraded,
                t.report_missing,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (0, TrialVerdict::Pass, true, false, false),
            (1, TrialVerdict::Pass, true, true, false),
            (2, TrialVerdict::Pass, true, false, true),
            (3, TrialVerdict::Pass, false, false, false),
            (4, TrialVerdict::Violation, true, false, false),
        ]
    );
    let after = s.list_trial_strip_rows(&r.id, Some(2), 2).unwrap();
    assert_eq!(after.iter().map(|t| t.seq).collect::<Vec<_>>(), vec![3, 4]);
    assert!(s.list_trial_strip_rows(&r.id, Some(4), 2).unwrap().is_empty());
}

fn hostile_text_in_trials_and_catalog_is_stored_verbatim(s: &dyn Store) {
    let (r, job) = leased(s, "w1");
    let nasty = "x'); DROP TABLE trials; --";
    let mut c = row(0, "a.b", true);
    c.msg = nasty.to_string();
    c.loc_file = Some(nasty.to_string());
    let mut t = trial(0);
    t.reason = nasty.to_string();
    t.behaviour_hash = Some(nasty.to_string());
    s.ingest_trials(&job.id, "w1", true, &batch(vec![c.clone()], vec![t]), 20)
        .unwrap();
    assert_eq!(s.get_catalog(&r.id).unwrap(), vec![c]);
    assert_eq!(s.get_trial(&r.id, 0).unwrap().unwrap().reason, nasty);
    assert_eq!(
        s.count_trials(&r.id).unwrap(),
        1,
        "the table is still there"
    );
}

fn runs_do_not_see_each_others_rows(s: &dyn Store) {
    let (a, ja) = leased(s, "wa");
    let (b, _jb) = leased(s, "wb");
    s.ingest_trials(
        &ja.id,
        "wa",
        true,
        &batch(vec![row(0, "a.b", true)], vec![trial(0)]),
        20,
    )
    .unwrap();
    assert_eq!(s.count_trials(&a.id).unwrap(), 1);
    assert_eq!(s.count_trials(&b.id).unwrap(), 0);
    assert!(s.get_catalog(&b.id).unwrap().is_empty());
    assert!(s.get_trial(&b.id, 0).unwrap().is_none());
    assert!(s.list_run_properties(&b.id).unwrap().is_empty());
}

fn finishing_a_later_attempt_removes_the_earlier_attempts_rows(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    let j = s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    s.lease_job("w1", 10, 1_000).unwrap().unwrap();
    s.ingest_trials(
        &j.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], vec![trial(0)]),
        1_100,
    )
    .unwrap();
    // The lease expires; the next attempt sends nothing and finishes.
    s.lease_job("w2", 10, 100_000).unwrap().unwrap();
    let result = JobResult {
        state: RunState::Error,
        steps: 0,
        trials: 0,
        error: Some("could not start".to_string()),
        failure: None,
        event_payload_json: "{}".to_string(),
        stats: RunStats::default(),
    };
    assert_eq!(
        s.finish_job(&j.id, "w2", &result, 100_100).unwrap(),
        FinishOutcome::Applied
    );
    assert_eq!(s.count_trials(&r.id).unwrap(), 0);
    assert!(s.get_catalog(&r.id).unwrap().is_empty());
    assert_eq!(s.get_run(&r.id).unwrap().unwrap().catalog_size, 0);
}

fn worker_secret_hashes_are_stored_apart_from_the_worker_record(s: &dyn Store) {
    let w = s.register_worker("remote", "{}", 1).unwrap();
    assert_eq!(s.worker_secret_hash(&w.id).unwrap(), None);
    s.set_worker_secret_hash(&w.id, "abc123").unwrap();
    assert_eq!(
        s.worker_secret_hash(&w.id).unwrap().as_deref(),
        Some("abc123")
    );
    assert_eq!(s.worker_secret_hash("wrk_unknown").unwrap(), None);
    assert!(s.set_worker_secret_hash("wrk_unknown", "x").is_err());
    let listed = format!("{:?}", s.list_workers().unwrap());
    assert!(
        !listed.contains("abc123"),
        "the hash is not part of the worker record"
    );
}

fn a_worker_and_its_secret_hash_are_registered_in_one_step(s: &dyn Store) {
    let w = s
        .register_worker_with_secret("remote", "{}", "feedface", 7)
        .unwrap();
    assert!(!w.trusted);
    assert_eq!(w.registered_at, 7);
    assert_eq!(
        s.worker_secret_hash(&w.id).unwrap().as_deref(),
        Some("feedface")
    );
    assert_eq!(s.list_workers().unwrap().len(), 1);
}

fn a_new_attempt_collects_the_blobs_only_its_discarded_rows_used(s: &dyn Store) {
    // A second run keeps a trial whose detail has the same bytes as one of the first run's.
    let (other, other_job) = leased(s, "wx");
    let shared = b"{\"shared\":1}".to_vec();
    let mut keep = trial(0);
    keep.detail = Some(shared.clone());
    s.ingest_trials(
        &other_job.id,
        "wx",
        true,
        &batch(vec![row(0, "a.b", true)], vec![keep]),
        5,
    )
    .unwrap();

    let r = new_run(s, "t", 1);
    let j = s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    s.lease_job("w1", 10, 1_000).unwrap().unwrap();
    let mut mine = trial(0);
    mine.detail = Some(shared.clone());
    mine.output = Some(b"{\"stdout\":\"only mine\"}".to_vec());
    s.ingest_trials(
        &j.id,
        "w1",
        true,
        &batch(vec![row(0, "a.b", true)], vec![mine]),
        1_100,
    )
    .unwrap();
    let mine_row = s.get_trial(&r.id, 0).unwrap().unwrap();
    let (detail_ref, output_ref) = (mine_row.detail_ref.unwrap(), mine_row.output_ref.unwrap());
    assert!(s.get_blob(&output_ref).unwrap().is_some());

    s.lease_job("w2", 10, 100_000).unwrap().unwrap();
    s.ingest_trials(
        &j.id,
        "w2",
        true,
        &batch(vec![row(0, "x.y", true)], vec![trial(0)]),
        100_001,
    )
    .unwrap();
    assert!(
        s.get_blob(&output_ref).unwrap().is_none(),
        "an orphaned blob is collected"
    );
    assert!(
        s.get_blob(&detail_ref).unwrap().is_some(),
        "a blob another run still uses stays"
    );
    assert_eq!(s.count_trials(&other.id).unwrap(), 1);
}
