//! Conformance: run the Go shim's sample programs (`examples/go-sql-cancel/cmd/hssample`) and fold what they write
//! with the reference parser, as `c_sdk.rs` does for the C SDK.
//!
//! The programs are built with `go build`. `HS_GO` names the go command (default `go` on PATH);
//! `HS_GO_SHIM_BUILD` names a directory of prebuilt `hssample` and `hssample_violation` binaries. When neither is
//! available (no Go 1.24 or newer: the module needs it and builds with `GOTOOLCHAIN=local`) the tests print a skip
//! notice and pass, unless `HS_REQUIRE_GO` is set, which makes a missing or too old Go an error.

use hairspring_props::{
    parse_line, AssertKind, FailureReason, LineOutcome, PropertyStatus, RunProps, TrialEvents,
    TrialProps,
};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::OnceLock;

#[path = "common/go_toolchain.rs"]
mod go_toolchain;

const MAX_LINE_BYTES: usize = 4096;
const VIOLATION_PREFIX: &str = "HAIRSPRING_VIOLATION: ";

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/go-sql-cancel")
}

fn go_command() -> String {
    std::env::var("HS_GO").unwrap_or_else(|_| "go".to_string())
}

fn go_available() -> bool {
    go_toolchain::go_usable(&go_command())
}

fn go_build(out: &Path, tags: Option<&str>) {
    let mut cmd = Command::new(go_command());
    cmd.current_dir(example_dir())
        .env("GOTOOLCHAIN", "local")
        .env("GOPROXY", "off")
        .arg("build")
        .arg("-o")
        .arg(out);
    if let Some(tags) = tags {
        cmd.arg("-tags").arg(tags);
    }
    let status = cmd.arg("./cmd/hssample").status().expect("go build starts");
    assert!(status.success(), "go build ./cmd/hssample failed");
}

/// Directory holding `hssample` and `hssample_violation`, or None when Go is not available (test is skipped).
fn build_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Some(prebuilt) = std::env::var_os("HS_GO_SHIM_BUILD") {
            return Some(go_toolchain::check_prebuilt_dir(
                "HS_GO_SHIM_BUILD",
                Path::new(&prebuilt),
                &["hssample", "hssample_violation"],
            ));
        }
        if !go_available() {
            assert!(
                !go_toolchain::require_go(),
                "HS_REQUIRE_GO is set but `{}` is not available or is older than Go 1.{}",
                go_command(),
                go_toolchain::MIN_GO_MINOR
            );
            eprintln!(
                "skip: no Go 1.{}+ toolchain (set HS_GO, or HS_REQUIRE_GO=1 to make this an error)",
                go_toolchain::MIN_GO_MINOR
            );
            return None;
        }
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("go_shim");
        std::fs::create_dir_all(&dir).expect("create build dir");
        go_build(&dir.join("hssample"), None);
        go_build(&dir.join("hssample_violation"), Some("hs_violation_only"));
        Some(dir)
    })
    .as_ref()
}

struct Run {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: ExitStatus,
}

/// Run a scenario of `binary` ("hssample" or "hssample_violation"), or None when Go is not available.
fn run(binary: &str, scenario: &str, env: &[(&str, &str)]) -> Option<Run> {
    let dir = build_dir()?;
    let mut cmd = Command::new(dir.join(binary));
    cmd.arg(scenario)
        .env_remove("HAIRSPRING_PROC")
        .env_remove("HAIRSPRING_EVENTS");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("sample program runs");
    Some(Run {
        stdout: out.stdout,
        stderr: out.stderr,
        status: out.status,
    })
}

fn fold(bytes: &[u8]) -> TrialProps {
    TrialEvents::from_output(bytes).fold()
}

fn trial(scenario: &str) -> Option<(TrialProps, Run)> {
    let r = run("hssample", scenario, &[])?;
    Some((fold(&r.stderr), r))
}

fn text_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8(bytes.to_vec())
        .expect("utf-8 output")
        .lines()
        .map(str::to_string)
        .collect()
}

macro_rules! trial_or_skip {
    ($scenario:expr) => {
        match trial($scenario) {
            Some(t) => t,
            None => return,
        }
    };
}

