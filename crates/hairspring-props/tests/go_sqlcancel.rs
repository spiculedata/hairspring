//! Conformance: build the sqlcancel workload (`examples/go-sql-cancel/cmd/sqlcancel`), run it, and fold what it
//! writes with the reference parser, as `go_shim.rs` does for the shim's sample programs.
//!
//! `HS_GO` names the go command (default `go` on PATH). `HS_SQLCANCEL_BUILD` names a directory of prebuilt
//! `sqlcancel` and `sqlcancel_violation` binaries. `SQLCANCEL_EXPECT=buggy|clean` says whether the toolchain has
//! the database/sql bug (stock go1.24.5) or not (go1.24.6, go1.24.5 plus the fix); when set, the positive control
//! (`-mode=yield`) must fire or be exactly zero. When no Go 1.24 or newer is available (the module
//! needs it and builds with `GOTOOLCHAIN=local`) the tests print a skip notice and pass, unless `HS_REQUIRE_GO` is
//! set.

use hairspring_props::{AssertKind, FailureReason, TrialEvents, TrialProps};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[path = "common/go_toolchain.rs"]
mod go_toolchain;

const PROPERTY: &str = "go.sql.scan_returns_own_row";
const VIOLATION_PREFIX: &str = "HAIRSPRING_VIOLATION: ";
const CATALOG_IDS: [&str; 7] = [
    "go.sql.cancel_overlapped_scan",
    "go.sql.conn_handed_to_waiter",
    "go.sql.harness.all_queries_ran",
    "go.sql.harness.conn_exclusive",
    "go.sql.harness.error_without_cancel",
    "go.sql.scan_error_after_cancel",
    "go.sql.scan_returns_own_row",
];
const USAGE_EXIT: i32 = 64;
const WORKERS: u64 = 4;

fn go_command() -> String {
    std::env::var("HS_GO").unwrap_or_else(|_| "go".to_string())
}

fn go_available() -> bool {
    go_toolchain::go_usable(&go_command())
}

fn go_build(out: &Path, tags: Option<&str>) {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/go-sql-cancel");
    let mut cmd = Command::new(go_command());
    cmd.current_dir(example)
        .env("GOTOOLCHAIN", "local")
        .env("GOPROXY", "off")
        .arg("build")
        .arg("-o")
        .arg(out);
    if let Some(tags) = tags {
        cmd.arg("-tags").arg(tags);
    }
    let status = cmd
        .arg("./cmd/sqlcancel")
        .status()
        .expect("go build starts");
    assert!(status.success(), "go build ./cmd/sqlcancel failed");
}

/// Directory holding `sqlcancel` and `sqlcancel_violation`, or None when Go is not available (test is skipped).
fn build_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Some(prebuilt) = std::env::var_os("HS_SQLCANCEL_BUILD") {
            return Some(go_toolchain::check_prebuilt_dir(
                "HS_SQLCANCEL_BUILD",
                Path::new(&prebuilt),
                &["sqlcancel", "sqlcancel_violation"],
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("go_sqlcancel");
        std::fs::create_dir_all(&dir).expect("create build dir");
        go_build(&dir.join("sqlcancel"), None);
        go_build(&dir.join("sqlcancel_violation"), Some("hs_violation_only"));
        Some(dir)
    })
    .as_ref()
}

struct Run {
    stdout: String,
    stderr: Vec<u8>,
    code: Option<i32>,
}

fn run(binary: &str, args: &[&str]) -> Option<Run> {
    let dir = build_dir()?;
    let out = Command::new(dir.join(binary))
        .args(args)
        .env_remove("HAIRSPRING_PROC")
        .env_remove("HAIRSPRING_EVENTS")
        .output()
        .expect("sqlcancel runs");
    Some(Run {
        stdout: String::from_utf8(out.stdout).expect("utf-8 stdout"),
        stderr: out.stderr,
        code: out.status.code(),
    })
}

fn fold(bytes: &[u8]) -> TrialProps {
    TrialEvents::from_output(bytes).fold()
}

fn stderr_lines(r: &Run) -> Vec<String> {
    String::from_utf8(r.stderr.clone())
        .expect("utf-8 stderr")
        .lines()
        .map(str::to_string)
        .collect()
}

fn expect() -> Option<String> {
    std::env::var("SQLCANCEL_EXPECT")
        .ok()
        .filter(|s| !s.is_empty())
}

macro_rules! run_or_skip {
    ($binary:expr, $args:expr) => {
        match run($binary, $args) {
            Some(r) => r,
            None => return,
        }
    };
}

#[test]
fn the_report_is_complete_with_the_seven_design_sites_and_no_thread_lines() {
    let r = run_or_skip!("sqlcancel", &["-mode=std", "-q=20"]);
    let t = fold(&r.stderr);

    assert_eq!(r.code, Some(0));
    assert!(!t.is_degraded(), "{:?}", t.health);
    assert!(t.report_missing().is_empty());
    let mut ids: Vec<&str> = t.properties.iter().map(|p| p.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, CATALOG_IDS);
    let p = &t.procs[0];
    assert_eq!(p.sdk.as_deref(), Some("go"));
    assert_eq!(p.signal_handlers, Some(false));
    let end = p.end.as_ref().expect("end line");
    assert_eq!(
        (end.sites, end.threads, end.dropped, end.status),
        (7, 0, 0, 0)
    );
    assert!(t.thread_reports().is_empty());
    assert_eq!(t.total_work(), WORKERS * 20);
    // hello + 7 catalog + 7 assert + work + end
    assert_eq!(stderr_lines(&r).len(), 17);
}

