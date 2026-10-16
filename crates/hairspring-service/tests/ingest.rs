//! Per-trial storage through the service: what a worker may send, what the server folds from it itself, and what it
//! refuses.
use hairspring_engine::config::{Config, MAX_TRIALS_CAP};
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_props::{
    AssertKind, DigestEntry, PropertyStatus, RunCatalog, RunProps, TrialDigest, TrialEvents,
    TrialProps, Witness,
};
use hairspring_service::{LeasedJob, Service, ServiceError, StartRun};
use hairspring_store::{RunState, SqliteStore, Store};
use std::sync::Arc;

fn setup() -> (Service, Arc<dyn Store>, Engine) {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    (Service::new(store.clone()), store, engine)
}

fn lease(svc: &Service, engine: &Engine, worker: &str) -> LeasedJob {
    svc.start_run(
        engine,
        &StartRun {
            target: "lock-lease".into(),
            seed: Some(1),
            budget: None,
        },
    )
    .unwrap();
    svc.lease(worker).unwrap().expect("a job")
}

fn props(id: &str, kind: AssertKind, trues: u64, falses: u64) -> TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let must_hit = kind.must_hit();
    let witness = |on: bool| {
        if on {
            r#"{"thread":"w-0","ord":5,"vals":[1,2,3,4]}"#
        } else {
            "null"
        }
    };
    let text = [
        ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
        ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"{}","must_hit":{must_hit},"msg":"m"}}"#, kind.as_str())),
        ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":{},"first_true":{}}}"#, trues + falses, witness(falses > 0), witness(trues > 0))),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
    ]
    .concat();
    TrialEvents::from_output(text.as_bytes()).fold()
}

/// Builds trial reports the way a worker does.
struct Recorder {
    catalog: RunCatalog,
    next: u64,
}

impl Recorder {
    fn new() -> Recorder {
        Recorder {
            catalog: RunCatalog::new(),
            next: 0,
        }
    }

    fn trial(
        &mut self,
        seed: u64,
        p: &TrialProps,
        verdict: ReportedVerdict,
        inconclusive: bool,
    ) -> TrialReport {
        let rec = std::mem::take(&mut self.catalog).record(p, inconclusive, 16384);
        self.catalog = rec.catalog;
        let seq = self.next;
        self.next += 1;
        TrialReport {
            seq,
            seed,
            verdict,
            reason: String::new(),
            exit_code: Some(0),
            signal: None,
            behaviour_hash: Some("00aa11bb22cc33dd".to_string()),
            schedule_fp: Some(format!("{:016x}", seed % 3)),
            syscalls: 100,
            work_ops: 4,
            elapsed_ms: 3,
            catalog: rec.updates,
            digest: rec.digest.encode(),
            detail: None,
            output: None,
        }
    }

    fn pass(&mut self, seed: u64, p: &TrialProps) -> TrialReport {
        self.trial(seed, p, ReportedVerdict::Pass, false)
    }
}

fn finish(svc: &Service, job: &LeasedJob, worker: &str, outcome: RunOutcome) {
    svc.complete(&job.job_id, worker, &outcome).unwrap();
}

fn passed(trials: u64, reported: Option<u64>) -> RunOutcome {
    RunOutcome {
        trials_reported: reported,
        ..RunOutcome::passed(trials, trials, 5)
    }
}

fn local() -> String {
    Service::local_worker_id()
}

