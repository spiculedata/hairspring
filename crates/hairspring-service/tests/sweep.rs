//! Sweep runs through the service: the mode travels with the job, every trial is stored, the server decides for
//! itself how the run ended, and a worker's sweep data is bounded and checked.
mod common;
use common::*;
use hairspring_engine::config::{Budget, MAX_TRIALS_CAP};
use hairspring_engine::format::to_text;
use hairspring_engine::mode::RunMode;
use hairspring_engine::report::{FailureReport, RunOutcome};
use hairspring_engine::sweep::{SweepSummary, MAX_DIGEST_BYTES_IN_MEMORY, MAX_FIRST_FAILING_SEEDS};
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_service::{RunCompletion, ServiceError, StartRun};

const WORKER: &str = "local-1";

fn lock_lease(trials: u64) -> StartRun {
    StartRun {
        target: "lock-lease".into(),
        seed: Some(7),
        budget: Some(Budget {
            max_steps: 10_000_000,
            max_trials: trials,
            wall_secs: 60,
        }),
    }
}

fn rows(verdicts: &[ReportedVerdict]) -> Vec<TrialReport> {
    verdicts
        .iter()
        .enumerate()
        .map(|(i, v)| TrialReport::without_events(i as u64, 1 + i as u64, *v, "", 1))
        .collect()
}

fn failure() -> FailureReport {
    FailureReport {
        target: "lock-lease".into(),
        backend: "sim".into(),
        message: "m".into(),
        original_len: 1,
        shrunk_len: 1,
        steps: vec![],
        bundle: hairspring_core::bundle::Bundle::new(1, "t", vec![]).encode(),
    }
}

/// What a sweep worker reports when it judged `verdicts` and the lowest violation is `failure()`.
fn sweep_outcome(verdicts: &[ReportedVerdict]) -> RunOutcome {
    let log = rows(verdicts);
    let violated = verdicts.contains(&ReportedVerdict::Violation);
    hairspring_engine::sweep::assemble(hairspring_engine::sweep::SweepEnd {
        steps_executed: log.len() as u64,
        rows: log,
        failure: violated.then(failure),
        elapsed_ms: 5,
    })
}

use ReportedVerdict::{Infra, Pass, Violation};

#[test]
fn a_run_started_as_a_sweep_is_leased_as_one_and_a_default_run_as_a_search() {
    let (svc, _store, engine) = setup();
    assert_eq!(lease(&svc, &engine, WORKER).mode, RunMode::Search);
    assert_eq!(lease_sweep(&svc, &engine, WORKER, 3).mode, RunMode::Sweep);
}

#[test]
fn a_lease_from_a_server_that_sent_no_mode_reads_as_a_search() {
    let text = r#"{"job_id":"j","run_id":"r","target":"t","seed":1,"budget":null}"#;
    let job: hairspring_service::LeasedJob = serde_json::from_str(text).unwrap();
    assert_eq!(job.mode, RunMode::Search);
}

#[test]
fn a_sweep_run_stores_every_trial_and_a_default_run_stores_none_and_stops() {
    let (svc, store, engine) = setup();
    let sweep = svc
        .run_sync_with_mode(&engine, &lock_lease(6), RunMode::Sweep)
        .unwrap();
    assert_eq!(sweep.run.trials, 6);
    assert_eq!(store.count_trials(&sweep.run.id).unwrap(), 6);
    assert!(!sweep.run.stopped_at_first_failure);
    assert_eq!(
        svc.completion_for(&sweep.run).unwrap(),
        RunCompletion::Complete
    );

    let search = svc.run_sync(&engine, &lock_lease(6)).unwrap();
    assert!(search.run.stopped_at_first_failure);
    assert_eq!(store.count_trials(&search.run.id).unwrap(), 0);
}

#[test]
fn a_stored_sweep_reads_back_with_its_aggregate_and_the_report_shows_a_rate() {
    let (svc, _store, engine) = setup();
    let run = svc
        .run_sync_with_mode(&engine, &lock_lease(6), RunMode::Sweep)
        .unwrap()
        .run;
    let outcome = svc.outcome(&run.id).unwrap().unwrap();
    let summary = outcome
        .sweep
        .as_ref()
        .expect("a sweep reads back its summary");
    assert_eq!(summary.trials, 6);
    assert_eq!(summary.passes + summary.violations, 6);
    assert!(summary.violations > 0, "the fixture fails at these seeds");
    assert_eq!(
        summary.first_failing_seeds[0], 7,
        "seeds come back as stored"
    );
    let text = to_text("lock-lease", &outcome);
    assert!(text.contains("  sweep: 6 trials:"), "{text}");
    assert!(text.contains("Wilson 95% interval"), "{text}");
    assert!(!text.contains("first hit"), "{text}");
}

