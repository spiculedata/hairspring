//! Findings from the code review of the scheduler, timeout, parallelism and fingerprint changes.
mod common;
use common::{engine, scratch, Opts, PASSTHROUGH};
use hairspring_core::bundle::Bundle;
use hairspring_engine::engine::RunRequest;
use hairspring_engine::report::RunStatus;
use hairspring_engine::scheduler::SchedulerSpec;

fn run_once(e: &hairspring_engine::engine::Engine) -> hairspring_engine::report::RunOutcome {
    e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap()
}

#[test]
fn a_hermit_usage_error_is_infrastructure_not_a_program_violation() {
    let usage = "#!/bin/sh\necho \"error: the argument '--chaos' cannot be used multiple times\" >&2\necho >&2\necho 'Usage: hermit run [OPTIONS] [PROGRAM] [ARGS]...' >&2\nexit 2\n";
    let out = run_once(&engine(
        &scratch("usage"),
        usage,
        "true",
        Opts::default(),
        &[],
    ));
    assert_eq!(out.status, RunStatus::Error, "{out:?}");
    assert!(out.failure.is_none());
    assert!(out.error.unwrap().to_lowercase().contains("usage"));
}

#[test]
fn a_pmu_skid_overshoot_is_infrastructure_and_the_panic_flag_is_always_passed_with_a_skid_margin() {
    let log = scratch("overshoot").join("argv.log");
    let fake = format!(
        "#!/bin/sh\necho \"$@\" >> {}\necho \"thread 'main' panicked at timer.rs:1: SKID_OVERSHOOT precise timer overshot its target\" >&2\nexit 101\n",
        log.display()
    );
    let scheduler = SchedulerSpec {
        chaos: true,
        skid_margin: Some(25),
        ..SchedulerSpec::default()
    };
    let o = Opts {
        scheduler,
        ..Opts::default()
    };
    let out = run_once(&engine(log.parent().unwrap(), &fake, "true", o, &[]));
    assert_eq!(out.status, RunStatus::Error, "{out:?}");
    assert!(out.error.unwrap().to_lowercase().contains("overshoot"));
    let argv = std::fs::read_to_string(&log).unwrap();
    assert!(
        argv.contains("--skid-margin=25") && argv.contains("--panic-on-rbc-overshoot"),
        "{argv}"
    );
}

#[test]
fn a_violation_in_a_batch_wins_over_a_budget_clipped_timeout_earlier_in_it() {
    // Seed 1 hangs and is cut by the 1 s wall budget; seed 2 (already run in the same batch) violates.
    let script = "[ \"$FAKE_SCHED_SEED\" = 1 ] && sleep 30; [ \"$FAKE_SCHED_SEED\" = 2 ] && { echo 'HAIRSPRING_VIOLATION: real bug'; exit 1; }; echo fine";
    let o = Opts {
        trials: 2,
        parallelism: 2,
        wall_secs: 1,
        ..Opts::default()
    };
    let out = run_once(&engine(&scratch("clip"), PASSTHROUGH, script, o, &[]));
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    assert!(out.failure.unwrap().message.contains("real bug"));
}

#[test]
fn doctor_does_not_compare_trials_that_were_killed_at_a_timeout() {
    let o = Opts {
        timeout_secs: Some(1),
        ..Opts::default()
    };
    let e = engine(&scratch("doctor"), PASSTHROUGH, "sleep 30", o, &[]);
    let err = e.doctor("svc", 1, 0).unwrap_err().to_string();
    assert!(err.contains("timed out"), "{err}");
}

const FAILS: &str = "echo 'HAIRSPRING_VIOLATION: boom'; exit 1";

#[test]
fn replay_refuses_a_bundle_recorded_with_different_hermit_args() {
    let with = Opts {
        hermit_args: vec!["--no-virtualize-cpuid".into()],
        ..Opts::default()
    };
    let recorded = run_once(&engine(&scratch("args1"), PASSTHROUGH, FAILS, with, &[]));
    let bundle = Bundle::decode(&recorded.failure.unwrap().bundle).unwrap();
    assert_eq!(
        bundle.params.get("hermit_args").map(String::as_str),
        Some("--no-virtualize-cpuid")
    );
    let other = engine(&scratch("args2"), PASSTHROUGH, FAILS, Opts::default(), &[]);
    let err = other.replay(&bundle).unwrap_err().to_string();
    assert!(err.contains("hermit_args"), "{err}");
}

#[test]
fn replaying_a_promoted_hang_where_hangs_are_no_longer_violations_says_so() {
    let promoted = Opts {
        timeout_secs: Some(1),
        hang_is_violation: true,
        ..Opts::default()
    };
    let recorded = run_once(&engine(
        &scratch("hang1"),
        PASSTHROUGH,
        "sleep 30",
        promoted,
        &[],
    ));
    let bundle = Bundle::decode(&recorded.failure.unwrap().bundle).unwrap();
    let relaxed = Opts {
        timeout_secs: Some(1),
        ..Opts::default()
    };
    let r = engine(&scratch("hang2"), PASSTHROUGH, "sleep 30", relaxed, &[])
        .replay(&bundle)
        .unwrap();
    assert!(!r.reproduced);
    assert!(
        r.steps
            .iter()
            .any(|s| s.label.contains("no longer treats a hang as a violation")),
        "{:?}",
        r.steps
    );
}

#[test]
fn a_rerun_that_cannot_start_is_reported_with_the_spawn_error() {
    // Seed 1 violates, then the fake removes its own execute bit, so every confirmation rerun fails to spawn.
    let fake = "#!/bin/sh\nchmod -x \"$0\"\nshift\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n";
    let out = run_once(&engine(
        &scratch("rerun-spawn"),
        fake,
        "echo 'HAIRSPRING_VIOLATION: real bug'; exit 1",
        Opts::default(),
        &[],
    ));
    let f = out.failure.as_ref().unwrap_or_else(|| panic!("{out:?}"));
    assert!(
        f.message.contains("inconclusive") && f.message.to_lowercase().contains("denied"),
        "the spawn error must reach the failure message: {}",
        f.message
    );
}
