//! The coverage series of a run is worked out when its job finishes and stored with the run, so a read of a finished
//! run scans nothing, in this process or after a restart.
mod common;

use common::*;
use hairspring_props::AssertKind;
use hairspring_service::Service;
use hairspring_store::{JobResult, RunState, RunStats, Store};
use std::sync::Arc;

/// A run of two trials, ingested but not finished.
fn open_run() -> (Service, Arc<dyn Store>, hairspring_service::LeasedJob) {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 2);
    let mut rec = Recorder::new();
    let a = props("c.a", AssertKind::Always, 1, 0);
    let b = props("c.b", AssertKind::Always, 1, 0);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(1, &a), rec.pass(2, &b)])
        .unwrap();
    (svc, store, job)
}

fn as_json(c: &hairspring_service::Coverage) -> String {
    serde_json::to_string(c).unwrap()
}

/// Finishes the job straight through the store with the given stored series, as a run finished by an older build or
/// with a damaged row would look.
fn finish_with_stored_series(
    store: &Arc<dyn Store>,
    job: &hairspring_service::LeasedJob,
    s: Option<&str>,
) {
    let result = JobResult {
        state: RunState::Passed,
        steps: 2,
        trials: 2,
        error: None,
        failure: None,
        event_payload_json: "{}".to_string(),
        stats: RunStats {
            coverage_json: s.map(str::to_owned),
            ..RunStats::default()
        },
    };
    store
        .finish_job(&job.job_id, &local(), &result, hairspring_store::now_ms())
        .unwrap();
}

#[test]
fn finishing_a_job_works_out_the_series_once_and_reads_scan_nothing() {
    let (svc, _store, job) = open_run();
    let scans_before = svc.coverage_scans();
    finish(&svc, &job, &local(), passed(2, Some(2)));
    assert_eq!(
        svc.coverage_scans(),
        scans_before + 1,
        "one scan, at completion"
    );
    for _ in 0..3 {
        assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 2);
    }
    assert_eq!(svc.coverage_scans(), scans_before + 1, "reads scan nothing");
}

#[test]
fn the_stored_series_equals_the_one_an_open_run_served() {
    let (svc, _store, job) = open_run();
    let open = svc.coverage(&job.run_id).unwrap().unwrap();
    finish(&svc, &job, &local(), passed(2, Some(2)));
    let finished = svc.coverage(&job.run_id).unwrap().unwrap();
    assert_eq!(as_json(&finished), as_json(&open));
    assert_eq!(finished.points.last().unwrap().ids_hit, 2);
}

#[test]
fn a_new_service_over_the_same_store_serves_the_series_without_scanning() {
    let (svc, store, job) = open_run();
    finish(&svc, &job, &local(), passed(2, Some(2)));
    let before = as_json(&svc.coverage(&job.run_id).unwrap().unwrap());

    let restarted = Service::new(store);
    let after = restarted.coverage(&job.run_id).unwrap().unwrap();
    assert_eq!(as_json(&after), before);
    assert_eq!(restarted.coverage_scans(), 0, "nothing was recomputed");
}

#[test]
fn a_run_finished_without_a_stored_series_is_scanned_once_and_then_cached() {
    let (svc, store, job) = open_run();
    finish_with_stored_series(&store, &job, None);
    let first = svc.coverage(&job.run_id).unwrap().unwrap();
    svc.coverage(&job.run_id).unwrap();
    assert_eq!(first.trials, 2);
    assert_eq!(
        svc.coverage_scans(),
        1,
        "an older run keeps the read-time path"
    );
}

#[test]
fn an_unreadable_stored_series_falls_back_to_a_scan_and_does_not_fail_the_read() {
    for damaged in ["not json", "{\"run_id\":\"other\"}", "[]", ""] {
        let (svc, store, job) = open_run();
        finish_with_stored_series(&store, &job, Some(damaged));
        let c = svc
            .coverage(&job.run_id)
            .unwrap_or_else(|e| panic!("{damaged:?} must not fail the read: {e}"))
            .unwrap();
        assert_eq!(c.trials, 2, "{damaged:?}");
        assert_eq!(c.run_id, job.run_id, "{damaged:?}");
        assert_eq!(svc.coverage_scans(), 1, "{damaged:?}");
    }
}

#[test]
fn a_stored_series_that_names_another_run_is_not_served() {
    let (svc, store, job) = open_run();
    let foreign = serde_json::json!({
        "run_id": "run_somewhere_else", "trials": 99, "downsampled": false, "points": []
    })
    .to_string();
    finish_with_stored_series(&store, &job, Some(&foreign));
    let c = svc.coverage(&job.run_id).unwrap().unwrap();
    assert_eq!((c.run_id.as_str(), c.trials), (job.run_id.as_str(), 2));
}

#[test]
fn a_cancelled_run_stores_no_series_and_is_still_readable() {
    let (svc, store, job) = open_run();
    assert!(store
        .cancel_run(&job.run_id, hairspring_store::now_ms())
        .unwrap());
    finish(&svc, &job, &local(), passed(2, Some(2)));
    assert_eq!(store.run_coverage_json(&job.run_id).unwrap(), None);
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 2);
}