#[test]
fn a_search_run_reads_back_without_a_sweep_summary() {
    let (svc, _store, engine) = setup();
    let run = svc.run_sync(&engine, &lock_lease(6)).unwrap().run;
    assert!(svc.outcome(&run.id).unwrap().unwrap().sweep.is_none());
}

#[test]
fn the_server_decides_from_the_mode_whether_a_failed_run_stopped_at_its_first_failure() {
    let (svc, store, engine) = setup();
    let verdicts = [Pass, Violation, Pass];
    let sweep = lease_sweep(&svc, &engine, WORKER, 3);
    svc.ingest_trials(&sweep.job_id, WORKER, &rows(&verdicts))
        .unwrap();
    finish(&svc, &sweep, WORKER, sweep_outcome(&verdicts));
    assert!(
        !store
            .get_run(&sweep.run_id)
            .unwrap()
            .unwrap()
            .stopped_at_first_failure
    );

    let search = lease(&svc, &engine, WORKER);
    let reported = rows(&[Pass, Violation]);
    svc.ingest_trials(&search.job_id, WORKER, &reported)
        .unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(2),
        ..RunOutcome::failed(2, 2, 5, failure())
    };
    finish(&svc, &search, WORKER, outcome);
    assert!(
        store
            .get_run(&search.run_id)
            .unwrap()
            .unwrap()
            .stopped_at_first_failure
    );
}

#[test]
fn a_failed_sweep_that_ran_fewer_trials_than_its_budget_without_a_limit_is_incomplete() {
    // What a worker that predates sweeps does with a sweep job: it stops at the first failure.
    let (svc, store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 10);
    let verdicts = [Pass, Pass, Violation];
    svc.ingest_trials(&job.job_id, WORKER, &rows(&verdicts))
        .unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(3),
        ..RunOutcome::failed(3, 3, 5, failure())
    };
    finish(&svc, &job, WORKER, outcome);
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(run.ingest_incomplete, "{run:?}");
    assert_eq!(svc.completion_for(&run).unwrap(), RunCompletion::EndedEarly);
}

#[test]
fn an_incomplete_sweep_is_reported_without_a_rate_and_has_no_summary() {
    let (svc, _store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 10);
    let verdicts = [Pass, Pass, Violation];
    svc.ingest_trials(&job.job_id, WORKER, &rows(&verdicts))
        .unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(3),
        ..RunOutcome::failed(3, 3, 5, failure())
    };
    finish(&svc, &job, WORKER, outcome);
    let detail = svc.get_run(&job.run_id).unwrap().unwrap();
    assert!(
        detail.sweep.is_none(),
        "a first-hit count is not offered as a summary"
    );
    let stored = svc.outcome(&job.run_id).unwrap().unwrap();
    assert!(stored.sweep.is_none());
    let text = to_text("lock-lease", &stored);
    assert!(!text.contains("Wilson"), "{text}");
    assert!(!text.contains('%'), "no rate of any kind: {text}");
    assert!(!text.contains("sweep:"), "{text}");
}

#[test]
fn an_incomplete_sweep_that_passed_states_counts_only_not_a_rate() {
    let (svc, store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 10);
    svc.ingest_trials(&job.job_id, WORKER, &rows(&[Pass, Pass, Pass]))
        .unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(3),
        ..RunOutcome::passed(3, 3, 5)
    };
    finish(&svc, &job, WORKER, outcome);
    assert!(
        store
            .get_run(&job.run_id)
            .unwrap()
            .unwrap()
            .ingest_incomplete
    );
    let stored = svc.outcome(&job.run_id).unwrap().unwrap();
    assert!(stored.rate_withheld && stored.sweep.is_none());
    let text = to_text("lock-lease", &stored);
    assert!(text.contains("counts only, not a rate"), "{text}");
    assert!(!text.contains("hits:") && !text.contains('%'), "{text}");
    assert!(!text.contains("upper bound"), "{text}");
}

