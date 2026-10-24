//! A run that panics on a PMU skid overshoot is infrastructure, not a verdict. A confirmation retries it once at the
//! same flags, then once at double the margin, and says so. A rerun at a higher margin is a different schedule: it
//! cannot count toward the reruns, but it can count for the control, with the caveat recorded.
mod common;
use common::confirm::*;
use hairspring_engine::confirm::*;
use hairspring_engine::scheduler::SchedulerSpec;

const BASE_MARGIN: &str = "25";

fn skid_rig(name: &str) -> Rig {
    let o = RigOptions {
        scheduler: SchedulerSpec {
            skid_margin: Some(25),
            ..SchedulerSpec::default()
        },
        control_script: "echo fine".to_string(),
        ..RigOptions::default()
    };
    let rig = rig(name, o);
    rig.set("skid_base", BASE_MARGIN);
    rig
}

fn margins(rig: &Rig) -> Vec<String> {
    rig.calls()
        .iter()
        .map(|call| {
            call.split(' ')
                .find_map(|a| a.strip_prefix("--skid-margin="))
                .unwrap_or("none")
                .to_string()
        })
        .collect()
}

fn one_rerun() -> ConfirmRequest {
    ConfirmRequest {
        reruns: 1,
        ..ConfirmRequest::default()
    }
}

fn detail<'a>(check: &'a CheckResult, key: &str) -> &'a str {
    check.detail.get(key).map_or("", String::as_str)
}

#[test]
fn an_overshoot_is_retried_once_at_the_same_flags_and_then_counts() {
    let rig = skid_rig("skid-retry");
    let bundle = rig.bundle();
    rig.set("skid_left", "1");
    let checks = rig
        .engine
        .confirm(&bundle, &one_rerun(), &mut |_| true)
        .unwrap();
    let rerun = &checks[0];
    assert_eq!(rerun.status, CheckStatus::Pass, "{rerun:?}");
    assert_eq!(rerun.counts.passed, 1);
    assert_eq!(detail(rerun, "skid_retries"), "1");
    assert_eq!(detail(rerun, "skid_margin_doubled"), "");
    // The replay is a separate check; the rerun's own calls are the first two.
    assert_eq!(&margins(&rig)[..2], [BASE_MARGIN, BASE_MARGIN]);
}

#[test]
fn a_second_overshoot_is_retried_at_double_the_margin_and_makes_check_one_inconclusive() {
    let rig = skid_rig("skid-double");
    let bundle = rig.bundle();
    rig.set("skid_left", "2");
    let checks = rig
        .engine
        .confirm(&bundle, &one_rerun(), &mut |_| true)
        .unwrap();
    let rerun = &checks[0];
    assert_eq!(&margins(&rig)[..3], [BASE_MARGIN, BASE_MARGIN, "50"]);
    assert_eq!(rerun.status, CheckStatus::Inconclusive, "{rerun:?}");
    assert_eq!(rerun.counts.passed, 0);
    assert_eq!(rerun.counts.inconclusive, 1);
    assert_eq!(detail(rerun, "skid_margin_doubled"), "true");
    assert!(
        detail(rerun, "caveat").contains("different schedule"),
        "{:?}",
        rerun.detail
    );
}

#[test]
fn an_overshoot_that_persists_at_double_the_margin_is_inconclusive_infra() {
    let rig = skid_rig("skid-hopeless");
    let bundle = rig.bundle();
    rig.set("skid_left", "100");
    rig.set("skid_double_fails", "1");
    let checks = rig
        .engine
        .confirm(&bundle, &one_rerun(), &mut |_| true)
        .unwrap();
    assert_eq!(checks[0].status, CheckStatus::Inconclusive);
    assert_eq!(&margins(&rig)[..3], [BASE_MARGIN, BASE_MARGIN, "50"]);
    assert!(detail(&checks[0], "inconclusive").contains("skid"));
    assert_eq!(finding_status(&checks), FindingStatus::Candidate);
}

#[test]
fn a_higher_skid_run_counts_for_the_control_with_the_caveat_recorded() {
    let o = RigOptions {
        scheduler: SchedulerSpec {
            skid_margin: Some(25),
            ..SchedulerSpec::default()
        },
        control_script: "echo fine # SKID_TARGET".to_string(),
        ..RigOptions::default()
    };
    let rig = rig("skid-control", o);
    rig.set("skid_base", BASE_MARGIN);
    let bundle = rig.bundle();
    // Only the control's guest overshoots, at the base margin, twice.
    rig.set("skid_for", "SKID_TARGET");
    rig.set("skid_left", "2");
    let req = ConfirmRequest {
        reruns: 1,
        control: Some("fixed".into()),
        ..ConfirmRequest::default()
    };
    let checks = rig.engine.confirm(&bundle, &req, &mut |_| true).unwrap();
    let rerun = &checks[0];
    assert_eq!(rerun.status, CheckStatus::Pass, "the reruns are unaffected");
    let control = checks
        .iter()
        .find(|c| c.kind == CheckKind::FixedControl)
        .unwrap();
    assert_eq!(control.status, CheckStatus::Pass, "{control:?}");
    assert_eq!(detail(control, "skid_margin_doubled"), "true");
    assert!(detail(control, "caveat").contains("different schedule"));
    assert_eq!(margins(&rig).last().map(String::as_str), Some("50"));
}

#[test]
fn a_skid_overshoot_is_not_retried_once_the_caller_has_cancelled() {
    let rig = skid_rig("skid-cancel");
    let bundle = rig.bundle();
    rig.set("skid_left", "5");
    let checks = rig
        .engine
        .confirm(&bundle, &one_rerun(), &mut |_| false)
        .unwrap();
    assert_eq!(
        rig.calls().len(),
        1,
        "no retry after the cancel: {:?}",
        rig.calls()
    );
    assert_eq!(checks[0].status, CheckStatus::Inconclusive);
    assert_eq!(checks[1].status, CheckStatus::Skipped);
}

#[test]
fn a_skid_retry_is_given_only_what_is_left_of_the_wall_budget() {
    let rig = skid_rig("skid-budget");
    let bundle = rig.bundle();
    rig.set("count", "0");
    rig.set("skid_left", "5");
    // The first run overshoots at once; the retry would sleep five seconds, far past the two second budget.
    rig.set("sleep", "5");
    rig.set("sleep_from", "2");
    let req = ConfirmRequest {
        reruns: 1,
        wall_secs: 2,
        ..ConfirmRequest::default()
    };
    let started = std::time::Instant::now();
    rig.engine.confirm(&bundle, &req, &mut |_| true).unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_millis(4000),
        "{:?}",
        started.elapsed()
    );
}
