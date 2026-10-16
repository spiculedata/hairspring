//! A run that ended on its time or step limit before reaching its trial budget is not a run with missing data: the
//! rows it holds are all it ran. A run that was ended early by its caller still is.
mod common;

use common::*;
use hairspring_engine::config::Budget;
use hairspring_engine::engine::Engine;
use hairspring_engine::report::RunOutcome;
use hairspring_props::{AssertKind, PropertyStatus};
use hairspring_service::{LeasedJob, RunCompletion, Service, StartRun};
use hairspring_store::Store;
use std::sync::Arc;

const BUDGET_TRIALS: u64 = 10;
const BUDGET_STEPS: u64 = 1000;
const WALL_SECS: u64 = 30;
const MS_PER_SEC: i64 = 1000;

/// Leases a run whose first lease began `started_ago_ms` ago.
fn lease_started(
    svc: &Service,
    store: &Arc<dyn Store>,
    engine: &Engine,
    started_ago_ms: i64,
) -> LeasedJob {
    let run = svc
        .start_run(
            engine,
            &StartRun {
                target: "lock-lease".into(),
                seed: Some(1),
                budget: Some(Budget {
                    max_steps: BUDGET_STEPS,
                    max_trials: BUDGET_TRIALS,
                    wall_secs: WALL_SECS,
                }),
            },
        )
        .unwrap();
    store
        .mark_running(&run.id, hairspring_store::now_ms() - started_ago_ms)
        .unwrap();
    svc.lease(&local()).unwrap().expect("a job")
}

/// Three trials of a must-hit property that is never satisfied, ingested for `job`.
fn ingest_unmet(svc: &Service, job: &LeasedJob) {
    let never = props("s.id", AssertKind::Sometimes, 0, 4);
    let mut rec = Recorder::new();
    let reports = vec![
        rec.pass(1, &never),
        rec.pass(2, &never),
        rec.pass(3, &never),
    ];
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
}

fn outcome(steps: u64) -> RunOutcome {
    RunOutcome {
        trials_reported: Some(3),
        ..RunOutcome::passed(steps, 3, 5)
    }
}

#[test]
fn a_run_that_ran_out_of_time_keeps_its_unmet_result_and_is_labelled_time_limited() {
    let (svc, store, engine) = setup();
    let job = lease_started(&svc, &store, &engine, (WALL_SECS as i64 + 5) * MS_PER_SEC);
    ingest_unmet(&svc, &job);
    finish(&svc, &job, &local(), outcome(3));

    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(!run.ingest_incomplete, "no data is missing");
    let stats = svc.property_stats(&job.run_id).unwrap();
    assert_eq!(stats[0].status, PropertyStatus::Unmet);
    let detail = svc.get_run(&job.run_id).unwrap().unwrap();
    let completion = detail.properties.unwrap().completion;
    assert_eq!(
        completion,
        RunCompletion::TimeOrStepLimited {
            trials_run: 3,
            trials_budget: BUDGET_TRIALS
        }
    );
    assert_eq!(completion.label(), "time- or step-limited (3 of 10 trials)");
}

#[test]
fn a_run_that_reached_its_step_limit_is_labelled_limited_too() {
    let (svc, store, engine) = setup();
    let job = lease_started(&svc, &store, &engine, 0);
    ingest_unmet(&svc, &job);
    finish(&svc, &job, &local(), outcome(BUDGET_STEPS));

    assert!(
        !store
            .get_run(&job.run_id)
            .unwrap()
            .unwrap()
            .ingest_incomplete
    );
    assert_eq!(
        svc.property_stats(&job.run_id).unwrap()[0].status,
        PropertyStatus::Unmet
    );
}

#[test]
fn a_run_ended_early_by_its_caller_is_still_incomplete_and_makes_no_unmet_claim() {
    let (svc, store, engine) = setup();
    // Started a moment ago, far fewer steps than the limit, three of ten trials: the caller stopped it.
    let job = lease_started(&svc, &store, &engine, MS_PER_SEC);
    ingest_unmet(&svc, &job);
    finish(&svc, &job, &local(), outcome(3));

    assert!(
        store
            .get_run(&job.run_id)
            .unwrap()
            .unwrap()
            .ingest_incomplete
    );
    assert_eq!(
        svc.property_stats(&job.run_id).unwrap()[0].status,
        PropertyStatus::Unrated
    );
    let completion = svc
        .get_run(&job.run_id)
        .unwrap()
        .unwrap()
        .properties
        .unwrap()
        .completion;
    assert_eq!(completion, RunCompletion::EndedEarly);
}

#[test]
fn a_run_that_used_its_whole_trial_budget_is_complete() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    ingest_unmet(&svc, &job);
    finish(&svc, &job, &local(), outcome(3));
    let completion = svc
        .get_run(&job.run_id)
        .unwrap()
        .unwrap()
        .properties
        .unwrap()
        .completion;
    assert_eq!(completion, RunCompletion::Complete);
}
