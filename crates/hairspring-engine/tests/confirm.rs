//! The confirmation pipeline's vocabulary: check results, the finding state derived from them, the request limits and
//! the logging guard.
use hairspring_engine::confirm::*;
use hairspring_props::claim_text::FindingState;
use std::collections::BTreeMap;

fn check(kind: CheckKind, status: CheckStatus) -> CheckResult {
    CheckResult::new(kind, status)
}

fn replay(status: CheckStatus, environment_changed: bool) -> CheckResult {
    check(CheckKind::BundleReplay, status)
        .with_detail("environment_changed", environment_changed.to_string())
}

fn status_of(checks: &[CheckResult]) -> FindingStatus {
    finding_status(checks)
}

#[test]
fn a_failure_with_no_checks_is_a_candidate() {
    assert_eq!(status_of(&[]), FindingStatus::Candidate);
}

#[test]
fn matching_reruns_alone_make_a_failure_reproduced_not_confirmed() {
    let checks = [check(CheckKind::SameSeedRerun, CheckStatus::Pass)];
    assert_eq!(status_of(&checks), FindingStatus::Reproduced);
}

#[test]
fn reruns_and_a_replay_on_the_same_environment_confirm_it() {
    let checks = [
        check(CheckKind::SameSeedRerun, CheckStatus::Pass),
        replay(CheckStatus::Pass, false),
    ];
    assert_eq!(status_of(&checks), FindingStatus::Confirmed);
}

#[test]
fn a_passing_strict_verify_on_top_of_that_verifies_it() {
    let checks = [
        check(CheckKind::SameSeedRerun, CheckStatus::Pass),
        replay(CheckStatus::Pass, false),
        check(CheckKind::StrictVerify, CheckStatus::Pass),
    ];
    assert_eq!(status_of(&checks), FindingStatus::Verified);
}

#[test]
fn a_failed_strict_verify_leaves_it_confirmed() {
    let checks = [
        check(CheckKind::SameSeedRerun, CheckStatus::Pass),
        replay(CheckStatus::Pass, false),
        check(CheckKind::StrictVerify, CheckStatus::Fail),
    ];
    assert_eq!(status_of(&checks), FindingStatus::Confirmed);
}

#[test]
fn a_strict_verify_without_the_earlier_checks_verifies_nothing() {
    let checks = [check(CheckKind::StrictVerify, CheckStatus::Pass)];
    assert_eq!(status_of(&checks), FindingStatus::Candidate);
}

#[test]
fn reruns_that_do_not_match_make_it_not_reproducible() {
    let checks = [check(CheckKind::SameSeedRerun, CheckStatus::Fail)];
    assert_eq!(status_of(&checks), FindingStatus::NotReproducible);
}

#[test]
fn a_replay_that_missed_on_an_unchanged_environment_makes_it_not_reproducible() {
    let checks = [
        check(CheckKind::SameSeedRerun, CheckStatus::Pass),
        replay(CheckStatus::Fail, false),
    ];
    assert_eq!(status_of(&checks), FindingStatus::NotReproducible);
}

#[test]
fn a_replay_inconclusive_because_the_environment_differs_is_not_a_miss() {
    let checks = [
        check(CheckKind::SameSeedRerun, CheckStatus::Pass),
        replay(CheckStatus::Inconclusive, true),
    ];
    assert_eq!(status_of(&checks), FindingStatus::Reproduced);
}

#[test]
fn inconclusive_reruns_prove_nothing() {
    let checks = [check(CheckKind::SameSeedRerun, CheckStatus::Inconclusive)];
    assert_eq!(status_of(&checks), FindingStatus::Candidate);
}

#[test]
fn a_control_that_still_fails_flags_the_finding() {
    let checks = [
        check(CheckKind::SameSeedRerun, CheckStatus::Pass),
        replay(CheckStatus::Pass, false),
        check(CheckKind::FixedControl, CheckStatus::Fail),
    ];
    assert_eq!(status_of(&checks), FindingStatus::ControlFailed);
}

#[test]
fn a_clean_or_inconclusive_control_does_not_change_the_state() {
    for control in [CheckStatus::Pass, CheckStatus::Inconclusive] {
        let checks = [
            check(CheckKind::SameSeedRerun, CheckStatus::Pass),
            replay(CheckStatus::Pass, false),
            check(CheckKind::FixedControl, control),
        ];
        assert_eq!(status_of(&checks), FindingStatus::Confirmed, "{control:?}");
    }
}

#[test]
fn the_latest_check_of_a_kind_wins() {
    let checks = [
        check(CheckKind::SameSeedRerun, CheckStatus::Fail),
        check(CheckKind::SameSeedRerun, CheckStatus::Pass),
    ];
    assert_eq!(status_of(&checks), FindingStatus::Reproduced);
}

#[test]
fn only_confirmed_and_verified_findings_reach_the_claim_ladder() {
    let claimable = |s: FindingStatus| s.claim_state();
    assert_eq!(claimable(FindingStatus::Confirmed), FindingState::Confirmed);
    assert_eq!(claimable(FindingStatus::Verified), FindingState::Verified);
    for s in [
        FindingStatus::Candidate,
        FindingStatus::Reproduced,
        FindingStatus::NotReproducible,
        FindingStatus::ControlFailed,
    ] {
        assert_eq!(claimable(s), FindingState::SingleRun, "{s:?}");
    }
}

