//! Conformance: run the Rust SDK's sample programs (`sdks/rust/samples`) and fold what they write with the reference
//! parser, as `c_sdk.rs` does for the C SDK. One test also compares the Rust SDK's report with the C SDK's byte for
//! byte.
//!
//! The programs are built with `cargo build -p hairspring-sdk --bins` (a nested cargo, run once per test binary).
//! `HS_RUST_SDK_BUILD` names a directory of prebuilt `hs_sample_*` and `hs_counter_*` programs instead.

use hairspring_props::{
    AssertKind, FailureReason, PropertyStat, PropertyStatus, RunProps, TrialEvents, TrialProps,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::OnceLock;

const BUILD_ENV: &str = "HS_RUST_SDK_BUILD";
/// The planted race fires on almost every native run; this many tries make a miss vanishingly unlikely.
const RACY_ATTEMPTS: usize = 200;
const LOCKED_RUNS: usize = 20;

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `bin` the SDK package builds, by name.
fn programs() -> &'static HashMap<String, PathBuf> {
    static PROGRAMS: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    PROGRAMS.get_or_init(|| match std::env::var_os(BUILD_ENV) {
        Some(dir) => prebuilt(Path::new(&dir)),
        None => build(),
    })
}

fn prebuilt(dir: &Path) -> HashMap<String, PathBuf> {
    std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{BUILD_ENV} names {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect()
}

fn build() -> HashMap<String, PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .current_dir(workspace_dir())
        .args([
            "build",
            "-p",
            "hairspring-sdk",
            "--bins",
            "--message-format=json",
        ])
        .stderr(Stdio::inherit())
        .output()
        .expect("cargo is required to build the Rust SDK samples");
    assert!(out.status.success(), "building sdks/rust samples failed");
    String::from_utf8(out.stdout)
        .expect("utf-8 cargo output")
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["kind"][0] == "bin")
        .filter_map(|m| {
            let name = m["target"]["name"].as_str()?.to_string();
            let exe = m["executable"].as_str()?.into();
            Some((name, exe))
        })
        .collect()
}

fn program(name: &str) -> &'static Path {
    programs()
        .get(name)
        .unwrap_or_else(|| panic!("no built program {name}"))
}

fn sample(scenario: &str) -> Command {
    let mut cmd = Command::new(program(&format!("hs_sample_{scenario}")));
    cmd.env_remove("HAIRSPRING_PROC")
        .env_remove("HAIRSPRING_PROC_PID")
        .env_remove("HAIRSPRING_EVENTS")
        .env_remove("HAIRSPRING_EVENTS_FILE");
    cmd
}

/// Run a command with stdout discarded and return what it wrote to stderr.
fn stderr_of(mut cmd: Command) -> (Vec<u8>, ExitStatus) {
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

fn tags(trial: &TrialProps) -> Vec<String> {
    let mut tags: Vec<String> = trial.procs.iter().map(|p| p.tag.clone()).collect();
    tags.sort();
    tags
}

// ---- the same checks as the C SDK's conformance tests ------------------------------------------------------------

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
    assert_eq!(p.sdk.as_deref(), Some("rust"));
    assert_eq!(p.signal_handlers, Some(false));
    let end = p.end.as_ref().expect("end line");
    assert_eq!(end.sites, 7);
    assert_eq!(end.threads, 1);
    assert_eq!(end.dropped, 0);
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
fn exit_records_the_status() {
    let trial = run_trial("exit_status", &[]);
    assert_eq!(trial.procs[0].end.as_ref().unwrap().status, 3);
}

#[test]
fn two_processes_on_one_pipe_fold_cleanly() {
    // macOS pipes are only atomic up to 512 bytes, so there the two processes share an O_APPEND events file.
    for round in 0..10 {
        let bin = program("hs_sample_big");
        let events =
            std::env::temp_dir().join(format!("hs_rust_sdk_{}_{round}.events", std::process::id()));
        let _ = std::fs::remove_file(&events);
        let use_file = !cfg!(target_os = "linux");
        let file_env = if use_file {
            format!("HAIRSPRING_EVENTS_FILE={} ", events.display())
        } else {
            String::new()
        };
        let script = format!(
            "{file_env}HAIRSPRING_PROC=pA {bin} & {file_env}HAIRSPRING_PROC=pB {bin} & wait",
            bin = bin.display()
        );
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        let (piped, _) = stderr_of(cmd);
        let bytes = if use_file {
            let read = std::fs::read(&events).expect("events file written");
            let _ = std::fs::remove_file(&events);
            read
        } else {
            piped
        };
        let trial = fold(&bytes);
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
    let image = program("hs_sample_merge").display().to_string();
    let trial = run_trial("spawn", &[&image]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.report_missing().is_empty());
    let found = tags(&trial);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0], "p1");
    assert!(found[1].starts_with("p1.s"), "{found:?}");
}

