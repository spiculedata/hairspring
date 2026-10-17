//! The coverage series of a finished run is served from memory; an open run's is always current.
mod common;

use common::*;
use hairspring_props::AssertKind;

#[test]
fn an_open_runs_coverage_follows_new_trials() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 10);
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 1, 0);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(1, &p)])
        .unwrap();
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 1);

    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(2, &p)])
        .unwrap();
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 2);
}

#[test]
fn a_finished_runs_coverage_is_the_same_on_every_read() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 2);
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 1, 0);
    svc.ingest_trials(
        &job.job_id,
        &local(),
        &[rec.pass(1, &p), rec.pass(2, &p)],
    )
    .unwrap();
    finish(&svc, &job, &local(), passed(2, Some(2)));

    let first = svc.coverage(&job.run_id).unwrap().unwrap();
    let second = svc.coverage(&job.run_id).unwrap().unwrap();
    assert_eq!(first.trials, 2);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
}

#[test]
fn coverage_of_an_unknown_run_is_none() {
    let (svc, _store, _engine) = setup();
    assert!(svc.coverage("no-such-run").unwrap().is_none());
}