#[test]
fn trials_are_stored_and_the_server_folds_the_statistics_itself() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let ok = props("a.id", AssertKind::Always, 3, 0);
    let bad = props("a.id", AssertKind::Always, 1, 2);
    let reports = vec![
        rec.pass(10, &ok),
        rec.trial(11, &bad, ReportedVerdict::Violation, false),
        rec.pass(12, &ok),
    ];
    let receipt = svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    assert_eq!((receipt.inserted, receipt.duplicates), (3, 0));
    finish(&svc, &job, &local(), passed(3, Some(3)));

    let mut direct = RunProps::new();
    for (seed, p) in [(10, &ok), (11, &bad), (12, &ok)] {
        direct = direct.add(seed, p);
    }
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert_eq!(run.catalog_size, 1);
    assert!(!run.ingest_incomplete);
    let stored = svc.property_stats(&job.run_id).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].status, PropertyStatus::Violation);
    assert_eq!(
        (
            stored[0].trials,
            stored[0].trials_false,
            stored[0].trials_rated
        ),
        (3, 1, 3)
    );
    let want = &direct.clone().finish()[0];
    assert_eq!(
        stored[0].rate.map(|r| (r.k, r.n)),
        want.rate.map(|r| (r.k, r.n))
    );
    assert_eq!(
        stored[0].first_failure.as_ref().map(|f| f.seed),
        want.first_failure.as_ref().map(|f| f.seed)
    );
    let summary: hairspring_props::RunSummary =
        serde_json::from_str(run.property_summary_json.as_deref().unwrap()).unwrap();
    assert_eq!(summary, direct.summary());
}

#[test]
fn a_workers_own_statistics_are_never_stored() {
    let (svc, _, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let report = rec.pass(10, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job.job_id, &local(), &[report]).unwrap();
    // The outcome claims the property failed in a million trials.
    let mut lie = RunProps::new();
    for seed in 0..5 {
        lie = lie.add(seed, &props("a.id", AssertKind::Always, 0, 9));
    }
    let outcome = passed(1, Some(1)).with_properties(lie.clone().finish(), lie.summary());
    finish(&svc, &job, &local(), outcome);
    let stored = svc.property_stats(&job.run_id).unwrap();
    assert_eq!((stored[0].trials, stored[0].trials_false), (1, 0));
    assert_eq!(stored[0].status, PropertyStatus::Pass);
}

#[test]
fn a_seed_above_two_to_the_fifty_three_round_trips_through_the_store() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let report = rec.pass(u64::MAX - 2, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job.job_id, &local(), &[report]).unwrap();
    assert_eq!(
        store.get_trial(&job.run_id, 0).unwrap().unwrap().seed,
        u64::MAX - 2
    );
}

#[test]
fn sending_a_batch_again_changes_nothing() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let batch = vec![
        rec.pass(1, &props("a.id", AssertKind::Always, 3, 0)),
        rec.pass(2, &props("a.id", AssertKind::Always, 3, 0)),
    ];
    svc.ingest_trials(&job.job_id, &local(), &batch).unwrap();
    let again = svc.ingest_trials(&job.job_id, &local(), &batch).unwrap();
    assert_eq!((again.inserted, again.duplicates), (0, 2));
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 2);
    let mut changed = batch[1].clone();
    changed.seed = 99;
    assert!(matches!(
        svc.ingest_trials(&job.job_id, &local(), &[changed]),
        Err(ServiceError::InvalidRequest(_))
    ));
}

#[test]
fn every_trial_row_names_its_worker_and_whether_it_was_trusted() {
    let (svc, store, engine) = setup();
    let remote = svc.register_worker("remote", "{}").unwrap();
    let job = lease(&svc, &engine, &remote.id);
    let mut rec = Recorder::new();
    let report = rec.pass(1, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job.job_id, &remote.id, &[report])
        .unwrap();
    let row = store.get_trial(&job.run_id, 0).unwrap().unwrap();
    assert_eq!(
        (row.worker_id.as_str(), row.worker_trusted),
        (remote.id.as_str(), false)
    );

    let job2 = lease(&svc, &engine, &local());
    let report = Recorder::new().pass(1, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job2.job_id, &local(), &[report])
        .unwrap();
    assert!(
        store
            .get_trial(&job2.run_id, 0)
            .unwrap()
            .unwrap()
            .worker_trusted
    );

    svc.set_worker_trust(&remote.id, true).unwrap();
    let job3 = lease(&svc, &engine, &remote.id);
    let report = Recorder::new().pass(1, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job3.job_id, &remote.id, &[report])
        .unwrap();
    assert!(
        store
            .get_trial(&job3.run_id, 0)
            .unwrap()
            .unwrap()
            .worker_trusted
    );
}