#[test]
fn an_incomplete_sweep_that_failed_does_not_say_it_stopped_at_its_first_hit() {
    let (svc, _store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 10);
    svc.ingest_trials(&job.job_id, WORKER, &rows(&[Pass, Violation]))
        .unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(2),
        ..RunOutcome::failed(2, 2, 5, failure())
    };
    finish(&svc, &job, WORKER, outcome);
    let text = to_text("lock-lease", &svc.outcome(&job.run_id).unwrap().unwrap());
    assert!(text.contains("counts only, not a rate"), "{text}");
    assert!(!text.contains("first hit"), "{text}");
}

/// A service whose digest allowance is one byte, so every trial's digest is replaced by a stand-in.
fn tiny_allowance() -> (
    hairspring_service::Service,
    std::sync::Arc<dyn hairspring_store::Store>,
    hairspring_engine::engine::Engine,
) {
    tiny_allowance_of(1)
}

/// A service whose per-run digest allowance is `bytes`. A trial row's digest is 4 bytes, so 5 holds one and replaces
/// the rest.
fn tiny_allowance_of(
    bytes: u64,
) -> (
    hairspring_service::Service,
    std::sync::Arc<dyn hairspring_store::Store>,
    hairspring_engine::engine::Engine,
) {
    let (svc, store, engine) = setup();
    let svc = svc.with_config(hairspring_service::ServiceConfig {
        digest_bytes_per_run: bytes,
        ..hairspring_service::ServiceConfig::default()
    });
    (svc, store, engine)
}

#[test]
fn a_sweep_that_ran_its_whole_budget_keeps_its_summary_when_only_its_digests_were_capped() {
    let (svc, store, engine) = tiny_allowance();
    let job = lease_sweep(&svc, &engine, WORKER, 4);
    let verdicts = [Pass, Violation, Pass, Pass];
    svc.ingest_trials(&job.job_id, WORKER, &rows(&verdicts))
        .unwrap();
    let mut outcome = sweep_outcome(&verdicts);
    outcome.trials_reported = Some(4);
    finish(&svc, &job, WORKER, outcome);
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(
        run.ingest_incomplete,
        "the capped digests make the run incomplete"
    );
    assert_eq!(
        svc.completion_for(&run).unwrap(),
        RunCompletion::DigestAllowanceUsedUp
    );
    let detail = svc.get_run(&job.run_id).unwrap().unwrap();
    let summary = detail.sweep.expect("verdicts and seeds are all it reads");
    assert_eq!((summary.trials, summary.violations), (4, 1));
    let stored = svc.outcome(&job.run_id).unwrap().unwrap();
    assert!(!stored.rate_withheld);
    assert!(to_text("lock-lease", &stored).contains("Wilson 95% interval"));
}

#[test]
fn a_short_sweep_with_capped_digests_withholds_its_summary_and_rate_on_every_surface() {
    // The label says "digest allowance used up" (any stand-in does), but the run also stopped short of its budget
    // with no limit the server can verify, so what its rows hold depends on how it stopped.
    let (svc, store, engine) = tiny_allowance_of(5);
    let job = lease_sweep(&svc, &engine, WORKER, 10);
    svc.ingest_trials(&job.job_id, WORKER, &rows(&[Pass, Pass, Pass]))
        .unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(3),
        ..RunOutcome::passed(3, 3, 5)
    };
    finish(&svc, &job, WORKER, outcome);
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert_eq!(
        svc.completion_for(&run).unwrap(),
        RunCompletion::DigestAllowanceUsedUp,
        "the label is the allowance's, as for any run with a stand-in"
    );
    let detail = svc.get_run(&job.run_id).unwrap().unwrap();
    assert!(detail.sweep.is_none() && !detail.sweep_partial);
    // REST and MCP serve this value as it is.
    let json = serde_json::to_value(&detail).unwrap();
    assert!(json.get("sweep").is_none(), "{json}");
    let stored = svc.outcome(&job.run_id).unwrap().unwrap();
    assert!(stored.rate_withheld && stored.sweep.is_none());
    let text = to_text("lock-lease", &stored);
    assert!(text.contains("counts only, not a rate"), "{text}");
    assert!(!text.contains("hits:") && !text.contains('%'), "{text}");
    assert!(!text.contains("upper bound"), "{text}");
}

