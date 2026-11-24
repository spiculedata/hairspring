//! `run` through the backend against the fake VM programs, and the wording of everything it says.
mod common;
use common::{quick, tiny_bounds, Rig};
use hairspring_core::bundle::Bundle;
use hairspring_dhyve::commands::Refusal;
use hairspring_dhyve::error::InfraKind;
use hairspring_dhyve::host::Timings;
use hairspring_dhyve::{DhyveBackend, BACKEND_ID};
use hairspring_engine::backend::{Backend, RunRequest};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::report::{RunOutcome, RunStatus};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::trial_log::ReportedVerdict;
use std::collections::BTreeMap;
use std::time::Duration;

const BANNED: [&str; 14] = [
    "deterministic",
    "NON-DETERMINISTIC",
    "reproduced",
    "proves",
    "fork",
    "dhyve.director",
    "deterministic.",
    " -o ",
    "caused",
    "because",
    "due to",
    "leads to",
    "results in",
    "triggered",
];

fn spec(timeout_secs: u64) -> TargetSpec {
    TargetSpec {
        name: "demo".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/true".into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: Some(timeout_secs),
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
            pass_env: false,
            backend: Some(BACKEND_ID.into()),
        },
        budget: Budget::default(),
    }
}

fn budget(trials: u64) -> Budget {
    Budget {
        max_trials: trials,
        ..Budget::default()
    }
}

fn backend(rig: &Rig, timings: Timings) -> DhyveBackend {
    DhyveBackend::with_settings(rig.host.clone(), timings, tiny_bounds())
}

fn run(rig: &Rig, trials: u64, timings: Timings) -> RunOutcome {
    backend(rig, timings)
        .run(
            &spec(2),
            &RunRequest::new("demo", 100),
            budget(trials),
            &mut |_| true,
        )
        .unwrap()
}

fn assert_plain(text: &str) {
    for word in BANNED {
        assert!(!text.contains(word), "`{word}` in `{text}`");
    }
}

#[test]
fn a_run_of_passing_trials_passes_and_logs_one_report_per_trial_in_seed_order() {
    let rig = Rig::new("b-pass");
    rig.modes(&[""]);
    let out = run(&rig, 3, quick());
    assert_eq!(out.status, RunStatus::Passed, "{:?}", out.error);
    assert_eq!(
        (out.trials, out.steps_executed, out.inconclusive),
        (3, 3, 0)
    );
    let seeds: Vec<u64> = out.trial_log.iter().map(|t| t.seed).collect();
    assert_eq!(seeds, vec![100, 101, 102]);
    assert!(out
        .trial_log
        .iter()
        .all(|t| t.verdict == ReportedVerdict::Pass));
    assert!(out.failure.is_none() && !out.stopped_at_first_failure);
    assert!(rig.vms().is_empty());
}

#[test]
fn the_first_finding_stops_the_run_and_names_its_seed_and_backend() {
    let rig = Rig::new("b-fail");
    rig.modes(&["", "", "exit=1"]);
    let out = run(&rig, 5, quick());
    assert_eq!(out.status, RunStatus::Failed);
    assert_eq!(out.trials, 2);
    assert!(out.stopped_at_first_failure);
    let failure = out.failure.unwrap();
    assert_eq!(failure.backend, "dhyve");
    assert_eq!(failure.message, "exit code 1");
    let bundle = Bundle::decode(&failure.bundle).unwrap();
    assert_eq!((bundle.seed, bundle.backend.as_str()), (101, "dhyve"));
    assert!(bundle.params.contains_key("checkpoint_sha256"));
    assert_plain(&failure.message);
}

#[test]
fn an_infrastructure_failure_is_a_run_error_with_no_finding() {
    let rig = Rig::new("b-infra");
    rig.modes(&["", "stale_stop"]);
    let out = run(&rig, 3, quick());
    assert_eq!(out.status, RunStatus::Error);
    assert!(out.failure.is_none());
    assert_eq!(out.trials, 0);
    let text = out.error.unwrap();
    assert!(text.starts_with(InfraKind::ResumeNotTaken.text()), "{text}");
    assert_plain(&text);
}

#[test]
fn a_prepare_that_fails_is_a_run_error_too() {
    let rig = Rig::new("b-prep");
    rig.modes(&["no_connect"]);
    let out = run(
        &rig,
        3,
        Timings {
            connect_wait: Duration::from_millis(300),
            ..quick()
        },
    );
    assert_eq!(out.status, RunStatus::Error);
    assert!(out.error.unwrap().starts_with(InfraKind::NoConnect.text()));
}