#[test]
fn a_run_with_one_untrusted_trial_is_flagged_unverified_and_the_claim_gate_refuses_it() {
    use hairspring_service::claims::{claim_gate, Confirmation, FailureEvidence};
    let (svc, _, engine) = setup();
    let remote = svc.register_worker("remote", "{}").unwrap();
    let job = lease(&svc, &engine, &remote.id);
    let mut rec = Recorder::new();
    let report = rec.pass(1, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job.job_id, &remote.id, &[report])
        .unwrap();
    let trust = svc.run_trust(&job.run_id).unwrap().unwrap();
    assert!(!trust.all_trusted);
    assert_eq!(trust.workers.len(), 1);
    assert!(!claim_gate(&trust.workers, &[], 1).allowed);

    // Trusted rows with no failure to rest a claim on are allowed; a failure needs its confirmation.
    let job2 = lease(&svc, &engine, &local());
    let report = Recorder::new().pass(1, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job2.job_id, &local(), &[report])
        .unwrap();
    let trusted = svc.run_trust(&job2.run_id).unwrap().unwrap();
    assert!(trusted.all_trusted);
    assert!(claim_gate(&trusted.workers, &[], 1).allowed);
    let unconfirmed = FailureEvidence {
        reporter: local(),
        confirmations: vec![],
    };
    assert!(!claim_gate(&trusted.workers, &[unconfirmed], 1).allowed);
    let on_untrusted = FailureEvidence {
        reporter: local(),
        confirmations: vec![Confirmation {
            replayer: "x".into(),
            replayer_trusted: false,
            reproduced: true,
        }],
    };
    assert!(!claim_gate(&trusted.workers, &[on_untrusted], 1).allowed);
}

#[test]
fn a_failure_confirmed_only_by_the_reporting_worker_does_not_open_the_gate_when_another_trusted_worker_exists(
) {
    use hairspring_service::claims::{claim_gate, Confirmation, FailureEvidence};
    use hairspring_store::Contributor;
    let contributors = vec![Contributor {
        worker_id: "w1".into(),
        trusted: true,
        trials: 10,
    }];
    let self_confirmed = |replayer: &str| FailureEvidence {
        reporter: "w1".into(),
        confirmations: vec![Confirmation {
            replayer: replayer.into(),
            replayer_trusted: true,
            reproduced: true,
        }],
    };
    assert!(!claim_gate(&contributors, &[self_confirmed("w1")], 2).allowed);
    assert!(claim_gate(&contributors, &[self_confirmed("w2")], 2).allowed);
    assert!(
        claim_gate(&contributors, &[self_confirmed("w1")], 1).allowed,
        "one trusted worker is all there is"
    );
    let not_reproduced = FailureEvidence {
        reporter: "w1".into(),
        confirmations: vec![Confirmation {
            replayer: "w2".into(),
            replayer_trusted: true,
            reproduced: false,
        }],
    };
    assert!(!claim_gate(&contributors, &[not_reproduced], 2).allowed);
    assert!(!claim_gate(&[], &[], 1).allowed, "nothing was run");
}

#[test]
fn a_trial_count_that_does_not_match_marks_the_run_incomplete_and_withholds_unmet() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    // A must-hit property that is never satisfied: UNMET in a complete run.
    let never = props("s.id", AssertKind::Sometimes, 0, 4);
    let reports = vec![rec.pass(1, &never), rec.pass(2, &never)];
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    // The worker says it ran three trials; the server holds two.
    finish(&svc, &job, &local(), passed(3, Some(3)));
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(run.ingest_incomplete);
    let stats = svc.property_stats(&job.run_id).unwrap();
    assert_eq!(
        stats[0].status,
        PropertyStatus::Unrated,
        "no UNMET claim from an incomplete run"
    );

    let job2 = lease(&svc, &engine, &local());
    let mut rec2 = Recorder::new();
    let reports = vec![rec2.pass(1, &never), rec2.pass(2, &never)];
    svc.ingest_trials(&job2.job_id, &local(), &reports).unwrap();
    finish(&svc, &job2, &local(), passed(2, Some(2)));
    assert!(
        !store
            .get_run(&job2.run_id)
            .unwrap()
            .unwrap()
            .ingest_incomplete
    );
    assert_eq!(
        svc.property_stats(&job2.run_id).unwrap()[0].status,
        PropertyStatus::Unmet
    );
}

