//! Conformance: run the C SDK's sample programs and fold what they write with the reference parser.
//!
//! The C side builds with `make -C sdks/c fixtures`; these tests then check, against the parser that the engine
//! uses, that the SDK's output is complete, undegraded and gives the verdicts the plan requires.

use hairspring_props::{
    AssertKind, FailureReason, PropertyStat, PropertyStatus, RunProps, TrialEvents, TrialProps,
};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Once;

static BUILD: Once = Once::new();

fn sdk_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdks/c")
}

/// `HS_C_SDK_BUILD` names a directory of prebuilt `hs_sample_<scenario>` programs (for hosts without `make`);
/// otherwise they are built into `sdks/c/build/conformance`.
fn prebuilt_dir() -> Option<PathBuf> {
    std::env::var_os("HS_C_SDK_BUILD").map(PathBuf::from)
}

fn build_dir() -> PathBuf {
    prebuilt_dir().unwrap_or_else(|| sdk_dir().join("build/conformance"))
}

/// Build the sample programs once per test binary. Fails loudly if `make` or a C compiler is missing.
fn build_samples() {
    BUILD.call_once(|| {
        if prebuilt_dir().is_some() {
            return;
        }
        let status = Command::new("make")
            .arg("-s")
            .arg("-C")
            .arg(sdk_dir())
            .arg(format!("BUILD={}", build_dir().display()))
            .arg("fixtures")
            .status()
            .expect("make is required to build the C SDK fixtures");
        assert!(status.success(), "building sdks/c fixtures failed");
    });
}

fn sample(scenario: &str) -> Command {
    build_samples();
    Command::new(build_dir().join(format!("hs_sample_{scenario}")))
}

/// Run a command with stdout discarded and return what it wrote to stderr.
fn stderr_of(mut cmd: Command) -> (Vec<u8>, std::process::ExitStatus) {
    let out = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("sample program runs");
    (out.stderr, out.status)
}

fn fold(bytes: &[u8]) -> TrialProps {
    TrialEvents::from_output(bytes).fold()
}

fn run_trial(scenario: &str, args: &[&str]) -> TrialProps {
    let mut cmd = sample(scenario);
    cmd.args(args);
    fold(&stderr_of(cmd).0)
}

fn stats(trial: &TrialProps) -> Vec<PropertyStat> {
    RunProps::new().add(1, trial).finish()
}

fn status_of(stats: &[PropertyStat], id: &str) -> PropertyStatus {
    stats
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("no stat for {id}"))
        .status
}

#[test]
fn basic_report_is_complete_and_undegraded() {
    let trial = run_trial("basic", &[]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.report_missing().is_empty());
    assert_eq!(trial.properties.len(), 7);
    assert_eq!(trial.total_work(), 300);
    assert!(!trial.perturbing);
    let p = &trial.procs[0];
    assert_eq!(p.tag, "p1");
    assert_eq!(p.sdk.as_deref(), Some("c"));
    assert_eq!(p.signal_handlers, Some(false));
    let end = p.end.as_ref().expect("end line");
    assert_eq!(end.sites, 7);
    assert_eq!(end.threads, 1);
}

#[test]
fn an_always_that_was_false_is_a_violation_with_its_witness() {
    let trial = run_trial("basic", &[]);
    let failures = trial.verdict();
    assert_eq!(failures.len(), 1, "{failures:?}");
    let f = &failures[0];
    assert_eq!(f.id, "basic.always.bad");
    assert_eq!(f.reason, FailureReason::AlwaysFalse);
    assert_eq!(f.kind, AssertKind::Always);
    let w = f.witness.as_ref().expect("first_false witness");
    assert_eq!(w.vals, [7, 8, 0, 0]);
    assert!(w.thread.starts_with("t#"));
    assert!(w.ord >= 1);
    assert_eq!(
        status_of(&stats(&trial), "basic.always.bad"),
        PropertyStatus::Violation
    );
}

#[test]
fn a_reachable_that_is_never_hit_is_unmet_and_the_rest_pass() {
    let s = stats(&run_trial("basic", &[]));
    assert_eq!(status_of(&s, "basic.reach.never"), PropertyStatus::Unmet);
    assert_eq!(status_of(&s, "basic.reach.hit"), PropertyStatus::Pass);
    assert_eq!(status_of(&s, "basic.always.ok"), PropertyStatus::Pass);
    assert_eq!(status_of(&s, "basic.sometimes"), PropertyStatus::Pass);
    assert_eq!(status_of(&s, "basic.unreach.clean"), PropertyStatus::Pass);
    assert_eq!(status_of(&s, "basic.aou.never"), PropertyStatus::Pass);
}

