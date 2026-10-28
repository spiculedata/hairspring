//! A sweep whose properties first fail at different seeds reports one failure per such seed. The server stores them all
//! in seed order, checks each as it checks the first, and the claim gate then needs every one of them confirmed.
mod common;
use common::*;
use hairspring_core::bundle::Bundle;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus};
use hairspring_engine::report::{FailureReport, RunOutcome};
use hairspring_engine::sweep::MAX_FAILURE_ROWS;
use hairspring_engine::trial_log::ReportedVerdict;
use hairspring_props::claim_text::{BuildRecord, Label, SeedSelection};
use hairspring_props::AssertKind::Always;
use hairspring_service::{PropertyQuery, Service, ServiceError};

const TRUSTED: &str = "remote";
const TRIALS: u64 = 6;

fn label() -> Label {
    Label {
        build: BuildRecord {
            instrumented: false,
            behaviour_changes: false,
        },
        knowledge: Some(vec![]),
        seed_selection: SeedSelection::Blind,
    }
}

fn report_at(seed: u64) -> FailureReport {
    FailureReport {
        target: "lock-lease".into(),
        backend: "sim".into(),
        message: format!("violated at sched-seed {seed}"),
        original_len: 1,
        shrunk_len: 1,
        steps: vec![],
        bundle: Bundle::new(seed, "t", vec![])
            .with_target("lock-lease")
            .with_param("sched_seed", seed.to_string())
            .encode(),
    }
}

/// Property `a` fails first at seed 3 and property `b` first at seed 5, in a sweep of seeds 1 to 6.
fn trials() -> Vec<hairspring_engine::trial_log::TrialReport> {
    let mut rec = Recorder::new();
    (1..=TRIALS)
        .map(|seed| {
            let a = u64::from(seed == 3);
            let b = u64::from(seed == 5);
            let p = props_multi(&[("a", Always, "a", 4 - a, a), ("b", Always, "b", 4 - b, b)]);
            let verdict = if a + b > 0 {
                ReportedVerdict::Violation
            } else {
                ReportedVerdict::Pass
            };
            rec.trial(seed, &p, verdict, false)
        })
        .collect()
}

fn outcome(more: Vec<FailureReport>) -> RunOutcome {
    RunOutcome {
        trials_reported: Some(TRIALS),
        ..RunOutcome::failed(TRIALS, TRIALS, 5, report_at(3))
    }
    .with_more_failures(more)
}

struct Rig {
    svc: Service,
    run_id: String,
    job_id: String,
}

fn rig() -> Rig {
    rig_in(true)
}

/// The same trials, in a sweep run or in a search run.
fn rig_in(sweep: bool) -> Rig {
    let (svc, _, engine) = setup();
    let (remote, secret) = svc.register_remote_worker(TRUSTED, "{}").unwrap();
    svc.set_worker_trust(&remote.id, true).unwrap();
    let job = if sweep {
        lease_sweep(&svc, &engine, &remote.id, TRIALS)
    } else {
        lease_with_budget(&svc, &engine, &remote.id, TRIALS)
    };
    svc.ingest_trials_with_secret(&job.job_id, &remote.id, Some(&secret), &trials())
        .unwrap();
    Rig {
        svc,
        run_id: job.run_id,
        job_id: job.job_id,
    }
}

fn complete(rig: &Rig, outcome: &RunOutcome) -> Result<(), ServiceError> {
    let worker = rig.svc.list_workers().unwrap()[0].id.clone();
    rig.svc.complete(&rig.job_id, &worker, outcome)
}

fn confirm(rig: &Rig, failure_id: &str) {
    let ok = |kind| {
        CheckResult::new(kind, CheckStatus::Pass)
            .with_counts(1, 0, 0)
            .with_times(1, 2)
            .with_environment("host.container", "none")
    };
    let checks = [
        ok(CheckKind::SameSeedRerun),
        ok(CheckKind::BundleReplay).with_detail("environment_changed", "false"),
    ];
    rig.svc
        .record_confirmation(failure_id, "cli-1", true, &checks)
        .unwrap();
}

fn claim_allowed(rig: &Rig) -> bool {
    rig.svc
        .property_page(&rig.run_id, &PropertyQuery::default(), Some(&label()))
        .unwrap()
        .unwrap()
        .claim
        .allowed
}

fn failure_ids(rig: &Rig) -> Vec<String> {
    let detail = rig.svc.get_run(&rig.run_id).unwrap().unwrap();
    detail.failures.iter().map(|f| f.id.clone()).collect()
}

