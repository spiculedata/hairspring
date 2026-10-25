//! The fixed-version control: the same seed on a supplied fixed build, which is clean if the failure is gone. A control
//! that still fails is flagged, and one that could not run is never read as clean.
mod common;
use common::confirm::*;
use hairspring_engine::confirm::*;
use hairspring_engine::EngineError;

fn with_control() -> ConfirmRequest {
    ConfirmRequest {
        reruns: 1,
        control: Some("fixed".into()),
        ..ConfirmRequest::default()
    }
}

fn control_of(checks: &[CheckResult]) -> &CheckResult {
    checks
        .iter()
        .find(|c| c.kind == CheckKind::FixedControl)
        .expect("a control check was recorded")
}

fn detail<'a>(check: &'a CheckResult, key: &str) -> &'a str {
    check.detail.get(key).map_or("", String::as_str)
}

fn run(rig: &Rig) -> Vec<CheckResult> {
    let bundle = rig.bundle();
    rig.engine
        .confirm(&bundle, &with_control(), &mut |_| true)
        .unwrap()
}

#[test]
fn a_control_that_does_not_fail_at_the_seed_passes_and_says_only_that() {
    let rig = rig("control-clean", RigOptions::default());
    let checks = run(&rig);
    let control = control_of(&checks);
    assert_eq!(control.status, CheckStatus::Pass);
    assert_eq!(control.counts.passed, 1);
    let note = detail(control, "note");
    assert!(note.contains("does not fail at this seed"), "{note}");
    assert!(
        !note.to_lowercase().contains("same interleaving"),
        "a different build is a different schedule: {note}"
    );
    assert_eq!(detail(control, "control_target"), "fixed");
    assert!(
        control.command.contains("--sched-seed=3"),
        "{}",
        control.command
    );
    assert!(!control.environment.is_empty());
    assert_eq!(finding_status(&checks), FindingStatus::Confirmed);
}

#[test]
fn a_control_that_still_fails_is_flagged() {
    let o = RigOptions {
        control_script: SEEDED_BUG.to_string(),
        ..RigOptions::default()
    };
    let rig = rig("control-fails", o);
    let checks = run(&rig);
    let control = control_of(&checks);
    assert_eq!(control.status, CheckStatus::Fail);
    assert!(
        detail(control, "flag").contains("control failed"),
        "{:?}",
        control.detail
    );
    assert_eq!(finding_status(&checks), FindingStatus::ControlFailed);
}

#[test]
fn a_control_that_could_not_run_is_inconclusive_never_clean() {
    let o = RigOptions {
        control_script: "echo fine # INFRA_MARKER".to_string(),
        ..RigOptions::default()
    };
    let rig = rig("control-infra", o);
    let checks = run(&rig);
    let control = control_of(&checks);
    assert_eq!(control.status, CheckStatus::Inconclusive, "{control:?}");
    assert_eq!(control.counts.inconclusive, 1);
    assert_eq!(control.counts.passed, 0);
    assert_ne!(finding_status(&checks), FindingStatus::ControlFailed);
}

#[test]
fn a_control_run_under_other_flags_is_not_a_control() {
    let o = RigOptions {
        control_hermit_args: vec!["--chaos-target-races".to_string()],
        ..RigOptions::default()
    };
    let rig = rig("control-flags", o);
    let bundle = rig.bundle();
    let checks = rig
        .engine
        .confirm(&bundle, &with_control(), &mut |_| true)
        .unwrap();
    let control = control_of(&checks);
    assert_eq!(control.status, CheckStatus::Inconclusive);
    assert!(
        detail(control, "inconclusive").contains("flags"),
        "{:?}",
        control.detail
    );
    assert!(
        rig.calls()
            .iter()
            .all(|c| !c.contains("--chaos-target-races")),
        "nothing was run under the other flags"
    );
}

#[test]
fn a_control_that_is_not_a_process_target_is_refused() {
    let rig = rig("control-unknown", RigOptions::default());
    let bundle = rig.bundle();
    let req = ConfirmRequest {
        control: Some("no-such-build".into()),
        ..ConfirmRequest::default()
    };
    let err = rig
        .engine
        .confirm(&bundle, &req, &mut |_| true)
        .unwrap_err();
    assert!(matches!(err, EngineError::UnknownTarget(_)));
}
