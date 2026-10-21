//! Conformance: run the Python SDK's sample programs (`sdks/python/samples/hs_sample.py`) and the lost-update fixture
//! (`examples/language-fixtures/python/at_py.py`) and fold what they write with the reference parser, as `c_sdk.rs`
//! does for the C SDK. One test also compares the Python SDK's report with the C SDK's byte for byte.
//!
//! `HS_PYTHON` names the interpreter; without it the first Python 3.12 or newer on `PATH` is used. When there is none
//! the tests print a skip notice and pass, unless `HS_REQUIRE_PYTHON` is set, which makes a missing or too old
//! Python an error. Nothing is installed or built: the SDK is pure Python, found through `PYTHONPATH`.

use hairspring_props::{
    AssertKind, FailureReason, PropertyStat, PropertyStatus, RunProps, TrialEvents, TrialProps,
};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::OnceLock;

#[path = "common/python_toolchain.rs"]
mod python_toolchain;

/// The planted race fires on essentially every native run; this many tries make a miss vanishingly unlikely.
const RACY_ATTEMPTS: usize = 50;
const LOCKED_RUNS: usize = 5;
const MAX_LINE_BYTES: usize = 4096;

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sample_script() -> PathBuf {
    workspace_dir().join("sdks/python/samples/hs_sample.py")
}

fn fixture_script() -> PathBuf {
    workspace_dir().join("examples/language-fixtures/python/at_py.py")
}

/// The interpreter to use, or None when there is no usable Python (the test is skipped).
fn python() -> Option<&'static str> {
    static PYTHON: OnceLock<Option<String>> = OnceLock::new();
    PYTHON
        .get_or_init(|| {
            let found = python_toolchain::find_python();
            if found.is_none() {
                assert!(
                    !python_toolchain::require_python(),
                    "HS_REQUIRE_PYTHON is set but there is no Python 3.{} or newer on PATH (set HS_PYTHON)",
                    python_toolchain::MIN_PYTHON_MINOR
                );
                eprintln!(
                    "skip: no Python 3.{}+ (set HS_PYTHON, or HS_REQUIRE_PYTHON=1 to make this an error)",
                    python_toolchain::MIN_PYTHON_MINOR
                );
            }
            found
        })
        .as_deref()
}