#[test]
fn a_trial_that_hangs_is_counted_inconclusive_and_the_run_goes_on() {
    let rig = Rig::new("b-hang");
    rig.modes(&["", "hang", ""]);
    let timings = Timings {
        trial_cap: Duration::from_secs(1),
        ..quick()
    };
    let out = backend(&rig, timings)
        .run(
            &{
                let mut s = spec(1);
                s.name = "demo".into();
                s
            },
            &RunRequest::new("demo", 5),
            budget(2),
            &mut |_| true,
        )
        .unwrap();
    assert_eq!(out.status, RunStatus::Passed);
    assert_eq!((out.trials, out.inconclusive), (2, 1));
    assert_eq!(out.trial_log[0].verdict, ReportedVerdict::Inconclusive);
}

#[test]
fn a_caller_that_says_stop_ends_the_run_with_cancelled() {
    let rig = Rig::new("b-cancel");
    let out = backend(&rig, quick())
        .run(
            &spec(2),
            &RunRequest::new("demo", 1),
            budget(3),
            &mut |_| false,
        )
        .unwrap();
    assert_eq!(out.status, RunStatus::Error);
    assert_eq!(out.error.as_deref(), Some("cancelled"));
}

#[test]
fn a_target_that_is_not_a_process_is_refused() {
    let rig = Rig::new("b-kind");
    let mut s = spec(2);
    s.kind = TargetKind::Wasm {
        module: "m.wasm".into(),
    };
    let err = backend(&rig, quick())
        .run(&s, &RunRequest::new("demo", 1), budget(1), &mut |_| true)
        .unwrap_err();
    assert!(err.to_string().contains("wasm"), "{err}");
}

#[test]
fn nothing_the_backend_says_carries_a_banned_word_or_an_option_name() {
    for kind in [
        InfraKind::HostBusy,
        InfraKind::Settings,
        InfraKind::Refused,
        InfraKind::StartFailed,
        InfraKind::NoConnect,
        InfraKind::NoReady,
        InfraKind::ControlLost,
        InfraKind::ResumeNotTaken,
        InfraKind::StoppedAgain,
        InfraKind::LogBound,
        InfraKind::LowSpace,
        InfraKind::SerialMissing,
        InfraKind::NoExitStatus,
        InfraKind::InputFiles,
        InfraKind::Checkpoint,
        InfraKind::ShutdownFailed,
        InfraKind::PrepareTimeout,
        InfraKind::FileOperation,
    ] {
        assert_plain(kind.text());
    }
    assert_plain(&Refusal::KernelModule.to_string());
    assert_plain(&Refusal::NotAllowed.to_string());
    let backend = DhyveBackend::new();
    assert_plain(backend.display_name());
    let info = serde_json::to_string(&backend.info()).unwrap();
    assert_plain(&info);
    assert!(
        !info.contains("unproven"),
        "the class is not a public label: {info}"
    );
    for scenario in [
        "stale_stop",
        "other_line",
        "limit_open",
        "log_overrun",
        "no_uart",
        "no_exit",
    ] {
        let rig = Rig::new("b-words");
        rig.modes(&["", scenario]);
        let out = run(&rig, 1, quick());
        assert_plain(&out.error.unwrap_or_default());
    }
}

#[test]
fn a_target_that_says_a_hang_is_the_bug_still_gets_an_inconclusive_trial() {
    let rig = Rig::new("b-hang-bug");
    rig.modes(&["", "hang"]);
    let timings = Timings {
        trial_cap: Duration::from_secs(1),
        ..quick()
    };
    let mut target = spec(1);
    if let TargetKind::Process {
        hang_is_violation, ..
    } = &mut target.kind
    {
        *hang_is_violation = true;
    }
    let out = backend(&rig, timings)
        .run(&target, &RunRequest::new("demo", 5), budget(1), &mut |_| {
            true
        })
        .unwrap();
    assert_eq!(out.status, RunStatus::Passed);
    assert!(out.failure.is_none());
    assert_eq!((out.trials, out.inconclusive), (1, 1));
}

#[test]
fn the_checkpoint_is_removed_when_the_run_ends() {
    let rig = Rig::new("b-ckpt-gone");
    rig.modes(&[""]);
    let out = run(&rig, 1, quick());
    assert_eq!(out.status, RunStatus::Passed);
    assert!(rig.left("kern").is_empty(), "a checkpoint file is left");
    assert!(!rig.host.state_dir.join("ckpt/00000000T0001Z").exists());
}
