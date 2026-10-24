//! The confirmation pipeline on the Hermit backend, against a fake `hermit`: same-seed reruns compared on more than
//! output, the bundle replay with its environment check, and the guards on what a confirmation may carry.
mod common;
use common::confirm::*;
use hairspring_core::bundle::Bundle;
use hairspring_engine::confirm::*;
use hairspring_engine::EngineError;

fn confirm(rig: &Rig, bundle: &Bundle, req: ConfirmRequest) -> Vec<CheckResult> {
    rig.engine.confirm(bundle, &req, &mut |_| true).unwrap()
}

fn kinds(checks: &[CheckResult]) -> Vec<CheckKind> {
    checks.iter().map(|c| c.kind).collect()
}

fn detail<'a>(check: &'a CheckResult, key: &str) -> &'a str {
    check.detail.get(key).map_or("", String::as_str)
}

#[test]
fn matching_reruns_and_a_replay_confirm_the_failure() {
    let rig = rig("pipe-ok", RigOptions::default());
    let bundle = rig.bundle();
    let checks = confirm(&rig, &bundle, ConfirmRequest::default());
    assert_eq!(
        kinds(&checks),
        [CheckKind::SameSeedRerun, CheckKind::BundleReplay]
    );
    let rerun = &checks[0];
    assert_eq!(rerun.status, CheckStatus::Pass);
    assert_eq!(rerun.counts.passed, 3);
    assert_eq!((rerun.counts.failed, rerun.counts.inconclusive), (0, 0));
    assert!(
        rerun.command.contains("--sched-seed=3"),
        "{}",
        rerun.command
    );
    assert!(
        !rerun.command.contains("--summary-json"),
        "{}",
        rerun.command
    );
    assert!(rerun.artefacts.contains(&bundle.params["output_hash"]));
    assert!(rerun
        .artefacts
        .contains(&bundle.params["schedule_fingerprint"]));
    assert!(!rerun.environment.is_empty());
    assert!(rerun.finished_at_ms >= rerun.started_at_ms);
    let replay = &checks[1];
    assert_eq!(replay.status, CheckStatus::Pass);
    assert_eq!(detail(replay, "environment_changed"), "false");
    assert_eq!(finding_status(&checks), FindingStatus::Confirmed);
    assert!(environment_pinned(&checks));
    assert_eq!(rig.calls().len(), 4, "three reruns and one replay");
}

#[test]
fn reruns_are_counted_as_asked() {
    let rig = rig("pipe-n", RigOptions::default());
    let bundle = rig.bundle();
    let req = ConfirmRequest {
        reruns: 5,
        ..ConfirmRequest::default()
    };
    let checks = confirm(&rig, &bundle, req);
    assert_eq!(checks[0].counts.passed, 5);
    assert_eq!(rig.calls().len(), 6);
}

#[test]
fn a_rerun_with_equal_output_but_a_different_schedule_fails_check_one() {
    let rig = rig("pipe-fp", RigOptions::default());
    let bundle = rig.bundle();
    rig.set("mode", "fpvary");
    let checks = confirm(&rig, &bundle, ConfirmRequest::default());
    let rerun = &checks[0];
    assert_eq!(rerun.status, CheckStatus::Fail);
    assert_eq!(rerun.counts.failed, 3);
    assert!(
        detail(rerun, "mismatch").contains("schedule fingerprint"),
        "{:?}",
        rerun.detail
    );
    assert_eq!(finding_status(&checks), FindingStatus::NotReproducible);
}

#[test]
fn reruns_that_no_longer_fail_fail_check_one() {
    let rig = rig("pipe-pass", RigOptions::default());
    let bundle = rig.bundle().with_param("sched_seed", "1");
    let checks = confirm(&rig, &bundle, ConfirmRequest::default());
    assert_eq!(checks[0].status, CheckStatus::Fail);
    assert!(detail(&checks[0], "mismatch").contains("did not fail"));
    assert_eq!(checks[1].status, CheckStatus::Fail);
    assert_eq!(finding_status(&checks), FindingStatus::NotReproducible);
}

#[test]
fn a_miss_on_a_changed_environment_is_inconclusive_and_named_as_such() {
    let rig = rig("pipe-env", RigOptions::default());
    let bundle = rig
        .bundle()
        .with_param("sched_seed", "1")
        .with_param("host.kernel_release", "0.0.0-recorded-elsewhere");
    let checks = confirm(&rig, &bundle, ConfirmRequest::default());
    for check in &checks {
        assert_eq!(check.status, CheckStatus::Inconclusive, "{check:?}");
        assert!(
            detail(check, "inconclusive").contains("environment differs"),
            "{:?}",
            check.detail
        );
    }
    assert_eq!(detail(&checks[1], "environment_changed"), "true");
    assert!(detail(&checks[1], "environment_differs").contains("host.kernel_release"));
    assert_eq!(finding_status(&checks), FindingStatus::Candidate);
    assert!(!environment_pinned(&checks));
}

#[test]
fn a_changed_loader_is_recorded_as_inconclusive_without_running_anything() {
    let rig = rig("pipe-loader", RigOptions::default());
    let bundle = rig
        .bundle()
        .with_param("host.loader.libc.sha256", "ab".repeat(32));
    let checks = confirm(&rig, &bundle, ConfirmRequest::default());
    assert_eq!(
        kinds(&checks),
        [CheckKind::SameSeedRerun, CheckKind::BundleReplay]
    );
    for check in &checks {
        assert_eq!(check.status, CheckStatus::Inconclusive, "{check:?}");
        assert!(detail(check, "inconclusive").contains("environment differs"));
    }
    assert!(rig.calls().is_empty(), "{:?}", rig.calls());
}