fn base_command(python: &str, script: &Path) -> Command {
    let mut cmd = Command::new(python);
    cmd.arg("-B")
        .arg(script)
        .env("PYTHONPATH", workspace_dir().join("sdks/python/src"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env_remove("HAIRSPRING_PROC")
        .env_remove("HAIRSPRING_PROC_PID")
        .env_remove("HAIRSPRING_EVENTS")
        .env_remove("HAIRSPRING_EVENTS_FILE");
    cmd
}

/// A sample scenario ready to run, or None without a Python.
fn sample(scenario: &str) -> Option<Command> {
    let mut cmd = base_command(python()?, &sample_script());
    cmd.arg(scenario);
    Some(cmd)
}

/// The fixture in one of its modes (the fixture finds the SDK by itself and is started isolated, as the box will).
fn fixture(mode: &str) -> Option<Command> {
    let mut cmd = Command::new(python()?);
    cmd.arg("-I")
        .arg("-B")
        .arg(fixture_script())
        .arg(mode)
        .env_remove("HAIRSPRING_PROC")
        .env_remove("HAIRSPRING_PROC_PID")
        .env_remove("HAIRSPRING_EVENTS")
        .env_remove("HAIRSPRING_EVENTS_FILE");
    Some(cmd)
}

struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: ExitStatus,
}

fn run(mut cmd: Command) -> Output {
    let out = cmd.output().expect("python runs");
    Output {
        stdout: out.stdout,
        stderr: out.stderr,
        status: out.status,
    }
}

fn fold(bytes: &[u8]) -> TrialProps {
    TrialEvents::from_output(bytes).fold()
}

macro_rules! or_skip {
    ($command:expr) => {
        match $command {
            Some(cmd) => run(cmd),
            None => return,
        }
    };
}

fn trial(scenario: &str, args: &[&str]) -> Option<(TrialProps, Output)> {
    let mut cmd = sample(scenario)?;
    cmd.args(args);
    let out = run(cmd);
    Some((fold(&out.stderr), out))
}

macro_rules! trial_or_skip {
    ($scenario:expr $(, $arg:expr)*) => {
        match trial($scenario, &[$($arg),*]) {
            Some(t) => t,
            None => return,
        }
    };
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

fn tags(trial: &TrialProps) -> Vec<String> {
    let mut tags: Vec<String> = trial.procs.iter().map(|p| p.tag.clone()).collect();
    tags.sort();
    tags
}

// ---- the same checks as the C SDK's conformance tests ------------------------------------------------------------

#[test]
fn basic_report_is_complete_and_undegraded() {
    let (trial, out) = trial_or_skip!("basic");
    assert!(out.status.success());
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.report_missing().is_empty());
    assert_eq!(trial.properties.len(), 7);
    assert_eq!(trial.total_work(), 300);
    assert!(!trial.perturbing);
    let p = &trial.procs[0];
    assert_eq!(p.tag, "p1");
    assert_eq!(p.sdk.as_deref(), Some("python"));
    assert_eq!(p.signal_handlers, Some(false));
    let end = p.end.as_ref().expect("end line");
    assert_eq!(
        (end.sites, end.threads, end.evals, end.dropped),
        (7, 1, 11, 0)
    );
    assert!(out.stdout.is_empty(), "the SDK writes to stderr only");
}

#[test]
fn every_line_is_a_well_formed_event_within_the_cap() {
    let (_, out) = trial_or_skip!("big");
    let text = String::from_utf8(out.stderr).expect("utf-8 output");
    assert!(
        text.lines().count() > 100,
        "hello, 64 catalog, 64 assert, ..."
    );
    for line in text.lines() {
        assert!(line.len() < MAX_LINE_BYTES, "line of {} bytes", line.len());
        assert!(
            matches!(
                hairspring_props::parse_line(line.as_bytes()),
                hairspring_props::LineOutcome::Event {
                    unknown_fields: 0,
                    ..
                }
            ),
            "not a clean event: {line}"
        );
    }
}

#[test]
fn an_always_that_was_false_is_a_violation_with_its_witness() {
    let (trial, _) = trial_or_skip!("basic");
    let failures = trial.verdict();
    assert_eq!(failures.len(), 1, "{failures:?}");
    let f = &failures[0];
    assert_eq!(f.id, "basic.always.bad");
    assert_eq!(f.reason, FailureReason::AlwaysFalse);
    assert_eq!(f.kind, AssertKind::Always);
    let w = f.witness.as_ref().expect("first_false witness");
    assert_eq!(w.vals, [7, 8, 0, 0]);
    assert_eq!(w.thread, "t#1");
    assert_eq!(w.ord, 6);
    assert_eq!(
        status_of(&stats(&trial), "basic.always.bad"),
        PropertyStatus::Violation
    );
}

#[test]
fn a_reachable_that_is_never_hit_is_unmet_only_because_it_was_declared() {
    let (trial, _) = trial_or_skip!("basic");
    let s = stats(&trial);
    assert_eq!(status_of(&s, "basic.reach.never"), PropertyStatus::Unmet);
    assert_eq!(status_of(&s, "basic.reach.hit"), PropertyStatus::Pass);
    assert_eq!(status_of(&s, "basic.always.ok"), PropertyStatus::Pass);
    assert_eq!(status_of(&s, "basic.sometimes"), PropertyStatus::Pass);
    assert_eq!(status_of(&s, "basic.unreach.clean"), PropertyStatus::Pass);
    assert_eq!(status_of(&s, "basic.aou.never"), PropertyStatus::Pass);
}

#[test]
fn one_id_at_two_call_sites_is_one_property_with_summed_counts() {
    let (trial, _) = trial_or_skip!("merge");
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
fn more_threads_than_the_old_end_line_could_hold_report_cleanly() {
    let (trial, _) = trial_or_skip!("threads", "100");
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
    let (trial, _) = trial_or_skip!("threads", "300");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    let end = trial.procs[0].end.as_ref().unwrap();
    assert_eq!(end.threads, 256);
    assert!(end.dropped > 0);
    assert_eq!(trial.thread_reports().len(), 256);
}

#[test]
fn exit_records_the_status_and_the_process_exits_with_it() {
    let (trial, out) = trial_or_skip!("exit_status");
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(trial.procs[0].end.as_ref().unwrap().status, 3);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
}

/// Two instrumented processes writing to one output at once, as the engine's pipe would see them.
fn two_writers(use_file: bool, round: usize) -> Option<TrialProps> {
    let python = python()?;
    let events = std::env::temp_dir().join(format!(
        "hs_python_sdk_{}_{round}.events",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&events);
    let file_env = if use_file {
        format!("HAIRSPRING_EVENTS_FILE={} ", events.display())
    } else {
        String::new()
    };
    let script = format!(
        "{file_env}HAIRSPRING_PROC=pA {python} -B {s} big & {file_env}HAIRSPRING_PROC=pB {python} -B {s} big & wait",
        s = sample_script().display()
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("PYTHONPATH", workspace_dir().join("sdks/python/src"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdout(Stdio::null())
        .output()
        .expect("sh runs");
    let bytes = if use_file {
        let read = std::fs::read(&events).expect("events file written");
        let _ = std::fs::remove_file(&events);
        read
    } else {
        out.stderr
    };
    Some(fold(&bytes))
}

#[test]
fn two_processes_on_one_pipe_fold_cleanly() {
    // macOS pipes are only atomic up to 512 bytes, so there the two processes share an O_APPEND events file.
    for round in 0..5 {
        let Some(trial) = two_writers(!cfg!(target_os = "linux"), round) else {
            return;
        };
        assert!(!trial.is_degraded(), "{:?}", trial.health);
        assert_eq!(trial.health.malformed, 0);
        assert!(trial.report_missing().is_empty());
        assert_eq!(tags(&trial), ["pA", "pB"]);
        assert_eq!(trial.properties.len(), 64);
        assert!(trial.properties.iter().all(|p| p.hits == 6));
    }
}

#[test]
fn a_spawned_instrumented_child_does_not_reuse_its_parents_tag() {
    let (trial, _) = trial_or_skip!("spawn");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.report_missing().is_empty());
    let found = tags(&trial);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0], "p1");
    assert!(found[1].starts_with("p1.s"), "{found:?}");
}

#[test]
fn a_multiprocessing_spawn_child_reports_under_its_own_tag() {
    let (trial, _) = trial_or_skip!("mp_spawn");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.report_missing().is_empty());
    assert_eq!(tags(&trial).len(), 2);
}

#[test]
fn a_spawned_tag_says_it_is_not_stable() {
    let out = or_skip!(sample("spawn"));
    let hellos: Vec<String> = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("\"kind\":\"hello\""))
        .map(str::to_string)
        .collect();
    assert_eq!(hellos.len(), 2, "{hellos:?}");
    for h in &hellos {
        let spawned = h.contains("\"proc\":\"p1.s");
        assert_eq!(h.contains("\"tag_stable\":false"), spawned, "{h}");
    }
}

#[test]
fn a_forked_child_that_exits_leaves_one_clean_report() {
    let (trial, _) = trial_or_skip!("fork");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert_eq!(trial.health.duplicates, 0);
    assert!(trial.report_missing().is_empty());
    assert_eq!(tags(&trial), ["p1"]);
}

#[test]
fn an_explicit_report_followed_by_more_work_is_still_one_report() {
    let (trial, _) = trial_or_skip!("explicit");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert_eq!(trial.health.duplicates, 0);
    assert_eq!(trial.properties.len(), 1);
}

#[test]
fn one_id_declared_with_two_kinds_is_a_catalog_conflict() {
    let (trial, _) = trial_or_skip!("conflict");
    assert_eq!(trial.conflicts.len(), 1, "{:?}", trial.health);
    assert!(trial
        .verdict()
        .iter()
        .any(|f| f.reason == FailureReason::CatalogConflict));
}

#[test]
fn a_program_with_no_assertions_still_reports_work() {
    let (trial, _) = trial_or_skip!("empty");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.properties.is_empty());
    assert_eq!(trial.total_work(), 1);
    assert_eq!(trial.procs[0].end.as_ref().unwrap().sites, 0);
}

