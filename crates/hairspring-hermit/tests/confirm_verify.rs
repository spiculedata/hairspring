//! Strict verify: Hermit's own two-run determinism check, with failure allowed, run when asked for.
mod common;
use common::confirm::*;
use hairspring_engine::confirm::*;

fn strict() -> ConfirmRequest {
    ConfirmRequest {
        reruns: 1,
        strict_verify: true,
        ..ConfirmRequest::default()
    }
}

fn verify_of(checks: &[CheckResult]) -> &CheckResult {
    checks
        .iter()
        .find(|c| c.kind == CheckKind::StrictVerify)
        .expect("a strict verify check was recorded")
}

fn run(rig: &Rig) -> Vec<CheckResult> {
    let bundle = rig.bundle();
    rig.engine
        .confirm(&bundle, &strict(), &mut |_| true)
        .unwrap()
}

fn detail<'a>(check: &'a CheckResult, key: &str) -> &'a str {
    check.detail.get(key).map_or("", String::as_str)
}

#[test]
fn a_matched_bitwise_verify_passes_and_verifies_the_finding() {
    let rig = rig("verify-ok", RigOptions::default());
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Pass, "{verify:?}");
    assert_eq!(finding_status(&checks), FindingStatus::Verified);
    for flag in [
        "--verify ",
        "--verify-allow=failure",
        "--verify-strict",
        "--verify-json=",
    ] {
        assert!(
            verify.command.contains(flag),
            "{flag} missing from {}",
            verify.command
        );
    }
    assert!(verify.command.contains("--sched-seed=3"));
    let used = rig
        .calls()
        .into_iter()
        .filter(|c| c.contains("--verify-strict"))
        .count();
    assert_eq!(used, 1);
}

#[test]
fn a_verify_without_bitwise_parity_fails() {
    let rig = rig("verify-bad", RigOptions::default());
    rig.set("verify", "diverged");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Fail);
    assert_eq!(detail(verify, "verdict"), "diverged");
    assert_eq!(finding_status(&checks), FindingStatus::Confirmed);
}

#[test]
fn a_verify_that_left_no_result_is_inconclusive() {
    let rig = rig("verify-missing", RigOptions::default());
    rig.set("verify", "missing");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive);
    assert!(detail(verify, "inconclusive").contains("no result"));
}

#[test]
fn a_verify_needs_the_private_directory_for_its_result() {
    let o = RigOptions {
        with_summaries: false,
        ..RigOptions::default()
    };
    let rig = rig("verify-nodir", o);
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive);
    assert!(rig.calls().iter().all(|c| !c.contains("--verify-strict")));
}

#[test]
fn verify_is_not_run_unless_asked_for() {
    let rig = rig("verify-off", RigOptions::default());
    let bundle = rig.bundle();
    let checks = rig
        .engine
        .confirm(&bundle, &ConfirmRequest::default(), &mut |_| true)
        .unwrap();
    assert!(checks.iter().all(|c| c.kind != CheckKind::StrictVerify));
}
