//! Hermit's "Deadlock detected" report is its own finding, not an ordinary failing exit and not a data property.
mod common;
use common::{engine, scratch, Opts, PASSTHROUGH};
use hairspring_core::bundle::Bundle;
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::report::{RunOutcome, RunStatus};

/// The guest stands in for Hermit's own report: the line on stderr and a non-zero exit.
const DEADLOCKS: &str = "echo 'ERROR: Deadlock detected' >&2; exit 1";

fn run(e: &Engine) -> RunOutcome {
    e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap()
}

fn deadlocking(name: &str, script: &str, o: Opts) -> Engine {
    engine(&scratch(name), PASSTHROUGH, script, o, &[])
}

#[test]
fn a_deadlock_is_reported_as_a_deadlock_with_hermits_own_text() {
    let out = run(&deadlocking("dl-msg", DEADLOCKS, Opts::default()));
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    let message = out.failure.unwrap().message;
    assert!(message.starts_with("deadlock at sched-seed"), "{message}");
    assert!(message.contains("Deadlock detected"), "{message}");
    assert!(!message.contains("property violated"), "{message}");
}

#[test]
fn a_deadlock_is_a_finding_without_hang_is_violation_and_without_a_timeout() {
    let o = Opts {
        timeout_secs: Some(30),
        hang_is_violation: false,
        ..Opts::default()
    };
    let started = std::time::Instant::now();
    let out = run(&deadlocking("dl-fast", DEADLOCKS, o));
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    assert!(started.elapsed().as_secs() < 20);
}

#[test]
fn the_bundle_records_the_failure_kind_and_replays_as_a_reproduction() {
    let e = deadlocking("dl-bundle", DEADLOCKS, Opts::default());
    let out = run(&e);
    let bundle = Bundle::decode(&out.failure.unwrap().bundle).unwrap();
    assert_eq!(
        bundle.params.get("failure_kind").map(String::as_str),
        Some("deadlock")
    );
    assert!(!bundle.params.contains_key("property"));
    assert!(e.replay(&bundle).unwrap().reproduced);
}

#[test]
fn a_plain_failing_exit_stays_an_exit_code_violation() {
    let out = run(&deadlocking("dl-plain", "exit 1", Opts::default()));
    let message = out.failure.unwrap().message;
    assert!(message.contains("exit code 1"), "{message}");
    assert!(!message.contains("deadlock"), "{message}");
}

#[test]
fn deadlock_text_from_a_program_that_exits_cleanly_is_not_a_finding() {
    let out = run(&deadlocking(
        "dl-clean",
        "echo 'Deadlock detected' >&2",
        Opts::default(),
    ));
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
}
