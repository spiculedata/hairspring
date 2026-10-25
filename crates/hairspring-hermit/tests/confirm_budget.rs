//! The wall budget and the caller's cancel reach every run of a confirmation, the replay included: a replay is given
//! only what is left of the budget, and is heard as a run between checks.
mod common;
use common::confirm::*;
use hairspring_engine::confirm::*;
use std::time::{Duration, Instant};

fn detail<'a>(check: &'a CheckResult, key: &str) -> &'a str {
    check.detail.get(key).map_or("", String::as_str)
}

/// A rig whose bundle is recorded and whose call counter starts again from zero.
fn fresh(name: &str) -> (Rig, hairspring_core::bundle::Bundle) {
    let rig = rig(name, RigOptions::default());
    let bundle = rig.bundle();
    rig.set("count", "0");
    (rig, bundle)
}

#[test]
fn a_replay_is_cut_at_what_is_left_of_the_wall_budget_and_is_inconclusive() {
    let (rig, bundle) = fresh("budget-replay");
    // The first call (the rerun) is quick; the second (the replay) would take four seconds, well inside the ten
    // second trial timeout but well outside the two second budget.
    rig.set("sleep", "4");
    rig.set("sleep_from", "2");
    let req = ConfirmRequest {
        reruns: 1,
        wall_secs: 2,
        ..ConfirmRequest::default()
    };
    let started = Instant::now();
    let checks = rig.engine.confirm(&bundle, &req, &mut |_| true).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(3500),
        "the replay ran past the budget: {:?}",
        started.elapsed()
    );
    assert_eq!(checks[0].status, CheckStatus::Pass);
    assert_eq!(checks[1].kind, CheckKind::BundleReplay);
    assert_eq!(checks[1].status, CheckStatus::Inconclusive, "not a miss");
    assert_eq!(checks[1].counts.inconclusive, 1);
    assert!(
        detail(&checks[1], "inconclusive").contains("wall budget"),
        "{:?}",
        checks[1].detail
    );
}

#[test]
fn a_cancel_is_heard_after_the_replay() {
    let (rig, bundle) = fresh("budget-cancel-replay");
    let req = ConfirmRequest {
        reruns: 1,
        control: Some("fixed".into()),
        strict_verify: true,
        ..ConfirmRequest::default()
    };
    let mut calls = 0;
    let checks = rig
        .engine
        .confirm(&bundle, &req, &mut |_| {
            calls += 1;
            calls < 2
        })
        .unwrap();
    assert_eq!(checks[1].kind, CheckKind::BundleReplay);
    assert_eq!(checks[1].status, CheckStatus::Pass);
    assert_eq!(checks[2].kind, CheckKind::FixedControl);
    assert_eq!(checks[2].status, CheckStatus::Skipped);
    assert_eq!(detail(&checks[2], "skipped"), "cancelled");
    assert_eq!(checks[3].status, CheckStatus::Skipped);
    assert_eq!(rig.calls().len(), 2, "{:?}", rig.calls());
}

/// A bundle from before outputs were hashed, so the reruns can only be compared with the first one.
fn without_baseline(
    mut bundle: hairspring_core::bundle::Bundle,
) -> hairspring_core::bundle::Bundle {
    bundle.params.remove("output_hash");
    bundle.params.remove("schedule_fingerprint");
    bundle
}

#[test]
fn a_hung_first_rerun_is_not_the_baseline_the_others_are_compared_with() {
    let rig = rig(
        "first-rerun",
        RigOptions {
            hangs_once: true,
            ..RigOptions::default()
        },
    );
    let bundle = without_baseline(rig.bundle());
    std::fs::write(rig.dir.join("hang_once"), "").unwrap();
    let req = ConfirmRequest {
        reruns: 2,
        ..ConfirmRequest::default()
    };
    let checks = rig.engine.confirm(&bundle, &req, &mut |_| true).unwrap();
    let rerun = &checks[0];
    assert_eq!(rerun.counts.failed, 0, "{:?}", rerun.detail);
    assert_eq!(
        (rerun.counts.passed, rerun.counts.inconclusive),
        (1, 1),
        "the hung rerun is inconclusive and the other matches"
    );
    assert_eq!(rerun.status, CheckStatus::Inconclusive);
}