#[test]
fn a_sweep_that_ended_in_an_error_is_served_with_a_partial_marker() {
    let (svc, _store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 10);
    let log = rows(&[Pass, Violation]);
    svc.ingest_trials(&job.job_id, WORKER, &log).unwrap();
    let mut outcome = hairspring_engine::sweep::assemble_error(
        "cancelled",
        hairspring_engine::sweep::SweepEnd {
            steps_executed: 2,
            rows: log,
            failure: None,
            elapsed_ms: 1,
        },
    );
    outcome.trials_reported = Some(2);
    finish(&svc, &job, WORKER, outcome);
    let detail = svc.get_run(&job.run_id).unwrap().unwrap();
    assert!(detail.sweep.is_some());
    assert!(detail.sweep_partial);
    let json = serde_json::to_value(&detail).unwrap();
    assert_eq!(json["sweep_partial"], true);
}

#[test]
fn a_sweep_that_finished_is_not_marked_partial_and_a_search_has_no_marker() {
    let (svc, _store, engine) = setup();
    let sweep = svc
        .run_sync_with_mode(&engine, &lock_lease(3), RunMode::Sweep)
        .unwrap();
    assert!(!sweep.sweep_partial);
    assert!(serde_json::to_value(&sweep)
        .unwrap()
        .get("sweep_partial")
        .is_none());
}

#[test]
fn a_failed_sweep_cannot_skip_the_incomplete_mark_on_its_workers_word_about_steps() {
    let (svc, store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 10);
    let verdicts = [Pass, Violation];
    svc.ingest_trials(&job.job_id, WORKER, &rows(&verdicts))
        .unwrap();
    // The budget's step limit is 1000; the worker says it spent them all.
    let outcome = RunOutcome {
        trials_reported: Some(2),
        ..RunOutcome::failed(1000, 2, 5, failure())
    };
    finish(&svc, &job, WORKER, outcome);
    assert!(
        store
            .get_run(&job.run_id)
            .unwrap()
            .unwrap()
            .ingest_incomplete
    );
}

#[test]
fn a_finished_sweep_serves_its_summary_with_the_run_and_a_search_does_not() {
    let (svc, _store, engine) = setup();
    let sweep = svc
        .run_sync_with_mode(&engine, &lock_lease(5), RunMode::Sweep)
        .unwrap();
    let summary = sweep
        .sweep
        .as_ref()
        .expect("a sweep's detail carries its summary");
    assert_eq!(summary.trials, 5);
    let json = serde_json::to_value(&sweep).unwrap();
    assert_eq!(json["sweep"]["trials"], 5);
    assert_eq!(json["sweep"]["first_failing_seeds"][0], "7");
    let search = svc.run_sync(&engine, &lock_lease(5)).unwrap();
    assert!(search.sweep.is_none());
    assert!(serde_json::to_value(&search)
        .unwrap()
        .get("sweep")
        .is_none());
}

#[test]
fn a_sweep_that_is_still_running_serves_no_summary() {
    let (svc, _store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 3);
    svc.ingest_trials(&job.job_id, WORKER, &rows(&[Pass, Pass]))
        .unwrap();
    assert!(svc.get_run(&job.run_id).unwrap().unwrap().sweep.is_none());
}

#[test]
fn a_sweep_row_must_carry_the_seed_its_position_names() {
    let (svc, store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 3);
    // The run starts at seed 1, so trial 1 is seed 2: a worker cannot name another failing seed.
    for forged in [1u64, 99, u64::MAX] {
        let mut log = rows(&[Pass, Violation]);
        log[1].seed = forged;
        let err = svc.ingest_trials(&job.job_id, WORKER, &log).unwrap_err();
        assert!(
            matches!(err, ServiceError::InvalidRequest(_)),
            "{forged}: {err:?}"
        );
    }
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 0);
    svc.ingest_trials(&job.job_id, WORKER, &rows(&[Pass, Violation]))
        .unwrap();
}

#[test]
fn a_sweep_seed_rule_wraps_at_the_top_of_the_seed_space() {
    let (svc, _store, engine) = setup();
    let req = StartRun {
        seed: Some(u64::MAX),
        ..lock_lease(3)
    };
    svc.start_run_with_mode(&engine, &req, RunMode::Sweep)
        .unwrap();
    let job = svc.lease(WORKER).unwrap().unwrap();
    let mut log = rows(&[Pass, Pass]);
    log[0].seed = u64::MAX;
    log[1].seed = 0;
    svc.ingest_trials(&job.job_id, WORKER, &log).unwrap();
}

