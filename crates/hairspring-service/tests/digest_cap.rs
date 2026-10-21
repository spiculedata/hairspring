//! A run may hold only so many bytes of digests. A trial whose digest does not fit is still recorded, with a marked
//! stand-in, and the run says it is incomplete; the batch is not refused and nothing is dropped without a trace.
mod common;

use common::*;
use hairspring_engine::trial_log::TrialReport;
use hairspring_props::AssertKind;
use hairspring_service::{
    completion_of, RunCompletion, Service, ServiceConfig, ServiceError, TrialQuery,
    DIGEST_CAPPED_REASON,
};
use hairspring_store::{MAX_DIGEST_BLOB_BYTES, MAX_DIGEST_BYTES_PER_RUN};

fn three_trials() -> Vec<TrialReport> {
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 3, 0);
    let mut reports: Vec<TrialReport> = (1..=3).map(|seed| rec.pass(seed, &p)).collect();
    reports[1].reason = "worker words".to_string();
    reports
}

/// A service whose digest allowance holds the first trial's digest and a little more.
fn tight(
    reports: &[TrialReport],
) -> (
    Service,
    std::sync::Arc<dyn hairspring_store::Store>,
    hairspring_engine::engine::Engine,
) {
    let (svc, store, engine) = setup();
    let cap = reports[0].digest.len() as u64 + 1;
    let svc = svc.with_config(ServiceConfig {
        digest_bytes_per_run: cap,
        ..ServiceConfig::default()
    });
    (svc, store, engine)
}

#[test]
fn a_digest_over_the_allowance_is_stored_as_a_marked_stand_in_and_the_batch_is_accepted() {
    let reports = three_trials();
    let (svc, _store, engine) = tight(&reports);
    let job = lease_with_budget(&svc, &engine, &local(), 3);

    let receipt = svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    assert_eq!((receipt.inserted, receipt.duplicates), (3, 0));

    let page = svc
        .trial_page(&job.run_id, &TrialQuery::default())
        .unwrap()
        .unwrap();
    let capped: Vec<bool> = page.trials.iter().map(|t| t.digest_capped).collect();
    assert_eq!(capped, [false, true, true], "the first digest fit");
    let row = &page.trials[1];
    assert!(!row.rated && !row.claimable, "a stand-in decides nothing");
    assert_eq!(row.class, "excluded");
    assert!(row.guest.reason.starts_with(DIGEST_CAPPED_REASON));
    assert!(
        row.guest.reason.contains("worker words"),
        "the worker's own reason is kept after the marker: {}",
        row.guest.reason
    );
    assert!(!row.digest_unreadable, "the stand-in is a valid digest");
    assert!(!page.trials[0]
        .guest
        .reason
        .starts_with(DIGEST_CAPPED_REASON));
}

#[test]
fn the_trial_detail_says_the_digest_was_replaced() {
    let reports = three_trials();
    let (svc, _store, engine) = tight(&reports);
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    let detail = svc.trial_detail(&job.run_id, 2).unwrap().unwrap();
    assert!(detail.trial.digest_capped);
    assert!(
        !svc.trial_detail(&job.run_id, 0)
            .unwrap()
            .unwrap()
            .trial
            .digest_capped
    );
}

#[test]
fn a_run_that_replaced_a_digest_does_not_finish_as_complete() {
    let reports = three_trials();
    let (svc, store, engine) = tight(&reports);
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    finish(&svc, &job, &local(), passed(3, Some(3)));

    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(run.ingest_incomplete, "a stand-in hides what the trial saw");
    assert_eq!(completion_of(&run), RunCompletion::EndedEarly);
    // Readers are told the true reason, not that the run was ended early, and UNMET stays withheld.
    let completion = svc.completion_for(&run).unwrap();
    assert_eq!(completion, RunCompletion::DigestAllowanceUsedUp);
    assert_eq!(
        completion.label(),
        "digest allowance used up: trial data is incomplete"
    );
    assert!(!completion.allows_unmet(true));
    let page = svc
        .property_page(&job.run_id, &Default::default(), None)
        .unwrap()
        .unwrap();
    assert_eq!(page.header.completion, RunCompletion::DigestAllowanceUsedUp);
    assert!(
        page.notes.iter().any(|n| n.contains("digest allowance")),
        "{:?}",
        page.notes
    );
    assert!(page.notes.iter().all(|n| !n.contains("ended early")));
}

#[test]
fn a_timed_out_trial_over_the_allowance_keeps_what_the_worker_reported() {
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 3, 0);
    let first = rec.pass(1, &p);
    let timed_out = rec.trial(
        2,
        &p,
        hairspring_engine::trial_log::ReportedVerdict::Inconclusive,
        true,
    );
    let (svc, _store, engine) = tight(std::slice::from_ref(&first));
    let job = lease_with_budget(&svc, &engine, &local(), 2);
    svc.ingest_trials(&job.job_id, &local(), &[first, timed_out])
        .unwrap();
    let row = svc.trial_detail(&job.run_id, 1).unwrap().unwrap().trial;
    assert!(row.digest_capped);
    assert!(row.timed_out, "the worker said it timed out");
    assert!(!row.rated && !row.claimable);
    assert_eq!(row.verdict, "inconclusive");
}

#[test]
fn a_run_within_its_allowance_finishes_as_complete() {
    let reports = three_trials();
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    finish(&svc, &job, &local(), passed(3, Some(3)));
    let run = store.get_run(&job.run_id).unwrap().unwrap();
    assert!(!run.ingest_incomplete);
    assert_eq!(completion_of(&run), RunCompletion::Complete);
}

#[test]
fn sending_a_batch_again_after_a_digest_was_replaced_is_a_duplicate() {
    let reports = three_trials();
    let (svc, _store, engine) = tight(&reports);
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    let again = svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    assert_eq!((again.inserted, again.duplicates), (0, 3));
}

#[test]
fn a_worker_cannot_claim_a_trial_was_capped_by_sending_the_reserved_text() {
    let mut reports = three_trials();
    reports[0].reason = format!("{DIGEST_CAPPED_REASON}forged");
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    let err = svc
        .ingest_trials(&job.job_id, &local(), &reports)
        .expect_err("the reserved text is refused");
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err}");
}

#[test]
fn the_default_allowance_is_the_documented_constant_and_holds_a_largest_digest() {
    let cfg = ServiceConfig::default();
    assert_eq!(cfg.digest_bytes_per_run, MAX_DIGEST_BYTES_PER_RUN);
    assert!(cfg.digest_bytes_per_run >= 8 * MAX_DIGEST_BLOB_BYTES as u64);
}
