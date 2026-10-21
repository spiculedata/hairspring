//! Native self-tests of the planted race (fixtures/counter_*.rs): the detector can fire on the racy build and never
//! fires on its locked twin. Nothing here runs under Hermit; this proves the race exists and the report sees it.

mod common;

use common::{events, number, of_kind, run, Run};

const RACY: &str = env!("CARGO_BIN_EXE_hs_counter_racy");
const LOCKED: &str = env!("CARGO_BIN_EXE_hs_counter_locked");
const EXPECTED: u64 = 4_000;
const RACY_ATTEMPTS: usize = 200;
const LOCKED_RUNS: usize = 20;

/// The `rust.counter_exact` assert line of a run.
fn counter_line(r: &Run) -> String {
    let ev = events(&r.stderr);
    of_kind(&ev, "assert")
        .into_iter()
        .find(|l| l.contains("\"id\":\"rust.counter_exact\""))
        .expect("the counter property is reported")
        .to_string()
}

fn final_value(r: &Run) -> u64 {
    let field = r
        .stdout
        .split_whitespace()
        .find_map(|w| w.strip_prefix("final="))
        .expect("stdout carries the final value");
    field.parse().expect("final value is a number")
}

#[test]
fn the_racy_build_loses_updates_and_the_report_says_the_property_was_false() {
    let mut hit = None;
    for _ in 0..RACY_ATTEMPTS {
        let r = run(RACY, &[], &[]);
        assert!(r.status.success(), "exit 0 whatever the property says");
        let line = counter_line(&r);
        if number(&line, "falses") > 0 {
            hit = Some((r, line));
            break;
        }
    }
    let (r, line) = hit.expect("the planted race never fired natively in 200 runs");
    assert_eq!((number(&line, "hits"), number(&line, "falses")), (1, 1));
    assert!(
        final_value(&r) < EXPECTED,
        "a false property means a lost update: {}",
        r.stdout
    );
    assert!(
        line.contains(&format!("\"vals\":[{},{EXPECTED},0,0]", final_value(&r))),
        "{line}"
    );
}

#[test]
fn the_locked_twin_never_loses_an_update() {
    for i in 0..LOCKED_RUNS {
        let r = run(LOCKED, &[], &[]);
        assert!(r.status.success());
        let line = counter_line(&r);
        assert_eq!(number(&line, "falses"), 0, "run {i}: {line}");
        assert_eq!(number(&line, "trues"), 1);
        assert_eq!(final_value(&r), EXPECTED, "run {i}");
    }
}

#[test]
fn a_property_false_and_a_short_count_always_go_together() {
    for _ in 0..20 {
        let r = run(RACY, &[], &[]);
        let property_false = number(&counter_line(&r), "falses") > 0;
        assert_eq!(property_false, final_value(&r) != EXPECTED, "{}", r.stdout);
    }
}

#[test]
fn both_builds_report_four_named_workers_and_the_coverage_marker() {
    for exe in [RACY, LOCKED] {
        let r = run(exe, &[], &[]);
        let ev = events(&r.stderr);
        let started: Vec<&str> = of_kind(&ev, "assert")
            .into_iter()
            .filter(|l| l.contains("\"id\":\"rust.all_started\""))
            .collect();
        assert_eq!(number(started[0], "trues"), 4);
        let threads = of_kind(&ev, "thread");
        for w in 0..4 {
            assert!(
                threads
                    .iter()
                    .any(|l| l.contains(&format!("\"thread\":\"worker-{w}\""))),
                "{w}"
            );
        }
        assert_eq!(number(of_kind(&ev, "work")[0], "ops"), EXPECTED);
        assert_eq!(number(of_kind(&ev, "end")[0], "sites"), 2);
    }
}

#[test]
fn stdout_is_one_fixed_width_line() {
    for exe in [RACY, LOCKED] {
        let r = run(exe, &[], &[]);
        let lines: Vec<&str> = r.stdout.lines().collect();
        assert_eq!(lines.len(), 1, "{}", r.stdout);
        let parts: Vec<&str> = lines[0].split(' ').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            (parts[0].len(), parts[1].len(), parts[2].len()),
            (26, 22, 20),
            "{}",
            lines[0]
        );
        assert!(
            parts[0].starts_with("final=")
                && parts[1].starts_with("order=")
                && parts[2].starts_with("rnd=")
        );
    }
}