#[test]
fn a_run_that_stopped_at_its_first_failure_makes_no_unmet_claim() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let never = props("s.id", AssertKind::Sometimes, 0, 4);
    let bad = props("a.id", AssertKind::Always, 0, 1);
    let reports = vec![
        rec.pass(1, &never),
        rec.trial(2, &bad, ReportedVerdict::Violation, false),
    ];
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    let failed = RunOutcome {
        trials_reported: Some(2),
        ..RunOutcome::failed(
            2,
            2,
            5,
            hairspring_engine::report::FailureReport {
                target: "lock-lease".into(),
                backend: "sim".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                steps: vec![],
                bundle: hairspring_core::bundle::Bundle::new(1, "t", vec![]).encode(),
            },
        )
    };
    finish(&svc, &job, &local(), failed);
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(run.stopped_at_first_failure);
    let stats = svc.property_stats(&job.run_id).unwrap();
    let s = stats.iter().find(|s| s.id == "s.id").unwrap();
    assert_eq!(s.status, PropertyStatus::Unrated);
}

#[test]
fn a_run_reported_without_a_count_but_holding_rows_is_incomplete() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let report = Recorder::new().pass(1, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job.job_id, &local(), &[report]).unwrap();
    finish(&svc, &job, &local(), passed(1, None));
    assert!(
        store
            .get_run(&job.run_id)
            .unwrap()
            .unwrap()
            .ingest_incomplete
    );
}

#[test]
fn a_run_with_no_trial_rows_and_no_count_is_not_incomplete() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    finish(&svc, &job, &local(), passed(7, None));
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(
        !run.ingest_incomplete,
        "a backend without properties sends no rows"
    );
    assert_eq!((run.catalog_size, run.state), (0, RunState::Passed));
    assert!(svc.property_stats(&job.run_id).unwrap().is_empty());
}

#[test]
fn the_local_path_ingests_the_outcomes_trial_log_before_completing() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let reports: Vec<_> = (0..600)
        .map(|i| rec.pass(i, &props("a.id", AssertKind::Always, 2, 0)))
        .collect();
    let outcome = RunOutcome::passed(600, 600, 5).with_trial_log(reports);
    finish(&svc, &job, &local(), outcome);
    assert_eq!(
        store.count_trials(&job.run_id).unwrap(),
        600,
        "more than one batch"
    );
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(!run.ingest_incomplete);
    assert_eq!(svc.property_stats(&job.run_id).unwrap()[0].trials, 600);
}