#[test]
fn a_changed_loader_can_be_allowed_and_the_difference_is_still_recorded() {
    let rig = rig("pipe-loader-ok", RigOptions::default());
    let bundle = rig
        .bundle()
        .with_param("host.loader.libc.sha256", "ab".repeat(32));
    let req = ConfirmRequest {
        allow_environment_change: true,
        ..ConfirmRequest::default()
    };
    let checks = confirm(&rig, &bundle, req);
    assert_eq!(checks[0].status, CheckStatus::Pass);
    assert_eq!(detail(&checks[1], "environment_changed"), "true");
    assert!(!environment_pinned(&checks));
}

#[test]
fn a_target_that_turns_on_hermit_logging_cannot_be_confirmed() {
    let o = RigOptions {
        hermit_args: vec!["--log=debug".to_string()],
        ..RigOptions::default()
    };
    let rig = rig("pipe-log", o);
    let bundle = rig.bundle();
    let err = rig
        .engine
        .confirm(&bundle, &ConfirmRequest::default(), &mut |_| true)
        .unwrap_err();
    assert!(
        matches!(&err, EngineError::InvalidRequest(m) if m.contains("--log")),
        "{err}"
    );
    assert!(rig.calls().is_empty());
}

#[test]
fn hermit_logging_in_the_hermit_environment_cannot_be_confirmed() {
    let o = RigOptions {
        extra_env: vec![("HERMIT_LOG".to_string(), "debug".to_string())],
        ..RigOptions::default()
    };
    let rig = rig("pipe-logenv", o);
    let bundle = rig.bundle();
    let err = rig
        .engine
        .confirm(&bundle, &ConfirmRequest::default(), &mut |_| true)
        .unwrap_err();
    assert!(
        matches!(&err, EngineError::InvalidRequest(m) if m.contains("HERMIT_LOG")),
        "{err}"
    );
}

#[test]
fn hermit_logging_in_the_targets_own_environment_cannot_be_confirmed() {
    let o = RigOptions {
        guest_env: vec![("HERMIT_LOG".to_string(), "debug".to_string())],
        ..RigOptions::default()
    };
    let rig = rig("pipe-guestlog", o);
    let bundle = rig.bundle();
    let err = rig
        .engine
        .confirm(&bundle, &ConfirmRequest::default(), &mut |_| true)
        .unwrap_err();
    assert!(
        matches!(&err, EngineError::InvalidRequest(m) if m.contains("HERMIT_LOG")),
        "{err}"
    );
    assert!(rig.calls().is_empty(), "nothing ran: {:?}", rig.calls());
}

#[test]
fn a_target_environment_that_only_looks_like_logging_is_allowed() {
    let o = RigOptions {
        guest_env: vec![("HERMIT_LOGGING".to_string(), "x".to_string())],
        ..RigOptions::default()
    };
    let rig = rig("pipe-guestlogok", o);
    let bundle = rig.bundle();
    assert!(rig
        .engine
        .confirm(&bundle, &ConfirmRequest::default(), &mut |_| true)
        .is_ok());
}

#[test]
fn a_confirmation_never_passes_logging_flags_to_hermit() {
    let rig = rig("pipe-nolog", RigOptions::default());
    let bundle = rig.bundle();
    let req = ConfirmRequest {
        strict_verify: true,
        control: Some("fixed".into()),
        ..ConfirmRequest::default()
    };
    confirm(&rig, &bundle, req);
    for call in rig.calls() {
        assert!(
            !call
                .split(' ')
                .any(|a| a.starts_with("--log") || a.starts_with("-v ") || a == "-v"),
            "{call}"
        );
    }
}

#[test]
fn a_cancelled_confirmation_stops_and_skips_what_did_not_run() {
    let rig = rig("pipe-cancel", RigOptions::default());
    let bundle = rig.bundle();
    let mut calls = 0;
    let checks = rig
        .engine
        .confirm(&bundle, &ConfirmRequest::default(), &mut |_| {
            calls += 1;
            calls < 2
        })
        .unwrap();
    assert_eq!(checks[0].status, CheckStatus::Inconclusive);
    assert!(checks[0].counts.total() < 3);
    assert_eq!(detail(&checks[0], "stopped"), "cancelled");
    assert_eq!(checks[1].kind, CheckKind::BundleReplay);
    assert_eq!(checks[1].status, CheckStatus::Skipped);
    assert_eq!(detail(&checks[1], "skipped"), "cancelled");
    assert!(rig.calls().len() <= 2, "{:?}", rig.calls());
}

#[test]
fn a_confirmation_that_runs_out_of_time_skips_the_rest() {
    let rig = rig("pipe-wall", RigOptions::default());
    let bundle = rig.bundle();
    rig.set("sleep", "1");
    let req = ConfirmRequest {
        wall_secs: 1,
        ..ConfirmRequest::default()
    };
    let checks = confirm(&rig, &bundle, req);
    assert_eq!(checks[0].status, CheckStatus::Inconclusive);
    assert!(checks[0].counts.total() < 3, "{:?}", checks[0].counts);
    assert_eq!(detail(&checks[0], "stopped"), "wall budget exhausted");
    assert_eq!(checks[1].status, CheckStatus::Skipped);
    assert_eq!(detail(&checks[1], "skipped"), "wall budget exhausted");
}