fn property<'a>(t: &'a TrialProps, id: &str) -> &'a hairspring_props::TrialProperty {
    t.properties
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("no property {id}"))
}

#[test]
fn basic_report_is_complete_and_undegraded() {
    let (t, r) = trial_or_skip!("basic");
    assert!(r.status.success());
    assert!(!t.is_degraded(), "{:?}", t.health);
    assert!(t.report_missing().is_empty());
    assert_eq!(t.properties.len(), 7);
    assert_eq!(t.total_work(), 300);
    assert!(!t.perturbing);
    let p = &t.procs[0];
    assert_eq!(p.tag, "p1");
    assert_eq!(p.sdk.as_deref(), Some("go"));
    assert_eq!(p.signal_handlers, Some(false));
    let end = p.end.as_ref().expect("end line");
    assert_eq!(
        (end.sites, end.threads, end.evals, end.dropped),
        (7, 0, 13, 0)
    );
    assert!(t.thread_reports().is_empty());
}

#[test]
fn every_line_is_a_well_formed_event_within_the_cap() {
    let (_, r) = trial_or_skip!("manyids");
    let lines = text_lines(&r.stderr);
    assert!(
        lines.len() > 1000,
        "expected hello, 500 catalog, 500 assert, work, end"
    );
    for l in &lines {
        assert!(l.len() < MAX_LINE_BYTES, "line of {} bytes", l.len());
        assert!(
            matches!(
                parse_line(l.as_bytes()),
                LineOutcome::Event {
                    unknown_fields: 0,
                    ..
                }
            ),
            "not a clean event: {l}"
        );
    }
    assert!(r.stdout.is_empty(), "the shim writes to stderr only");
}

#[test]
fn an_always_that_was_false_is_a_violation_with_its_witness() {
    let (t, _) = trial_or_skip!("basic");
    let failures = t.verdict();
    assert_eq!(failures.len(), 1, "{failures:?}");
    let f = &failures[0];
    assert_eq!(f.id, "basic.always.bad");
    assert_eq!(f.reason, FailureReason::AlwaysFalse);
    assert_eq!(f.kind, AssertKind::Always);
    let w = f.witness.as_ref().expect("first_false witness");
    assert_eq!(w.vals, [7, 8, 0, 0]);
    assert_eq!(w.thread, "go");
    assert_eq!(w.ord, 2);
    let stats = RunProps::new().add(1, &t).finish();
    let status = |id: &str| stats.iter().find(|s| s.id == id).unwrap().status;
    assert_eq!(status("basic.always.bad"), PropertyStatus::Violation);
    assert_eq!(status("basic.reach.never"), PropertyStatus::Unmet);
    assert_eq!(status("basic.reach.hit"), PropertyStatus::Pass);
    assert_eq!(status("basic.sometimes"), PropertyStatus::Pass);
    assert_eq!(status("basic.always.ok"), PropertyStatus::Pass);
    assert_eq!(status("basic.unreach.clean"), PropertyStatus::Pass);
    assert_eq!(status("basic.aou.never"), PropertyStatus::Pass);
}

#[test]
fn extreme_witness_values_survive_the_round_trip() {
    let (t, _) = trial_or_skip!("basic");
    let w = property(&t, "basic.reach.hit")
        .first_true
        .as_ref()
        .expect("first_true");
    assert_eq!(w.vals, [-1, 0, 0, i64::MAX]);
}

#[test]
fn one_id_at_two_call_sites_is_one_property_with_summed_counts() {
    let (t, _) = trial_or_skip!("merge");
    assert!(!t.is_degraded(), "{:?}", t.health);
    assert_eq!(t.health.duplicates, 0);
    let p = property(&t, "merge.shared");
    assert_eq!((p.hits, p.trues, p.falses), (3, 2, 1));
    assert_eq!(t.procs[0].end.as_ref().unwrap().sites, 1);
}