#[test]
fn a_stored_run_rebuilds_its_inconclusive_and_schedule_counts() {
    let (svc, _, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let outcome = RunOutcome::passed(10, 10, 5)
        .with_inconclusive(2)
        .with_schedule_stats(4, 900);
    finish(&svc, &job, &local(), outcome);
    let back = svc.outcome(&job.run_id).unwrap().unwrap();
    assert_eq!(
        (back.inconclusive, back.distinct_schedules, back.work),
        (2, 4, 900)
    );
}

#[test]
fn a_stored_run_rebuilds_its_property_statistics_and_summary() {
    let (svc, _, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let reports = vec![
        rec.pass(1, &props("a.id", AssertKind::Always, 3, 0)),
        rec.trial(
            2,
            &props("a.id", AssertKind::Always, 1, 1),
            ReportedVerdict::Violation,
            false,
        ),
    ];
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    finish(&svc, &job, &local(), passed(2, Some(2)));
    let back = svc.outcome(&job.run_id).unwrap().unwrap();
    assert_eq!(back.properties.len(), 1);
    assert_eq!(back.properties[0].status, PropertyStatus::Violation);
    assert_eq!(back.property_run.as_ref().map(|s| s.trials), Some(2));
    let rate = back.properties[0].rate.unwrap();
    assert_eq!((rate.k, rate.n), (1, 2));
    assert!(rate.lower < 0.5 && rate.upper > 0.5);
}

// ---------------------------------------------------------------------------------------------------------------
// Hostile input. Every case sends a batch a compromised worker might send and expects it refused, with nothing stored.

fn refused(svc: &Service, store: &Arc<dyn Store>, job: &LeasedJob, batch: &[TrialReport]) {
    let err = svc.ingest_trials(&job.job_id, &local(), batch).unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err}");
    assert_eq!(
        store.count_trials(&job.run_id).unwrap(),
        0,
        "nothing stored"
    );
    assert!(
        store.get_catalog(&job.run_id).unwrap().is_empty(),
        "no catalog rows either"
    );
}

fn one(seed: u64) -> TrialReport {
    Recorder::new().pass(seed, &props("a.id", AssertKind::Always, 3, 0))
}

#[test]
fn hostile_digests_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    let mut garbage = good.clone();
    garbage.digest = vec![9, 9, 9];
    refused(&svc, &store, &job, &[garbage]);
    let mut forged_version = good.clone();
    forged_version.digest[0] = 7;
    refused(&svc, &store, &job, &[forged_version]);
    let mut empty = good.clone();
    empty.digest = Vec::new();
    refused(&svc, &store, &job, &[empty]);
    // An index the catalog does not hold.
    let mut past = good.clone();
    let mut d = TrialDigest::decode(&past.digest).unwrap();
    d.entries[0].idx = 5;
    past.digest = d.encode();
    refused(&svc, &store, &job, &[past]);
    // Counts that do not add up.
    let mut bad_counts = good.clone();
    let mut d = TrialDigest::decode(&bad_counts.digest).unwrap();
    d.entries[0].hits += 1;
    bad_counts.digest = d.encode();
    refused(&svc, &store, &job, &[bad_counts]);
    // More witnesses than allowed.
    let mut many = good.clone();
    let entries: Vec<DigestEntry> = (0..65)
        .map(|i| DigestEntry {
            idx: if i == 0 { 0 } else { i },
            kind: AssertKind::Always,
            catalogued: true,
            conflicted: false,
            hits: 1,
            trues: 0,
            falses: 1,
            witness: Some(Witness {
                proc: "p1".into(),
                thread: "w".into(),
                ord: 1,
                vals: [0; 4],
            }),
            decl: None,
        })
        .collect();
    many.digest = TrialDigest {
        entries,
        ..TrialDigest::default()
    }
    .encode();
    refused(&svc, &store, &job, &[many]);
    // An excluded trial carries no entries.
    let mut excluded = good.clone();
    let mut d = TrialDigest::decode(&excluded.digest).unwrap();
    d.flags.excluded = true;
    excluded.digest = d.encode();
    refused(&svc, &store, &job, &[excluded]);
}

#[test]
fn hostile_catalog_updates_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    let mutate = |f: &dyn Fn(&mut TrialReport)| {
        let mut r = good.clone();
        f(&mut r);
        r
    };
    let oversize = mutate(&|r| r.catalog[0].row.id = "x".repeat(129));
    let control = mutate(&|r| r.catalog[0].row.id = "a\u{1}b".to_string());
    let bidi = mutate(&|r| r.catalog[0].row.msg = "evil \u{202e} text".to_string());
    let long_msg = mutate(&|r| r.catalog[0].row.msg = "m".repeat(513));
    let gap = mutate(&|r| r.catalog[0].idx = 3);
    let nul = mutate(&|r| r.catalog[0].row.msg = "a\0b".to_string());
    for r in [oversize, control, bidi, long_msg, gap, nul] {
        refused(&svc, &store, &job, &[r]);
    }
    let mut twice = good.clone();
    let mut again = twice.catalog[0].clone();
    again.idx = 1;
    twice.catalog.push(again);
    refused(&svc, &store, &job, &[twice]);
}