#[test]
fn ids_and_messages_at_their_limits_fold_cleanly() {
    let (trial, _) = trial_or_skip!("limits");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.properties.iter().any(|p| p.id.len() == 128));
    assert!(trial.properties.iter().any(|p| p.msg.len() == 512));
    assert!(trial
        .properties
        .iter()
        .any(|p| p.id == "bad_id_with:odd/chars"));
}

#[test]
fn a_closed_stderr_pipe_does_not_kill_the_program_with_sigpipe() {
    let Some(mut cmd) = sample("sigpipe") else {
        return;
    };
    // Standard error is a pipe whose read end is closed before the program writes its report (it writes at exit,
    // well after the interpreter has started).
    let mut child = cmd
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("python starts");
    drop(child.stderr.take());
    let status = child.wait().expect("python exits");
    assert!(status.success(), "{status:?}");
}

#[test]
fn the_events_file_receives_the_report_instead_of_standard_error() {
    let Some(mut cmd) = sample("basic") else {
        return;
    };
    let file = std::env::temp_dir().join(format!("hs_python_events_{}", std::process::id()));
    let _ = std::fs::remove_file(&file);
    cmd.env("HAIRSPRING_EVENTS_FILE", &file);
    let out = run(cmd);
    assert!(out.stderr.is_empty() && out.stdout.is_empty());
    let from_file = fold(&std::fs::read(&file).expect("events file written"));
    let _ = std::fs::remove_file(&file);
    assert!(!from_file.is_degraded(), "{:?}", from_file.health);
    assert_eq!(from_file.properties.len(), 7);
    assert_eq!(from_file.procs[0].end.as_ref().unwrap().dropped, 0);
}