#[test]
fn the_site_kinds_follow_the_design() {
    let r = run_or_skip!("sqlcancel", &["-q=5"]);
    let t = fold(&r.stderr);
    let kind = |id: &str| t.properties.iter().find(|p| p.id == id).expect(id).kind;
    assert_eq!(kind(PROPERTY), AssertKind::Always);
    assert_eq!(
        kind("go.sql.scan_error_after_cancel"),
        AssertKind::Reachable
    );
    assert_eq!(kind("go.sql.cancel_overlapped_scan"), AssertKind::Sometimes);
    assert_eq!(kind("go.sql.conn_handed_to_waiter"), AssertKind::Sometimes);
    for harness in [
        "go.sql.harness.error_without_cancel",
        "go.sql.harness.conn_exclusive",
        "go.sql.harness.all_queries_ran",
    ] {
        assert_eq!(kind(harness), AssertKind::Always);
    }
}

#[test]
fn harness_invariants_hold_and_every_scan_is_evaluated() {
    let r = run_or_skip!("sqlcancel", &["-q=30"]);
    let t = fold(&r.stderr);
    for p in t.properties.iter().filter(|p| p.id.contains(".harness.")) {
        assert_eq!(p.falses, 0, "{} was false", p.id);
        assert!(p.hits > 0, "{} never evaluated", p.id);
    }
    let own = t.properties.iter().find(|p| p.id == PROPERTY).unwrap();
    let errs = t
        .properties
        .iter()
        .find(|p| p.id == "go.sql.scan_error_after_cancel")
        .unwrap();
    assert_eq!(
        own.hits + errs.hits,
        WORKERS * 30,
        "every Scan is either ok or an allowed error"
    );
}

#[test]
fn stdout_is_one_fixed_width_line_that_counts_the_queries() {
    let a = run_or_skip!("sqlcancel", &["-q=10"]);
    let b = run_or_skip!("sqlcancel", &["-q=10"]);
    assert_eq!(a.stdout.matches('\n').count(), 1);
    assert!(
        a.stdout.starts_with("sqlcancel mode=std queries=      40 "),
        "{}",
        a.stdout
    );
    assert_eq!(a.stdout.len(), b.stdout.len());
}

#[test]
fn a_usage_error_exits_64_and_sends_no_protocol_lines() {
    let r = run_or_skip!("sqlcancel", &["-mode=fast"]);
    assert_eq!(r.code, Some(USAGE_EXIT));
    assert!(r.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&r.stderr).contains("HAIRSPRING_EVENT"));
}

#[test]
fn events_off_writes_no_protocol_lines() {
    let Some(dir) = build_dir() else { return };
    let out = Command::new(dir.join("sqlcancel"))
        .args(["-q=5"])
        .env("HAIRSPRING_EVENTS", "off")
        .output()
        .unwrap();
    assert!(out.stderr.is_empty());
    assert!(out.stdout.starts_with(b"sqlcancel "));
}

#[test]
fn the_positive_control_matches_the_toolchain() {
    let Some(expect) = expect() else { return };
    let r = run_or_skip!("sqlcancel", &["-mode=yield", "-q=100"]);
    let t = fold(&r.stderr);
    assert_eq!(r.code, Some(0), "a violation is not an exit status");
    let property = t.properties.iter().find(|p| p.id == PROPERTY).unwrap();
    match expect.as_str() {
        "buggy" => {
            assert!(property.falses > 0, "the control did not fire");
            let failures = t.verdict();
            assert_eq!(failures.len(), 1, "{failures:?}");
            assert_eq!(failures[0].id, PROPERTY);
            assert_eq!(failures[0].reason, FailureReason::AlwaysFalse);
            let class = failures[0].witness.as_ref().expect("witness").vals[3];
            assert!(
                class == 1 || class == 2,
                "class {class}: foreign or torn expected"
            );
        }
        "clean" => {
            assert_eq!(property.falses, 0);
            assert!(t.verdict().is_empty(), "{:?}", t.verdict());
        }
        other => panic!("SQLCANCEL_EXPECT={other}, want buggy or clean"),
    }
}

#[test]
fn the_violation_only_build_names_the_property_exactly_when_the_full_build_does() {
    let Some(expect) = expect() else { return };
    let r = run_or_skip!("sqlcancel_violation", &["-mode=yield", "-q=100"]);
    assert_eq!(r.code, Some(0));
    assert!(r.stdout.starts_with("sqlcancel mode=yield "));
    let lines = stderr_lines(&r);
    match expect.as_str() {
        "buggy" => {
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(
                lines[0].starts_with(&format!("{VIOLATION_PREFIX}{PROPERTY}: ")),
                "{}",
                lines[0]
            );
        }
        "clean" => assert!(lines.is_empty(), "{lines:?}"),
        other => panic!("SQLCANCEL_EXPECT={other}, want buggy or clean"),
    }
}