#[test]
fn a_sweep_that_ran_every_trial_of_its_budget_is_complete() {
    let (svc, store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 4);
    let verdicts = [Pass, Violation, Pass, Pass];
    svc.ingest_trials(&job.job_id, WORKER, &rows(&verdicts))
        .unwrap();
    let mut outcome = sweep_outcome(&verdicts);
    outcome.trials_reported = Some(4);
    finish(&svc, &job, WORKER, outcome);
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(!run.ingest_incomplete);
    assert_eq!(svc.completion_for(&run).unwrap(), RunCompletion::Complete);
}

#[test]
fn a_workers_own_sweep_summary_is_never_stored_or_trusted() {
    let (svc, _store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 2);
    svc.ingest_trials(&job.job_id, WORKER, &rows(&[Pass, Pass]))
        .unwrap();
    // Consistent with itself and with its trial count, but not with the rows the server holds.
    let claim = SweepSummary {
        trials: 2,
        violations: 2,
        first_failing_seeds: vec![1, 2],
        ..SweepSummary::default()
    };
    let outcome = RunOutcome {
        trials_reported: Some(2),
        ..RunOutcome::passed(2, 2, 5).with_sweep(claim)
    };
    finish(&svc, &job, WORKER, outcome);
    let stored = svc.outcome(&job.run_id).unwrap().unwrap();
    let summary = stored.sweep.unwrap();
    assert_eq!((summary.violations, summary.passes), (0, 2));
}

#[test]
fn an_inconsistent_or_oversize_sweep_summary_is_refused() {
    let (svc, _store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 5);
    let bad = [
        // The verdict counts do not add up to the trials.
        SweepSummary {
            trials: 5,
            passes: 1,
            ..SweepSummary::default()
        },
        // It does not match the outcome's own trial count.
        SweepSummary {
            trials: 4,
            passes: 4,
            ..SweepSummary::default()
        },
        // It names more failing seeds than a summary may.
        SweepSummary {
            trials: 5,
            violations: 5,
            first_failing_seeds: (0..=MAX_FIRST_FAILING_SEEDS as u64).collect(),
            ..SweepSummary::default()
        },
    ];
    for summary in bad {
        let outcome = RunOutcome::passed(5, 5, 5).with_sweep(summary);
        let err = svc.complete(&job.job_id, WORKER, &outcome).unwrap_err();
        assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err:?}");
    }
}

#[test]
fn a_trial_record_past_the_runs_trial_budget_is_refused() {
    let (svc, store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 3);
    let log = rows(&[Pass, Pass, Pass, Pass]);
    let err = svc.ingest_trials(&job.job_id, WORKER, &log).unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err:?}");
    assert_eq!(
        store.count_trials(&job.run_id).unwrap(),
        0,
        "all or nothing"
    );
    svc.ingest_trials(&job.job_id, WORKER, &log[..3]).unwrap();
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 3);
}

#[test]
fn a_trial_record_numbered_at_the_trial_cap_is_refused_even_for_a_budget_at_the_cap() {
    let (svc, _store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, MAX_TRIALS_CAP);
    let mut log = rows(&[Pass]);
    log[0].seq = MAX_TRIALS_CAP;
    assert!(svc.ingest_trials(&job.job_id, WORKER, &log).is_err());
}

#[test]
fn every_verdict_a_sweep_can_produce_is_accepted_and_stored_with_its_verdict() {
    let (svc, store, engine) = setup();
    let job = lease_sweep(&svc, &engine, WORKER, 5);
    let verdicts = [
        Pass,
        Violation,
        ReportedVerdict::Inconclusive,
        ReportedVerdict::Unusable,
        Infra,
    ];
    svc.ingest_trials(&job.job_id, WORKER, &rows(&verdicts))
        .unwrap();
    let stored: Vec<_> = store
        .list_trials(&job.run_id, None, 10)
        .unwrap()
        .iter()
        .map(|t| t.verdict.as_str())
        .collect();
    assert_eq!(
        stored,
        vec!["pass", "violation", "inconclusive", "unusable", "infra"]
    );
}

#[test]
fn the_in_memory_digest_bound_of_a_sweep_is_the_servers_allowance() {
    assert_eq!(
        MAX_DIGEST_BYTES_IN_MEMORY,
        hairspring_store::MAX_DIGEST_BYTES_PER_RUN
    );
}