#[test]
fn the_detail_says_when_reruns_were_compared_with_the_first_rerun() {
    let rig = rig("baseline-note", RigOptions::default());
    let recorded = rig.bundle();
    let req = ConfirmRequest {
        reruns: 2,
        ..ConfirmRequest::default()
    };
    let with = rig.engine.confirm(&recorded, &req, &mut |_| true).unwrap();
    assert!(!with[0].detail.contains_key("compared_with"));
    let old = without_baseline(recorded);
    let without = rig.engine.confirm(&old, &req, &mut |_| true).unwrap();
    assert!(
        detail(&without[0], "compared_with").contains("first rerun"),
        "{:?}",
        without[0].detail
    );
}

fn hang_is_the_bug(name: &str) -> (Rig, hairspring_core::bundle::Bundle) {
    let rig = rig(
        name,
        RigOptions {
            hang_is_violation: true,
            ..RigOptions::default()
        },
    );
    let bundle = rig.bundle();
    rig.set("count", "0");
    (rig, bundle)
}

#[test]
fn a_rerun_the_wall_budget_cut_short_is_inconclusive_even_where_a_hang_is_the_bug() {
    let (rig, bundle) = hang_is_the_bug("clip-rerun");
    rig.set("sleep", "5");
    let req = ConfirmRequest {
        reruns: 1,
        wall_secs: 1,
        ..ConfirmRequest::default()
    };
    let checks = rig.engine.confirm(&bundle, &req, &mut |_| true).unwrap();
    let c = &checks[0].counts;
    assert_eq!(
        (c.passed, c.failed, c.inconclusive),
        (0, 0, 1),
        "{:?}",
        checks[0]
    );
    assert_eq!(checks[0].status, CheckStatus::Inconclusive);
}

#[test]
fn a_control_the_wall_budget_cut_short_is_inconclusive_not_control_failed() {
    let (rig, bundle) = hang_is_the_bug("clip-control");
    // The rerun and the replay are quick; the control (the third call) would take five seconds.
    rig.set("sleep", "5");
    rig.set("sleep_from", "3");
    let req = ConfirmRequest {
        reruns: 1,
        wall_secs: 3,
        control: Some("fixed".into()),
        ..ConfirmRequest::default()
    };
    let checks = rig.engine.confirm(&bundle, &req, &mut |_| true).unwrap();
    let control = checks
        .iter()
        .find(|c| c.kind == CheckKind::FixedControl)
        .unwrap();
    assert_eq!(control.status, CheckStatus::Inconclusive, "{control:?}");
    assert_eq!(control.counts.failed, 0);
    assert_ne!(finding_status(&checks), FindingStatus::ControlFailed);
}

#[test]
fn a_real_hang_at_the_targets_full_time_limit_is_still_a_violation_where_a_hang_is_the_bug() {
    let rig = rig(
        "real-hang",
        RigOptions {
            hangs_once: true,
            hang_is_violation: true,
            ..RigOptions::default()
        },
    );
    let bundle = without_baseline(rig.bundle());
    std::fs::write(rig.dir.join("hang_once"), "").unwrap();
    // The budget is ample, so the run gets the target's own two seconds and hangs through all of them.
    let req = ConfirmRequest {
        reruns: 1,
        ..ConfirmRequest::default()
    };
    let checks = rig.engine.confirm(&bundle, &req, &mut |_| true).unwrap();
    let c = &checks[0].counts;
    assert_eq!(
        (c.passed, c.failed, c.inconclusive),
        (1, 0, 0),
        "{:?}",
        checks[0]
    );
}