#[test]
fn the_environment_is_pinned_only_by_a_recorded_identity_that_still_matches() {
    let identity = |c: CheckResult| c.with_environment("host.container", "none");
    let pinned = [
        identity(check(CheckKind::SameSeedRerun, CheckStatus::Pass)),
        identity(replay(CheckStatus::Pass, false)),
    ];
    assert!(environment_pinned(&pinned));
    let unrecorded = [
        check(CheckKind::SameSeedRerun, CheckStatus::Pass),
        replay(CheckStatus::Pass, false),
    ];
    assert!(!environment_pinned(&unrecorded));
    let changed = [
        identity(check(CheckKind::SameSeedRerun, CheckStatus::Pass)),
        identity(replay(CheckStatus::Pass, true)),
    ];
    assert!(!environment_pinned(&changed));
    assert!(!environment_pinned(&[]));
}

#[test]
fn kinds_and_statuses_have_stable_wire_names() {
    for (kind, name) in [
        (CheckKind::SameSeedRerun, "same_seed_rerun"),
        (CheckKind::BundleReplay, "bundle_replay"),
        (CheckKind::FixedControl, "fixed_control"),
        (CheckKind::StrictVerify, "strict_verify"),
    ] {
        assert_eq!(kind.as_str(), name);
        assert_eq!(CheckKind::parse(name), Some(kind));
    }
    for (status, name) in [
        (CheckStatus::Pass, "pass"),
        (CheckStatus::Fail, "fail"),
        (CheckStatus::Inconclusive, "inconclusive"),
        (CheckStatus::Skipped, "skipped"),
    ] {
        assert_eq!(status.as_str(), name);
        assert_eq!(CheckStatus::parse(name), Some(status));
    }
    assert_eq!(CheckKind::parse("rerun"), None);
    assert_eq!(CheckStatus::parse("PASS"), None);
}

#[test]
fn a_result_serialises_with_its_wire_names() {
    let r = check(CheckKind::FixedControl, CheckStatus::Inconclusive);
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["kind"], "fixed_control");
    assert_eq!(v["status"], "inconclusive");
}

#[test]
fn a_result_bounds_everything_it_carries() {
    let long = "x".repeat(10_000);
    let mut r = check(CheckKind::SameSeedRerun, CheckStatus::Pass).with_command(long.clone());
    for i in 0..100 {
        r = r
            .with_detail(format!("key{i}-{long}"), long.clone())
            .with_artefact(long.clone())
            .with_environment(format!("name{i}"), long.clone());
    }
    let r = r.bounded();
    assert!(r.command.chars().count() <= MAX_COMMAND_CHARS);
    assert!(r.detail.len() <= MAX_DETAIL_ENTRIES);
    assert!(r
        .detail
        .iter()
        .all(|(k, v)| k.chars().count() <= MAX_DETAIL_KEY_CHARS
            && v.chars().count() <= MAX_DETAIL_VALUE_CHARS));
    assert!(r.artefacts.len() <= MAX_ARTEFACTS);
    assert!(r
        .artefacts
        .iter()
        .all(|a| a.chars().count() <= MAX_ARTEFACT_CHARS));
    assert!(r.environment.len() <= MAX_ENVIRONMENT_ENTRIES);
}

#[test]
fn bounding_strips_control_characters_from_text() {
    let r = check(CheckKind::BundleReplay, CheckStatus::Fail)
        .with_command("a\nb\u{202E}c")
        .with_detail("why", "line one\nline two\u{0007}")
        .bounded();
    assert_eq!(r.command, "a b c");
    assert_eq!(r.detail["why"], "line one line two ");
}

#[test]
fn the_default_request_runs_three_reruns_and_nothing_optional() {
    let r = ConfirmRequest::default().validated().unwrap();
    assert_eq!(r.reruns, DEFAULT_RERUNS);
    assert_eq!(DEFAULT_RERUNS, 3);
    assert!(r.control.is_none() && !r.strict_verify && !r.allow_environment_change);
}

#[test]
fn a_request_outside_the_limits_is_refused() {
    let with = |f: fn(&mut ConfirmRequest)| {
        let mut r = ConfirmRequest::default();
        f(&mut r);
        r.validated()
    };
    assert!(with(|r| r.reruns = 0).is_err());
    assert!(with(|r| r.reruns = MAX_RERUNS + 1).is_err());
    assert!(with(|r| r.reruns = MAX_RERUNS).is_ok());
    assert!(with(|r| r.wall_secs = 0).is_err());
    assert!(with(|r| r.wall_secs = MAX_WALL_SECS + 1).is_err());
    assert!(with(|r| r.control = Some(String::new())).is_err());
    assert!(with(|r| r.control = Some("x".repeat(65))).is_err());
    assert!(with(|r| r.control = Some("fixed-build".into())).is_ok());
}

#[test]
fn logging_options_are_refused_in_arguments_and_environment() {
    let none = BTreeMap::new();
    for bad in [
        "--log",
        "--log=debug",
        "--log=hermit=trace",
        "-v",
        "-vv",
        "--verbose",
    ] {
        let args = vec!["--chaos-target-races".to_string(), bad.to_string()];
        assert!(forbidden_logging(&args, &none).is_some(), "{bad}");
    }
    for fine in [
        "--chaos-target-races",
        "--logical-clock",
        "--base-env=minimal",
        "-x",
    ] {
        assert!(
            forbidden_logging(&[fine.to_string()], &none).is_none(),
            "{fine}"
        );
    }
    let env = BTreeMap::from([("HERMIT_LOG".to_string(), "debug".to_string())]);
    assert!(forbidden_logging(&[], &env).is_some());
    let other = BTreeMap::from([("HERMIT_LOGGING".to_string(), "x".to_string())]);
    assert!(forbidden_logging(&[], &other).is_none());
}