#[test]
fn one_id_at_two_call_sites_is_one_property_with_summed_counts() {
    let trial = run_trial("merge", &[]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert_eq!(trial.health.duplicates, 0);
    let shared = trial
        .properties
        .iter()
        .find(|p| p.id == "merge.shared")
        .unwrap();
    assert_eq!((shared.hits, shared.trues, shared.falses), (4, 3, 1));
    assert_eq!(trial.procs[0].end.as_ref().unwrap().sites, 2);
}

#[test]
fn fork_children_report_under_derived_tags() {
    let trial = run_trial("fork", &[]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.report_missing().is_empty());
    let mut tags: Vec<&str> = trial.procs.iter().map(|p| p.tag.as_str()).collect();
    tags.sort_unstable();
    assert_eq!(tags, ["p1", "p1.f1", "p1.f2"]);
    // Parent evaluations are not double counted by the children: 2 parent hits + 2 * 3 child hits.
    let parent = trial
        .properties
        .iter()
        .find(|p| p.id == "fork.parent")
        .unwrap();
    let child = trial
        .properties
        .iter()
        .find(|p| p.id == "fork.child")
        .unwrap();
    assert_eq!((parent.hits, child.hits), (2, 6));
}

#[test]
fn more_threads_than_the_old_end_line_could_hold_report_cleanly() {
    let trial = run_trial("threads", &["100"]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert_eq!(trial.thread_reports().len(), 101);
    assert_eq!(trial.procs[0].end.as_ref().unwrap().threads, 101);
    assert!(trial
        .thread_reports()
        .iter()
        .any(|(_, t)| t.thread == "w-99" && t.key_stable));
}

#[test]
fn threads_beyond_the_table_are_capped_and_counted_without_degrading() {
    let trial = run_trial("threads", &["300"]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    let end = trial.procs[0].end.as_ref().unwrap();
    assert_eq!(end.threads, 256);
    assert!(end.dropped > 0);
    assert_eq!(trial.thread_reports().len(), 256);
}

#[test]
fn a_crash_without_signal_handlers_leaves_a_catalog_and_no_end() {
    let (bytes, status) = stderr_of(sample("crash"));
    assert!(!status.success());
    let trial = fold(&bytes);
    assert_eq!(trial.report_missing(), ["p1"]);
    assert_eq!(trial.properties.len(), 2);
    assert!(trial.properties.iter().all(|p| p.hits == 0));
}

#[test]
fn a_crash_with_signal_handlers_still_reports() {
    let mut cmd = sample("crash");
    cmd.env("HAIRSPRING_SIGNAL_REPORT", "1");
    let (bytes, status) = stderr_of(cmd);
    assert!(!status.success());
    let trial = fold(&bytes);
    assert!(trial.report_missing().is_empty());
    assert_eq!(trial.procs[0].signal_handlers, Some(true));
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    let reached = trial
        .properties
        .iter()
        .find(|p| p.id == "crash.reach")
        .unwrap();
    assert_eq!(reached.hits, 1);
}

#[test]
fn hs_exit_records_the_status() {
    let trial = run_trial("exit_status", &[]);
    assert_eq!(trial.procs[0].end.as_ref().unwrap().status, 3);
}

#[test]
fn eval_lines_mark_the_trial_perturbing() {
    let trial = run_trial("eval", &[]);
    assert!(trial.perturbing);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
}

#[test]
fn two_processes_on_one_pipe_fold_cleanly() {
    // macOS pipes are only atomic up to 512 bytes; the report's long lines are exercised on Linux.
    let scenario = if cfg!(target_os = "linux") {
        "big"
    } else {
        "small"
    };
    for _ in 0..10 {
        build_samples();
        let bin = build_dir().join(format!("hs_sample_{scenario}"));
        let script = format!(
            "HAIRSPRING_PROC=pA {bin} & HAIRSPRING_PROC=pB {bin} & wait",
            bin = bin.display()
        );
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        let trial = fold(&stderr_of(cmd).0);
        assert!(!trial.is_degraded(), "{:?}", trial.health);
        assert_eq!(trial.health.malformed, 0);
        assert!(trial.report_missing().is_empty());
        let mut tags: Vec<&str> = trial.procs.iter().map(|p| p.tag.as_str()).collect();
        tags.sort_unstable();
        assert_eq!(tags, ["pA", "pB"]);
        assert_eq!(trial.properties.len(), 64);
        assert!(trial.properties.iter().all(|p| p.hits == 6));
    }
}
