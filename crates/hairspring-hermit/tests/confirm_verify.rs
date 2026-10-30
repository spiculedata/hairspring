//! Strict verify: Hermit's own two-run determinism check, run when asked for. Either exit status of the guest is
//! accepted; the engine judges the finding.
mod common;
use common::confirm::*;
use hairspring_engine::confirm::*;
use std::time::Duration;

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
        "--verify-allow=both",
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

/// A guest that reports its finding through output and exits 0, which Hermit rejects unless both statuses are allowed.
const EXITS_ZERO: &str = "[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: balance went negative'; exit 0; }; echo fine";

fn verify_calls(rig: &Rig) -> Vec<String> {
    rig.calls()
        .into_iter()
        .filter(|c| c.contains("--verify-strict"))
        .collect()
}

#[test]
fn the_verify_asks_hermit_to_accept_either_exit_status() {
    let rig = rig("verify-allow-both", RigOptions::default());
    run(&rig);
    let calls = verify_calls(&rig);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("--verify-allow=both"), "{}", calls[0]);
    assert!(!calls[0].contains("--verify-allow=failure"), "{}", calls[0]);
}

#[test]
fn a_guest_that_exits_zero_can_be_verified() {
    let o = RigOptions {
        svc_script: Some(EXITS_ZERO.to_string()),
        ..RigOptions::default()
    };
    let rig = rig("verify-exit-zero", o);
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Pass, "{verify:?}");
    assert_eq!(finding_status(&checks), FindingStatus::Verified);
}

#[test]
fn a_guest_that_exits_non_zero_can_still_be_verified() {
    let rig = rig("verify-exit-nonzero", RigOptions::default());
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Pass, "{verify:?}");
    assert_eq!(finding_status(&checks), FindingStatus::Verified);
}

#[test]
fn a_verify_hermit_rejected_shows_its_reason_and_error_line() {
    let rig = rig("verify-rejected", RigOptions::default());
    rig.set("verify", "rejected");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    let text = detail(verify, "inconclusive");
    assert!(
        text.starts_with("Hermit could not run the verify"),
        "{text}"
    );
    assert!(text.contains("first_run_rejected(exit_code=1)"), "{text}");
    assert!(
        text.contains("Error: First run during --verify exited with code 1"),
        "{text}"
    );
    assert_ne!(finding_status(&checks), FindingStatus::Verified);
}

#[test]
fn a_verify_that_hit_an_infrastructure_error_shows_the_reason() {
    let rig = rig("verify-infra", RigOptions::default());
    rig.set("verify", "infra");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    let text = detail(verify, "inconclusive");
    assert!(
        text.contains("run_failed(detail=could not start the guest)"),
        "{text}"
    );
    assert!(text.contains("Error: could not start the guest"), "{text}");
}

#[test]
fn a_timed_out_verify_says_so_and_differs_from_a_hermit_failure() {
    let o = RigOptions {
        trial_timeout: Duration::from_secs(1),
        ..RigOptions::default()
    };
    let hung = rig("verify-hang", o);
    hung.set("verify", "hang");
    let checks = run(&hung);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    let timed_out = detail(verify, "inconclusive").to_string();
    assert!(timed_out.contains("timed out after 2 s"), "{timed_out}");
    assert!(!timed_out.contains("Hermit could not run"), "{timed_out}");

    let failed = rig("verify-fail-text", RigOptions::default());
    failed.set("verify", "rejected");
    let checks = run(&failed);
    let failed_text = detail(verify_of(&checks), "inconclusive").to_string();
    assert!(!failed_text.contains("timed out"), "{failed_text}");
    assert_ne!(timed_out, failed_text);
}

#[test]
fn a_malformed_result_is_inconclusive_with_a_clear_message() {
    let rig = rig("verify-malformed", RigOptions::default());
    rig.set("verify", "malformed");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    assert!(detail(verify, "inconclusive").contains("not JSON"));
}