#[test]
fn an_events_file_that_cannot_be_opened_falls_back_to_standard_error_and_is_counted() {
    let Some(mut cmd) = sample("basic") else {
        return;
    };
    cmd.env(
        "HAIRSPRING_EVENTS_FILE",
        "/nonexistent-dir-for-hairspring/events",
    );
    let t = fold(&run(cmd).stderr);
    assert_eq!(t.properties.len(), 7);
    assert_eq!(t.procs[0].end.as_ref().unwrap().dropped, 1);
}

#[test]
fn events_off_writes_nothing() {
    let Some(mut cmd) = sample("basic") else {
        return;
    };
    cmd.env("HAIRSPRING_EVENTS", "off");
    let out = run(cmd);
    assert!(out.stderr.is_empty() && out.stdout.is_empty());
}

#[test]
fn the_process_tag_comes_from_the_environment() {
    let Some(mut cmd) = sample("exit_status") else {
        return;
    };
    cmd.env("HAIRSPRING_PROC", "p1.f3");
    assert_eq!(fold(&run(cmd).stderr).procs[0].tag, "p1.f3");
}

// ---- the planted race --------------------------------------------------------------------------------------------

fn fixture_trial(mode: &str) -> Option<TrialProps> {
    let out = run(fixture(mode)?);
    assert!(out.status.success(), "exit 0 whatever the property says");
    Some(fold(&out.stderr))
}

#[test]
fn the_buggy_build_is_reported_as_a_violation_with_its_witness() {
    if python().is_none() {
        return;
    }
    let trial = (0..RACY_ATTEMPTS)
        .filter_map(|_| fixture_trial("buggy"))
        .find(|t| !t.verdict().is_empty())
        .expect("the planted race never fired natively");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    let failures = trial.verdict();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].id, "py.counter_exact");
    assert_eq!(failures[0].reason, FailureReason::AlwaysFalse);
    let w = failures[0].witness.as_ref().expect("a witness");
    assert!(w.vals[0] < 4_000, "{:?}", w.vals);
    assert_eq!(w.vals[1..], [4_000, 4, 1_000]);
    let s = stats(&trial);
    assert_eq!(status_of(&s, "py.counter_exact"), PropertyStatus::Violation);
    assert_eq!(status_of(&s, "py.all_started"), PropertyStatus::Pass);
}