#[test]
fn a_forked_child_that_exits_leaves_one_clean_report() {
    let trial = run_trial("fork", &[]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert_eq!(trial.health.duplicates, 0);
    assert!(trial.report_missing().is_empty());
    assert_eq!(tags(&trial), ["p1"]);
}

fn hello_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| l.contains("\"kind\":\"hello\""))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_spawned_tag_says_it_is_not_stable() {
    let image = program("hs_sample_merge").display().to_string();
    let mut cmd = sample("spawn");
    cmd.arg(image);
    let (bytes, _) = stderr_of(cmd);
    assert!(!fold(&bytes).is_degraded());
    let hellos = hello_lines(&bytes);
    assert_eq!(hellos.len(), 2, "{hellos:?}");
    for h in &hellos {
        let spawned = h.contains("\"proc\":\"p1.s");
        assert_eq!(h.contains("\"tag_stable\":false"), spawned, "{h}");
    }
}

#[test]
fn one_id_declared_with_two_kinds_is_a_catalog_conflict() {
    let trial = run_trial("conflict", &[]);
    assert_eq!(trial.conflicts.len(), 1, "{:?}", trial.health);
    assert!(trial
        .verdict()
        .iter()
        .any(|f| f.reason == FailureReason::CatalogConflict));
}

#[test]
fn a_program_with_no_assertions_still_reports_work() {
    let trial = run_trial("empty", &[]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.properties.is_empty());
    assert_eq!(trial.total_work(), 1);
    assert_eq!(trial.procs[0].end.as_ref().unwrap().sites, 0);
}

#[test]
fn ids_and_messages_at_their_limits_fold_cleanly() {
    let trial = run_trial("limits", &[]);
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    assert!(trial.properties.iter().any(|p| p.id.len() == 128));
    assert!(trial.properties.iter().any(|p| p.msg.len() == 512));
}

#[test]
fn a_call_site_outside_the_catalog_section_degrades_nothing_but_is_counted_as_dropped() {
    let trial = run_trial("audit", &[]);
    assert_eq!(trial.procs[0].end.as_ref().unwrap().dropped, 1);
    assert_eq!(
        trial.health.malformed, 0,
        "the note is plain text, not a protocol line"
    );
}

// ---- the planted race --------------------------------------------------------------------------------------------

fn counter_trial(name: &str) -> TrialProps {
    let (bytes, status) = stderr_of(Command::new(program(name)));
    assert!(status.success(), "exit 0 whatever the property says");
    fold(&bytes)
}

#[test]
fn the_racy_build_is_reported_as_a_violation_with_its_witness() {
    let trial = (0..RACY_ATTEMPTS)
        .map(|_| counter_trial("hs_counter_racy"))
        .find(|t| !t.verdict().is_empty())
        .expect("the planted race never fired natively");
    assert!(!trial.is_degraded(), "{:?}", trial.health);
    let failures = trial.verdict();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].id, "rust.counter_exact");
    assert_eq!(failures[0].reason, FailureReason::AlwaysFalse);
    let w = failures[0].witness.as_ref().expect("a witness");
    assert!(w.vals[0] < 4_000 && w.vals[1] == 4_000, "{:?}", w.vals);
    assert_eq!(
        status_of(&stats(&trial), "rust.counter_exact"),
        PropertyStatus::Violation
    );
    assert_eq!(
        status_of(&stats(&trial), "rust.all_started"),
        PropertyStatus::Pass
    );
}

#[test]
fn the_locked_twin_is_never_reported() {
    for round in 0..LOCKED_RUNS {
        let trial = counter_trial("hs_counter_locked");
        assert!(!trial.is_degraded(), "{round}: {:?}", trial.health);
        assert!(trial.verdict().is_empty(), "{round}: {:?}", trial.verdict());
        assert_eq!(trial.total_work(), 4_000);
        assert_eq!(
            status_of(&stats(&trial), "rust.counter_exact"),
            PropertyStatus::Pass
        );
    }
}

// ---- the C SDK as the reference for the wire format --------------------------------------------------------------

fn c_sample_basic() -> PathBuf {
    let dir = workspace_dir().join("sdks/c");
    let build = dir.join("build/conformance");
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
    let (c_out, _) = stderr_of(Command::new(c_sample_basic()));
    let (rust_out, _) = stderr_of(sample("basic"));
    // The lines that carry counters, witnesses and padding are identical, character for character.
    for kind in ["assert", "work", "thread", "end"] {
        let (c, rust) = (lines_of_kind(&c_out, kind), lines_of_kind(&rust_out, kind));
        assert!(!c.is_empty(), "{kind}");
        assert_eq!(rust, c, "{kind} lines differ");
    }
    // `hello` and `catalog` differ only in the SDK's name and version and in source locations.
    for kind in ["hello", "catalog"] {
        let c: Vec<_> = lines_of_kind(&c_out, kind)
            .iter()
            .map(|l| without_sdk_and_location(l))
            .collect();
        let rust: Vec<_> = lines_of_kind(&rust_out, kind)
            .iter()
            .map(|l| without_sdk_and_location(l))
            .collect();
        assert_eq!(rust.len(), c.len(), "{kind}");
        for line in &c {
            assert!(
                rust.contains(line),
                "{kind}: {line} missing from the Rust SDK's report"
            );
        }
    }
}