#[test]
fn every_reported_failure_is_stored_in_seed_order() {
    let rig = rig();
    complete(&rig, &outcome(vec![report_at(5)])).unwrap();
    let detail = rig.svc.get_run(&rig.run_id).unwrap().unwrap();
    let messages: Vec<_> = detail.failures.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(
        messages,
        vec!["violated at sched-seed 3", "violated at sched-seed 5"]
    );
}

#[test]
fn a_further_failure_for_another_target_is_refused() {
    let rig = rig();
    let stray = FailureReport {
        target: "elsewhere".into(),
        ..report_at(5)
    };
    let err = complete(&rig, &outcome(vec![stray])).unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err:?}");
    assert!(failure_ids(&rig).is_empty(), "nothing was stored");
}

#[test]
fn more_failures_than_a_sweep_may_store_are_refused_and_one_fewer_is_not() {
    let rising =
        |n: usize| -> Vec<FailureReport> { (0..n as u64).map(|i| report_at(5 + i)).collect() };
    // The first failure plus MAX_FAILURE_ROWS - 1 further ones is the most a sweep may report.
    refused(&rig(), &outcome(rising(MAX_FAILURE_ROWS)));
    let rig = rig();
    complete(&rig, &outcome(rising(MAX_FAILURE_ROWS - 1))).unwrap();
    assert_eq!(failure_ids(&rig).len(), MAX_FAILURE_ROWS);
}

#[test]
fn the_claim_stays_withheld_until_every_first_failure_seed_is_confirmed() {
    let rig = rig();
    complete(&rig, &outcome(vec![report_at(5)])).unwrap();
    let ids = failure_ids(&rig);
    assert_eq!(ids.len(), 2);
    assert!(!claim_allowed(&rig), "nothing is confirmed yet");
    confirm(&rig, &ids[0]);
    assert!(!claim_allowed(&rig), "seed 5 is still unconfirmed");
    confirm(&rig, &ids[1]);
    assert!(
        claim_allowed(&rig),
        "both first-failure seeds are confirmed"
    );
}

fn refused(rig: &Rig, outcome: &RunOutcome) {
    let err = complete(rig, outcome).unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err:?}");
    assert!(failure_ids(rig).is_empty(), "nothing was stored");
}

#[test]
fn further_failures_without_a_first_failure_are_refused() {
    let rig = rig();
    let orphan = RunOutcome {
        failure: None,
        ..outcome(vec![report_at(5)])
    };
    refused(&rig, &orphan);
}

#[test]
fn further_failures_in_a_run_that_is_not_a_sweep_are_refused() {
    let rig = rig_in(false);
    refused(&rig, &outcome(vec![report_at(5)]));
}

#[test]
fn further_failures_must_have_strictly_ascending_distinct_seeds_after_the_first() {
    for seeds in [vec![2], vec![3], vec![5, 5], vec![6, 5]] {
        let rig = rig();
        let more = seeds.iter().map(|s| report_at(*s)).collect();
        refused(&rig, &outcome(more));
    }
}

#[test]
fn the_bundles_of_one_run_may_not_total_more_than_the_request_can_carry() {
    let rig = rig();
    let big = |seed: u64| {
        let mut f = report_at(seed);
        let bundle = Bundle::new(seed, "t", vec![])
            .with_target("lock-lease")
            .with_param("sched_seed", seed.to_string())
            .with_param("padding", "x".repeat(700_000));
        f.bundle = bundle.encode();
        f
    };
    let oversized = RunOutcome {
        failure: Some(big(3)),
        ..outcome(vec![big(5), big(6)])
    };
    refused(&rig, &oversized);
}

#[test]
fn a_result_the_server_refuses_ends_the_run_as_an_error_and_releases_the_job() {
    let rig = rig();
    let worker = rig.svc.list_workers().unwrap()[0].id.clone();
    let bad = outcome(vec![report_at(2)]);
    rig.svc
        .complete_or_error(&rig.job_id, &worker, &bad)
        .expect("the refusal is recorded, not returned");
    let run = rig.svc.get_run(&rig.run_id).unwrap().unwrap().run;
    assert_eq!(run.state, hairspring_store::RunState::Error, "{run:?}");
    assert!(run.error.unwrap_or_default().contains("refused"));
    // The job is closed: nothing is left to lease for this run.
    assert!(rig.svc.lease(&worker).unwrap().is_none());
}