#[test]
fn the_fixed_build_is_never_reported() {
    for round in 0..LOCKED_RUNS {
        let Some(trial) = fixture_trial("fixed") else {
            return;
        };
        assert!(!trial.is_degraded(), "{round}: {:?}", trial.health);
        assert!(trial.verdict().is_empty(), "{round}: {:?}", trial.verdict());
        assert_eq!(trial.total_work(), 4_000);
        assert_eq!(
            status_of(&stats(&trial), "py.counter_exact"),
            PropertyStatus::Pass
        );
    }
}

#[test]
fn the_fixture_prints_one_contract_line_on_stdout() {
    let out = or_skip!(fixture("fixed"));
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert_eq!(text.lines().count(), 1, "{text:?}");
    assert!(text.starts_with("LANGAT v1 lang=py mode=fixed final=00000000000000004000 expect=00000000000000004000 order="));
    assert!(text.ends_with('\n'));
}

// ---- the C SDK as the reference for the wire format --------------------------------------------------------------

fn c_sample_basic() -> PathBuf {
    let dir = workspace_dir().join("sdks/c");
    let build = dir.join("build/conformance-python");
    let status = Command::new("make")
        .arg("-s")
        .arg("-C")
        .arg(&dir)
        .arg(format!("BUILD={}", build.display()))
        .arg("fixtures")
        .status()
        .expect("make is required to build the C SDK fixtures");
    assert!(status.success(), "building sdks/c fixtures failed");
    build.join("hs_sample_basic")
}

fn lines_of_kind(bytes: &[u8], kind: &str) -> Vec<String> {
    let needle = format!("\"kind\":\"{kind}\"");
    let mut lines: Vec<String> = String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| l.starts_with("HAIRSPRING_EVENT ") && l.contains(&needle))
        .map(str::to_string)
        .collect();
    lines.sort();
    lines
}

/// The line as JSON without the members that name the SDK or the source location.
fn without_sdk_and_location(line: &str) -> serde_json::Value {
    let mut v: serde_json::Value =
        serde_json::from_str(line.strip_prefix("HAIRSPRING_EVENT ").unwrap()).unwrap();
    let o = v.as_object_mut().unwrap();
    for key in ["sdk", "sdk_version", "loc"] {
        o.remove(key);
    }
    v
}

#[test]
fn the_report_matches_the_c_sdks_byte_for_byte() {
    // Same ids, messages, values and order in both programs (sdks/c/tests/fixtures/hs_sample.c, scenario `basic`).
    let Some(python_basic) = sample("basic") else {
        return;
    };
    let c_out = Command::new(c_sample_basic())
        .env_remove("HAIRSPRING_PROC")
        .env_remove("HAIRSPRING_PROC_PID")
        .env_remove("HAIRSPRING_EVENTS")
        .env_remove("HAIRSPRING_EVENTS_FILE")
        .stdout(Stdio::null())
        .output()
        .expect("C sample runs")
        .stderr;
    let py_out = run(python_basic).stderr;
    // The lines that carry counters, witnesses and padding are identical, character for character.
    for kind in ["assert", "work", "thread", "end"] {
        let (c, py) = (lines_of_kind(&c_out, kind), lines_of_kind(&py_out, kind));
        assert!(!c.is_empty(), "{kind}");
        assert_eq!(py, c, "{kind} lines differ");
    }
    // `hello` and `catalog` differ only in the SDK's name and version and in source locations.
    for kind in ["hello", "catalog"] {
        let c: Vec<_> = lines_of_kind(&c_out, kind)
            .iter()
            .map(|l| without_sdk_and_location(l))
            .collect();
        let py: Vec<_> = lines_of_kind(&py_out, kind)
            .iter()
            .map(|l| without_sdk_and_location(l))
            .collect();
        assert_eq!(py.len(), c.len(), "{kind}");
        for line in &c {
            assert!(
                py.contains(line),
                "{kind}: {line} missing from the Python SDK's report"
            );
        }
    }
}
