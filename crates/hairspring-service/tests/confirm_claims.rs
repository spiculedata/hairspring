//! A run's claim uses the recorded check results only. A failure with no checks, or checks that do not reach confirmed,
//! gets no sentence; one the checks confirm gets the environment-pinned wording; and the words the rules forbid never
//! appear.
mod common;

use common::fixture::*;
use common::*;
use hairspring_core::bundle::Bundle;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus};
use hairspring_props::claim_text::{BuildRecord, Label, SeedSelection};
use hairspring_service::{PropertyPage, PropertyQuery, Service};
use hairspring_store::{NewFailure, Store};
use std::sync::Arc;

fn declared() -> Label {
    Label {
        build: BuildRecord {
            instrumented: false,
            behaviour_changes: false,
        },
        knowledge: Some(vec![]),
        seed_selection: SeedSelection::Blind,
    }
}

/// The run of `trial_set` (seed 3 fails `b.broken`) reported by a trusted remote worker, with a stored failure for seed 3.
struct Rig {
    svc: Service,
    store: Arc<dyn Store>,
    run_id: String,
    failure_id: String,
}

fn rig() -> Rig {
    let (svc, store, engine) = setup();
    let (remote, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    svc.set_worker_trust(&remote.id, true).unwrap();
    let job = lease_with_budget(&svc, &engine, &remote.id, 6);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials_with_secret(&job.job_id, &remote.id, Some(&secret), &reports)
        .unwrap();
    finish(&svc, &job, &remote.id, passed(6, Some(6)));
    let bundle = Bundle::new(3, "t", vec![])
        .with_target("lock-lease")
        .with_param("sched_seed", "3")
        .encode();
    let failure = store
        .add_failure(
            &NewFailure {
                run_id: job.run_id.clone(),
                target: "lock-lease".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: store.put_blob(bundle.as_bytes()).unwrap(),
                steps_json: "[]".into(),
            },
            9,
        )
        .unwrap();
    Rig {
        svc,
        store,
        run_id: job.run_id,
        failure_id: failure.id,
    }
}

fn pass(kind: CheckKind) -> CheckResult {
    CheckResult::new(kind, CheckStatus::Pass)
        .with_counts(1, 0, 0)
        .with_times(1, 2)
}

fn rerun(pinned: bool) -> CheckResult {
    let r = pass(CheckKind::SameSeedRerun);
    if pinned {
        r.with_environment("host.container", "none")
    } else {
        r
    }
}

fn skipped(kind: CheckKind) -> CheckResult {
    CheckResult::new(kind, CheckStatus::Skipped)
        .with_detail("skipped", "cancelled")
        .with_times(7, 7)
}

fn replay(pinned: bool) -> CheckResult {
    let r = pass(CheckKind::BundleReplay).with_detail("environment_changed", "false");
    if pinned {
        r.with_environment("host.container", "none")
    } else {
        r
    }
}

fn page(rig: &Rig) -> PropertyPage {
    rig.svc
        .property_page(&rig.run_id, &PropertyQuery::default(), Some(&declared()))
        .unwrap()
        .unwrap()
}

fn record(rig: &Rig, trusted: bool, checks: &[CheckResult]) {
    rig.svc
        .record_confirmation(&rig.failure_id, "cli-1", trusted, checks)
        .unwrap();
}

#[test]
fn a_failure_with_no_checks_makes_no_claim() {
    let rig = rig();
    let p = page(&rig);
    assert!(!p.claim.allowed);
    assert_eq!(p.claim.text, None);
    assert!(p.claim.caveats.contains(&"single_run"));
}

#[test]
fn confirming_checks_open_the_claim_with_the_environment_pinned_wording() {
    let rig = rig();
    record(&rig, true, &[rerun(true), replay(true)]);
    let p = page(&rig);
    assert!(p.claim.allowed, "{:?}", p.claim.withheld);
    let text = p.claim.text.as_deref().expect("a sentence");
    assert!(
        text.starts_with(
            "Reproduced (confirmed): a property failed in seed 3 under the recorded environment;"
        ),
        "{text}"
    );
    assert!(!p.claim.caveats.contains(&"environment_not_pinned"));
}

#[test]
fn checks_without_a_recorded_environment_say_the_environment_is_not_pinned() {
    let rig = rig();
    record(&rig, true, &[rerun(false), replay(false)]);
    let p = page(&rig);
    let text = p.claim.text.as_deref().expect("a sentence");
    assert!(text.contains("environment not pinned"), "{text}");
    assert!(p.claim.caveats.contains(&"environment_not_pinned"));
}

#[test]
fn a_verified_failure_says_verified() {
    let rig = rig();
    record(
        &rig,
        true,
        &[rerun(true), replay(true), pass(CheckKind::StrictVerify)],
    );
    let text = page(&rig).claim.text.expect("a sentence");
    assert!(text.starts_with("Reproduced (verified):"), "{text}");
}

#[test]
fn reruns_alone_do_not_reach_the_claim() {
    let rig = rig();
    record(&rig, true, &[pass(CheckKind::SameSeedRerun)]);
    let p = page(&rig);
    assert!(!p.claim.allowed);
    assert_eq!(p.claim.text, None);
}

#[test]
fn checks_that_say_not_reproducible_withdraw_the_claim() {
    let rig = rig();
    record(&rig, true, &[rerun(true), replay(true)]);
    assert!(page(&rig).claim.text.is_some());
    let miss = CheckResult::new(CheckKind::SameSeedRerun, CheckStatus::Fail)
        .with_counts(0, 3, 0)
        .with_times(5, 6);
    record(&rig, true, &[miss]);
    let p = page(&rig);
    assert_eq!(
        p.claim.text, None,
        "the newest rerun check says it did not repeat"
    );
    assert!(!p.claim.allowed);
}

#[test]
fn a_control_that_still_fails_withdraws_the_claim() {
    let rig = rig();
    record(&rig, true, &[rerun(true), replay(true)]);
    let control = CheckResult::new(CheckKind::FixedControl, CheckStatus::Fail)
        .with_counts(0, 1, 0)
        .with_times(5, 6);
    record(&rig, true, &[control]);
    let p = page(&rig);
    assert_eq!(p.claim.text, None);
}

#[test]
fn checks_from_an_untrusted_runner_never_open_the_claim() {
    let rig = rig();
    record(&rig, false, &[rerun(true), replay(true)]);
    let p = page(&rig);
    assert!(!p.claim.allowed);
    assert_eq!(p.claim.text, None);
    assert!(
        p.claim
            .withheld
            .iter()
            .any(|w| w.contains("no reproduction on a trusted worker")),
        "{:?}",
        p.claim.withheld
    );
}

#[test]
fn no_generated_sentence_uses_the_forbidden_words() {
    let rig = rig();
    record(
        &rig,
        true,
        &[rerun(true), replay(true), pass(CheckKind::StrictVerify)],
    );
    let text = page(&rig).claim.text.unwrap().to_lowercase();
    for word in ["found", "discovered", "unassisted"] {
        assert!(!text.contains(word), "{word}: {text}");
    }
    let _ = &rig.store;
}

#[test]
fn an_untrusted_runners_checks_and_a_trusted_runners_control_do_not_open_the_claim() {
    let rig = rig();
    record(
        &rig,
        false,
        &[rerun(true), replay(true), pass(CheckKind::StrictVerify)],
    );
    record(&rig, true, &[pass(CheckKind::FixedControl)]);
    let p = page(&rig);
    assert!(!p.claim.allowed, "{:?}", p.claim.withheld);
    assert_eq!(p.claim.text, None);
}

#[test]
fn a_trusted_confirmation_still_opens_the_claim_beside_an_untrusted_one() {
    let rig = rig();
    record(&rig, false, &[rerun(true), replay(true)]);
    record(&rig, true, &[rerun(true), replay(true)]);
    assert!(page(&rig).claim.allowed);
}

#[test]
fn a_cancelled_confirmation_after_a_confirmed_one_leaves_it_confirmed() {
    let rig = rig();
    record(&rig, true, &[rerun(true), replay(true)]);
    record(
        &rig,
        true,
        &[
            skipped(CheckKind::SameSeedRerun),
            skipped(CheckKind::BundleReplay),
        ],
    );
    let checks = rig.svc.failure_checks(&rig.failure_id).unwrap().unwrap();
    assert_eq!(checks.finding, "confirmed");
    assert!(checks.environment_pinned);
    let text = page(&rig).claim.text.expect("a sentence");
    assert!(text.starts_with("Reproduced (confirmed):"), "{text}");
}

#[test]
fn an_old_strict_verify_does_not_lift_a_newer_confirmation_to_verified() {
    let rig = rig();
    record(
        &rig,
        true,
        &[rerun(true), replay(true), pass(CheckKind::StrictVerify)],
    );
    record(&rig, true, &[rerun(true), replay(true)]);
    let checks = rig.svc.failure_checks(&rig.failure_id).unwrap().unwrap();
    assert_eq!(checks.finding, "confirmed");
    let text = page(&rig).claim.text.expect("a sentence");
    assert!(text.starts_with("Reproduced (confirmed):"), "{text}");
}

#[test]
fn reruns_and_a_replay_from_different_confirmations_do_not_pin_the_environment() {
    let rig = rig();
    record(&rig, true, &[replay(true)]);
    record(&rig, true, &[rerun(true)]);
    let checks = rig.svc.failure_checks(&rig.failure_id).unwrap().unwrap();
    assert_eq!(checks.finding, "rerun_repeated");
    assert!(!checks.environment_pinned);
    assert_eq!(page(&rig).claim.text, None);
}

#[test]
fn the_finding_names_the_confirmation_it_comes_from() {
    let rig = rig();
    record(&rig, true, &[rerun(true), replay(true)]);
    record(&rig, true, &[skipped(CheckKind::SameSeedRerun)]);
    let checks = rig.svc.failure_checks(&rig.failure_id).unwrap().unwrap();
    let from = checks.confirmation_id.clone().expect("a confirmation");
    let first = checks.checks.iter().find(|c| c.kind == "bundle_replay");
    assert_eq!(first.map(|c| c.confirmation_id.clone()), Some(from));
}

#[test]
fn an_untrusted_newer_confirmation_cannot_withdraw_a_trusted_claim() {
    let rig = rig();
    record(&rig, true, &[rerun(true), replay(true)]);
    let miss = CheckResult::new(CheckKind::SameSeedRerun, CheckStatus::Fail)
        .with_counts(0, 3, 0)
        .with_times(50, 60);
    record(&rig, false, &[miss]);
    let p = page(&rig);
    assert!(p.claim.allowed, "{:?}", p.claim.withheld);
    assert!(p.claim.text.is_some());
}
