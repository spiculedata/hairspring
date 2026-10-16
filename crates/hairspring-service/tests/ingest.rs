//! Per-trial storage through the service: what a worker may send, what the server folds from it itself, and what it
//! refuses.
mod common;

use common::*;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_props::{AssertKind, PropertyStatus, RunProps, TrialDigest};
use hairspring_service::ServiceError;
use hairspring_store::RunState;

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
    let (remote, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    let job = lease(&svc, &engine, &remote.id);
    let mut rec = Recorder::new();
    let report = rec.pass(1, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials_with_secret(&job.job_id, &remote.id, Some(&secret), &[report])
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
    svc.ingest_trials_with_secret(&job3.job_id, &remote.id, Some(&secret), &[report])
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
    let (remote, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    let job = lease(&svc, &engine, &remote.id);
    let mut rec = Recorder::new();
    let report = rec.pass(1, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials_with_secret(&job.job_id, &remote.id, Some(&secret), &[report])
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

    let job2 = lease_with_budget(&svc, &engine, &local(), 2);
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
fn a_pass_from_a_target_that_sends_no_events_is_stored_and_left_out_of_the_statistics() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let counted = Recorder::new().pass(2, &props("a.id", AssertKind::Always, 3, 0));
    // No events at all: the digest is excluded, the verdict is still a pass.
    let plain = TrialReport {
        seq: 1,
        seed: 1,
        catalog: Vec::new(),
        digest: TrialDigest {
            flags: hairspring_props::DigestFlags {
                excluded: true,
                ..hairspring_props::DigestFlags::default()
            },
            ..TrialDigest::default()
        }
        .encode(),
        ..counted.clone()
    };
    svc.ingest_trials(&job.job_id, &local(), &[counted, plain])
        .unwrap();
    finish(&svc, &job, &local(), passed(2, Some(2)));
    let row = store.get_trial(&job.run_id, 1).unwrap().unwrap();
    assert!(
        !row.rated && !row.claimable,
        "an excluded trial is never rated"
    );
    assert!(store.get_trial(&job.run_id, 0).unwrap().unwrap().rated);
    assert_eq!(svc.property_stats(&job.run_id).unwrap()[0].trials, 1);
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
    let job = lease_with_budget(&svc, &engine, &local(), 600);
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

/// Two trials that disagree about an id's kind, the second carrying the marker text in its message and location.
fn mismatching_trials() -> Vec<TrialReport> {
    let mut rec = Recorder::new();
    let first = rec.pass(1, &props("guest.idmark", AssertKind::Always, 3, 0));
    let mut other = props("guest.idmark", AssertKind::Sometimes, 3, 0);
    other.properties[0].msg = "GUESTMSGMARK".to_string();
    other.properties[0].loc = Some(hairspring_props::Loc {
        file: "GUESTFILEMARK.c".to_string(),
        line: 1,
        func: None,
    });
    let second = rec.pass(2, &other);
    vec![first, second]
}

#[test]
fn the_programs_text_from_a_kind_mismatch_is_not_stored_in_the_run_summary() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    svc.ingest_trials(&job.job_id, &local(), &mismatching_trials())
        .unwrap();
    finish(&svc, &job, &local(), passed(2, Some(2)));
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    let stored = run.property_summary_json.clone().unwrap();
    assert!(!stored.contains("MARK"), "{stored}");
    let summary: hairspring_props::RunSummary = serde_json::from_str(&stored).unwrap();
    assert_eq!(summary.kind_mismatches, 1, "the count stays");
    assert!(summary.kind_mismatch_details.is_empty());
    let outcome =
        serde_json::to_string(&svc.outcome(&job.run_id).unwrap().unwrap().property_run).unwrap();
    assert!(!outcome.contains("MARK"), "{outcome}");
}

#[test]
fn nothing_from_the_program_appears_in_a_run_detail_outside_the_guest_object() {
    let (svc, _, engine) = setup();
    let job = lease(&svc, &engine, &local());
    svc.ingest_trials(&job.job_id, &local(), &mismatching_trials())
        .unwrap();
    finish(&svc, &job, &local(), passed(2, Some(2)));
    let mut detail = serde_json::to_value(svc.get_run(&job.run_id).unwrap().unwrap()).unwrap();
    let guest = detail["properties"]["guest"].take();
    assert!(guest.is_object(), "{detail}");
    let outside = detail.to_string();
    assert!(
        !outside.contains("guest.idmark") && !outside.contains("MARK"),
        "{outside}"
    );
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

#[test]
fn rows_left_by_an_earlier_attempt_are_not_folded_into_a_later_attempts_result() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let reports: Vec<_> = (0..3)
        .map(|i| rec.pass(i, &props("old.id", AssertKind::Always, 0, 1)))
        .collect();
    svc.ingest_trials(&job.job_id, &local(), &reports)
        .unwrap_err();
    let verdicts: Vec<_> = reports
        .into_iter()
        .map(|r| TrialReport {
            verdict: ReportedVerdict::Violation,
            ..r
        })
        .collect();
    svc.ingest_trials(&job.job_id, &local(), &verdicts).unwrap();
    svc.fail(&job.job_id, &local(), "interrupted").unwrap();
    let again = svc.lease(&local()).unwrap().expect("requeued");
    // The second attempt ends without sending any trials.
    finish(
        &svc,
        &again,
        &local(),
        RunOutcome::error("the worker could not start"),
    );
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(!run.ingest_incomplete, "this attempt held no rows");
    assert!(
        svc.property_stats(&job.run_id).unwrap().is_empty(),
        "no figures from attempt one"
    );
    assert!(run.property_summary_json.is_none());
}