#[test]
fn concurrent_goroutines_are_counted_exactly() {
    let (t, _) = trial_or_skip!("concurrent");
    assert!(!t.is_degraded(), "{:?}", t.health);
    let p = property(&t, "conc.count");
    assert_eq!((p.hits, p.trues, p.falses), (8000, 7992, 8));
    assert_eq!(t.total_work(), 8000);
    let w = t.verdict()[0].witness.clone().expect("witness");
    assert_eq!(w.vals[1], 500);
}

#[test]
fn the_exit_status_is_advisory_and_the_process_exits_with_it() {
    let (t, r) = trial_or_skip!("exit3");
    assert_eq!(r.status.code(), Some(3));
    assert_eq!(t.procs[0].end.as_ref().unwrap().status, 3);
    assert!(!t.is_degraded(), "{:?}", t.health);
}

#[test]
fn a_hit_unreachable_site_fails_the_trial() {
    let (t, _) = trial_or_skip!("unreach");
    let failures = t.verdict();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].id, "unreach.hit");
    assert_eq!(failures[0].kind, AssertKind::Unreachable);
}

#[test]
fn text_that_escapes_past_the_line_cap_still_catalogs_the_site() {
    let (t, r) = trial_or_skip!("longtext");
    assert!(!t.is_degraded(), "{:?}", t.health);
    assert_eq!(property(&t, "long.text").hits, 1);
    assert!(text_lines(&r.stderr)
        .iter()
        .all(|l| l.len() < MAX_LINE_BYTES));
}

#[test]
fn a_program_with_no_sites_reports_an_empty_catalog() {
    let (t, _) = trial_or_skip!("empty");
    assert!(!t.is_degraded(), "{:?}", t.health);
    assert!(t.properties.is_empty());
    assert_eq!(t.procs[0].end.as_ref().unwrap().sites, 0);
}

#[test]
fn the_process_tag_comes_from_the_environment() {
    let Some(r) = run("hssample", "exit3", &[("HAIRSPRING_PROC", "p1.f3")]) else {
        return;
    };
    assert_eq!(fold(&r.stderr).procs[0].tag, "p1.f3");
}

#[test]
fn events_off_writes_nothing() {
    let Some(r) = run("hssample", "basic", &[("HAIRSPRING_EVENTS", "off")]) else {
        return;
    };
    assert!(r.stderr.is_empty() && r.stdout.is_empty());
}

#[test]
fn the_violation_only_build_prints_only_violation_lines() {
    let Some(r) = run("hssample_violation", "basic", &[]) else {
        return;
    };
    let lines = text_lines(&r.stderr);
    assert_eq!(
        lines,
        ["HAIRSPRING_VIOLATION: basic.always.bad: row mixed up (1 false)"]
    );
    assert!(r.stdout.is_empty());
    let folded = fold(&r.stderr);
    assert!(folded.procs.is_empty() && folded.properties.is_empty());
    assert!(folded.verdict().is_empty());
    assert!(lines.iter().all(|l| l.starts_with(VIOLATION_PREFIX)));
}

#[test]
fn the_violation_only_build_is_silent_when_nothing_failed() {
    let Some(r) = run("hssample_violation", "exit3", &[]) else {
        return;
    };
    assert!(r.stderr.is_empty() && r.stdout.is_empty());
    assert_eq!(r.status.code(), Some(3));
}

#[test]
fn both_builds_agree_on_which_sites_are_findings() {
    for scenario in [
        "basic",
        "merge",
        "concurrent",
        "unreach",
        "exit3",
        "manyids",
    ] {
        let (t, full) = trial_or_skip!(scenario);
        let lean = run("hssample_violation", scenario, &[]).expect("violation build");
        let mut from_events: Vec<String> = t.verdict().iter().map(|f| f.id.clone()).collect();
        let mut from_lines: Vec<String> = text_lines(&lean.stderr)
            .iter()
            .map(|l| {
                l.strip_prefix(VIOLATION_PREFIX)
                    .and_then(|rest| rest.split(':').next())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        from_events.sort();
        from_lines.sort();
        assert_eq!(from_events, from_lines, "scenario {scenario}");
        assert_eq!(
            full.status.code(),
            lean.status.code(),
            "scenario {scenario}"
        );
    }
}
