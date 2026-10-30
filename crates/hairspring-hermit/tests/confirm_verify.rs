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
    assert!(
        text.contains("first_run_rejected(exit_code=1, stderr_bytes=3003, stdout_bytes=128)"),
        "{text}"
    );
    assert!(
        text.contains("Error: First run during --verify exited with code 1"),
        "{text}"
    );
    assert_ne!(finding_status(&checks), FindingStatus::Verified);
}

#[test]
fn a_skid_overshoot_reads_the_infrastructure_error_not_a_null_reason() {
    let rig = rig("verify-skid", RigOptions::default());
    rig.set("verify", "skid");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    let text = detail(verify, "inconclusive");
    assert!(text.contains("skid_overshoot(count=2)"), "{text}");
    assert!(!text.contains("null"), "{text}");
}

#[test]
fn a_changed_host_file_is_worded_as_a_comparison_that_cannot_be_trusted() {
    let rig = rig("verify-host-input", RigOptions::default());
    rig.set("verify", "host_input");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    let text = detail(verify, "inconclusive");
    assert!(
        text.contains("host file the guest opened changed during run 2: /etc/hosts"),
        "{text}"
    );
    assert!(!text.contains("could not run the verify"), "{text}");
}

#[test]
fn a_run_that_failed_in_hermit_shows_which_run_and_why() {
    let rig = rig("verify-run-failed", RigOptions::default());
    rig.set("verify", "run_failed");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    let text = detail(verify, "inconclusive");
    assert!(
        text.contains(
            "run_failed(detail=the backend failed to start, failure=backend_failure, run=run1)"
        ),
        "{text}"
    );
    assert!(
        text.contains("Hermit said: Error: the backend failed to start"),
        "{text}"
    );
}

#[test]
fn a_verify_in_which_the_guest_did_not_fail_does_not_verify_the_finding() {
    let rig = rig("verify-healed", RigOptions::default());
    rig.set("verify", "heal");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Fail, "{verify:?}");
    assert!(
        detail(verify, "mismatch").contains("did not fail"),
        "{:?}",
        verify.detail
    );
    assert_ne!(finding_status(&checks), FindingStatus::Verified);
}

#[test]
fn a_verify_miss_on_a_changed_environment_is_inconclusive() {
    let rig = rig("verify-healed-env", RigOptions::default());
    rig.set("verify", "heal");
    let bundle = rig
        .bundle()
        .with_param("host.kernel_release", "0.0.0-recorded-elsewhere");
    let checks = rig
        .engine
        .confirm(&bundle, &strict(), &mut |_| true)
        .unwrap();
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    assert!(detail(verify, "inconclusive").contains("environment differs"));
}

#[test]
fn a_verify_that_fails_a_different_kind_of_way_does_not_verify() {
    let o = RigOptions {
        svc_script: Some(EXITS_ZERO.to_string()),
        ..RigOptions::default()
    };
    let rig = rig("verify-other-kind", o);
    let bundle = rig.bundle().with_param("failure_kind", "deadlock");
    let checks = rig
        .engine
        .confirm(&bundle, &strict(), &mut |_| true)
        .unwrap();
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Fail, "{verify:?}");
    assert!(
        detail(verify, "mismatch").contains("different kind"),
        "{:?}",
        verify.detail
    );
}

#[test]
fn a_result_file_missing_after_a_hermit_failure_names_the_error_line_once() {
    let rig = rig("verify-missing-fail", RigOptions::default());
    rig.set("verify", "missing_fail");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    let text = detail(verify, "inconclusive");
    assert_eq!(text.matches("Error: Hermit fell over").count(), 1, "{text}");
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

#[test]
fn a_timeout_is_a_timeout_even_when_the_target_treats_a_hang_as_the_bug() {
    let o = RigOptions {
        trial_timeout: Duration::from_secs(1),
        hang_is_violation: true,
        ..RigOptions::default()
    };
    let rig = rig("verify-hang-violation", o);
    rig.set("verify", "hang");
    let checks = run(&rig);
    let verify = verify_of(&checks);
    assert_eq!(verify.status, CheckStatus::Inconclusive, "{verify:?}");
    assert!(
        detail(verify, "inconclusive").contains("timed out after 2 s"),
        "{:?}",
        verify.detail
    );
}
