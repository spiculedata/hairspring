//! `hairspring test --sweep`: the whole trial budget runs past every failure, and the report states a rate.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-cli-sweep-{name}-{}-{}",
        std::process::id(),
        hairspring_store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hairspring(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("binary runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A sweep of the demo that fails at every seed from 7, with room in its step budget for every trial.
const FAILING: &[&str] = &[
    "test",
    "lock-lease",
    "--sweep",
    "--budget-steps",
    "50000000",
];

fn with<'a>(base: &[&'a str], more: &[&'a str]) -> Vec<&'a str> {
    base.iter().chain(more).copied().collect()
}

#[test]
fn a_sweep_runs_the_whole_trial_budget_past_its_failures_and_exits_1() {
    let d = scratch("fail");
    let o = hairspring(&d, &with(FAILING, &["--trials", "5", "--seed", "7"]));
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(s.contains("FAILED") && s.contains("lock-lease"), "{s}");
    assert!(s.contains("  sweep: 5 trials:"), "{s}");
    assert!(s.contains("  failing seeds: 7, 8"), "{s}");
    assert!(s.contains("Wilson 95% interval"), "{s}");
    assert!(!s.contains("first hit"), "{s}");
    let bundles = std::fs::read_dir(d.join(".hairspring/bundles"))
        .unwrap()
        .count();
    assert_eq!(bundles, 1, "one bundle, for the lowest failing seed");
}

#[test]
fn a_sweep_of_a_correct_target_exits_0_and_states_an_upper_bound_not_a_bare_rate() {
    let d = scratch("pass");
    let o = hairspring(
        &d,
        &[
            "test",
            "pubsub-atomic",
            "--sweep",
            "--trials",
            "3",
            "--seed",
            "1",
            "--budget-steps",
            "1000000",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(s.contains("PASSED"), "{s}");
    assert!(s.contains("  sweep: 3 trials: 3 pass, 0 violation"), "{s}");
    assert!(
        s.contains("  hits: 0/3 rated trials, exact one-sided 95% upper bound"),
        "{s}"
    );
}

#[test]
fn a_seed_range_names_the_trials_and_the_seeds_they_run() {
    let d = scratch("range");
    let o = hairspring(&d, &with(FAILING, &["--seeds", "7..10"]));
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(s.contains("sweeping lock-lease (seeds 7..10)"), "{s}");
    assert!(s.contains("  sweep: 3 trials:"), "{s}");
    assert!(s.contains("  failing seeds: 7, 8, 9\n"), "{s}");
}

#[test]
fn a_sweep_reports_where_it_ended_when_its_step_budget_cut_it_short() {
    let d = scratch("short");
    let o = hairspring(
        &d,
        &[
            "test",
            "pubsub-atomic",
            "--sweep",
            "--trials",
            "10",
            "--budget-steps",
            "50000",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(
        s.contains("note: ended after 1 of 10 trials, before its trial budget"),
        "{s}"
    );
}

#[test]
fn a_sweep_that_ran_every_trial_has_no_budget_note() {
    let d = scratch("full");
    let o = hairspring(&d, &with(FAILING, &["--trials", "3", "--seed", "7"]));
    assert!(!out(&o).contains("note: ended after"), "{}", out(&o));
}

#[test]
fn the_json_outcome_carries_the_sweep_summary_with_seeds_as_strings() {
    let d = scratch("json");
    let o = hairspring(
        &d,
        &with(
            FAILING,
            &["--trials", "4", "--seed", "7", "--format", "json"],
        ),
    );
    let v: serde_json::Value = serde_json::from_str(&out(&o)).expect("json output parses");
    let sweep = &v[0]["outcome"]["sweep"];
    assert_eq!(sweep["trials"], 4, "{v}");
    assert_eq!(sweep["first_failing_seeds"][0], "7");
    assert_eq!(v[0]["outcome"]["stopped_at_first_failure"], false);
}

#[test]
fn without_sweep_a_run_stops_at_its_first_failure_and_has_no_sweep_section() {
    let d = scratch("search");
    let o = hairspring(&d, &["test", "lock-lease", "--seed", "7"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!out(&o).contains("sweep:"), "{}", out(&o));
}

#[test]
fn trials_and_seeds_are_refused_without_sweep() {
    let d = scratch("needs-sweep");
    for args in [
        &["test", "lock-lease", "--trials", "5"][..],
        &["test", "lock-lease", "--seeds", "1..5"][..],
    ] {
        let o = hairspring(&d, args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(err(&o).contains("--sweep"), "{}", err(&o));
    }
}

#[test]
fn a_bad_seed_range_or_trial_count_is_refused_before_anything_runs() {
    let d = scratch("bad-range");
    let bad: [&[&str]; 8] = [
        &["--seeds", "10..7"],
        &["--seeds", "7..7"],
        &["--seeds", "abc"],
        &["--seeds", "1..10002"],
        &["--seeds", "1..5", "--seed", "3"],
        &["--seeds", "1..5", "--trials", "2"],
        &["--trials", "0"],
        &["--trials", "10001"],
    ];
    for extra in bad {
        let o = hairspring(&d, &with(&["test", "lock-lease", "--sweep"], extra));
        assert_eq!(
            o.status.code(),
            Some(2),
            "{extra:?}: {}{}",
            out(&o),
            err(&o)
        );
        assert!(out(&o).is_empty(), "nothing ran: {extra:?}");
    }
}