#[test]
fn hostile_row_fields_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    let cases: Vec<(&str, Box<dyn Fn(&mut TrialReport)>)> = vec![
        (
            "a sequence number past the budget",
            Box::new(|r| r.seq = MAX_TRIALS_CAP),
        ),
        ("a huge sequence number", Box::new(|r| r.seq = u64::MAX)),
        ("a long reason", Box::new(|r| r.reason = "r".repeat(501))),
        (
            "a reason with a newline",
            Box::new(|r| r.reason = "a\nb".to_string()),
        ),
        (
            "a reason with an escape",
            Box::new(|r| r.reason = "a\u{1b}[31m".to_string()),
        ),
        (
            "a bidi reason",
            Box::new(|r| r.reason = "a\u{202e}b".to_string()),
        ),
        (
            "a hash with punctuation",
            Box::new(|r| r.behaviour_hash = Some("x'; --".to_string())),
        ),
        (
            "a long fingerprint",
            Box::new(|r| r.schedule_fp = Some("a".repeat(65))),
        ),
        (
            "counters past the signed range",
            Box::new(|r| r.syscalls = u64::MAX),
        ),
        (
            "an elapsed time past the signed range",
            Box::new(|r| r.elapsed_ms = u64::MAX),
        ),
        (
            "a pass that timed out",
            Box::new(|r| {
                let mut d = TrialDigest::decode(&r.digest).unwrap();
                d.flags.inconclusive = true;
                r.digest = d.encode();
            }),
        ),
        (
            "a pass whose property failed",
            Box::new(|r| {
                let mut d = TrialDigest::decode(&r.digest).unwrap();
                d.entries[0].falses = 1;
                d.entries[0].hits = d.entries[0].trues + 1;
                r.digest = d.encode();
            }),
        ),
    ];
    for (what, f) in cases {
        let mut r = good.clone();
        f(&mut r);
        let err = svc.ingest_trials(&job.job_id, &local(), &[r]).unwrap_err();
        assert!(
            matches!(err, ServiceError::InvalidRequest(_)),
            "{what}: {err}"
        );
        assert_eq!(store.count_trials(&job.run_id).unwrap(), 0, "{what}");
    }
}

#[test]
fn hostile_detail_and_output_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    let detail = |s: String| {
        let mut r = good.clone();
        r.detail = Some(s);
        r
    };
    let output = |s: String| {
        let mut r = good.clone();
        r.output = Some(s);
        r
    };
    let cases = vec![
        detail("not json".to_string()),
        detail("{\"v\":9}".to_string()),
        detail(" ".repeat(256 * 1024 + 1)),
        output("not json".to_string()),
        output("[1,2]".to_string()),
        output("{\"stdout\":1}".to_string()),
        output("{\"stdout\":\"a\",\"stderr\":\"b\",\"extra\":\"c\"}".to_string()),
        output(format!(
            "{{\"stdout\":\"{}\",\"stderr\":\"\"}}",
            "a".repeat(128 * 1024)
        )),
    ];
    for r in cases {
        refused(&svc, &store, &job, &[r]);
    }
    let mut ok = good.clone();
    ok.output = Some("{\"stdout\":\"fine\",\"stderr\":\"\"}".to_string());
    svc.ingest_trials(&job.job_id, &local(), &[ok]).unwrap();
}

#[test]
fn an_oversized_batch_is_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let batch: Vec<_> = (0..257)
        .map(|i| rec.pass(i, &props("a.id", AssertKind::Always, 1, 0)))
        .collect();
    refused(&svc, &store, &job, &batch);
}

#[test]
fn only_the_lease_holder_may_send_trials_and_only_while_the_run_is_open() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let stranger = svc.register_worker("stranger", "{}").unwrap();
    let err = svc
        .ingest_trials(&job.job_id, &stranger.id, &[one(1)])
        .unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)));
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 0);
    svc.cancel_run(&job.run_id).unwrap();
    let err = svc
        .ingest_trials(&job.job_id, &local(), &[one(1)])
        .unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)));
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 0);
}

#[test]
fn a_new_attempt_starts_the_runs_trials_over() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let first: Vec<_> = (0..3)
        .map(|i| rec.pass(i, &props("old.id", AssertKind::Always, 1, 0)))
        .collect();
    svc.ingest_trials(&job.job_id, &local(), &first).unwrap();
    svc.fail(&job.job_id, &local(), "interrupted").unwrap();
    let again = svc.lease(&local()).unwrap().expect("requeued");
    assert_eq!(again.job_id, job.job_id);
    let report = Recorder::new().pass(50, &props("new.id", AssertKind::Always, 1, 0));
    svc.ingest_trials(&again.job_id, &local(), &[report])
        .unwrap();
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 1);
    assert_eq!(store.get_catalog(&job.run_id).unwrap()[0].prop_id, "new.id");
}
