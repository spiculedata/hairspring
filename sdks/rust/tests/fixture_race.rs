//! Native self-tests of the language fixture (fixtures/lang_at): one binary, three modes chosen by `argv[1]`.
//! `buggy` can lose an update natively, `fixed` never does, `wide` is the buggy program with a ten times longer window.
//! Nothing here runs under Hermit; this proves the race exists, the report sees it and the output follows the
//! fixture contract (one `LANGAT v1` stdout line, exit 0 / 3 / 64).

mod common;

use common::{events, number, of_kind, run, Run};

const FIXTURE: &str = env!("CARGO_BIN_EXE_at-rust");
const EXPECTED: u64 = 4_000;
const RACY_ATTEMPTS: usize = 200;
const FIXED_RUNS: usize = 20;
const MODES: [&str; 3] = ["buggy", "fixed", "wide"];
const USAGE_EXIT: i32 = 64;

/// The fields of a `LANGAT v1` line.
struct LangAtLine {
    mode: String,
    final_value: u64,
    expect: u64,
    order: String,
}

fn hex16(field: &str) -> bool {
    field.len() == 16
        && field
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn digits20(field: &str) -> Option<u64> {
    if field.len() == 20 && field.bytes().all(|b| b.is_ascii_digit()) {
        field.parse().ok()
    } else {
        None
    }
}

/// Parse the whole stdout of a run; panics unless it is exactly one well-formed line.
fn parse_line(stdout: &str) -> LangAtLine {
    assert!(stdout.ends_with('\n'), "{stdout:?}");
    assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
    let parts: Vec<&str> = stdout.trim_end_matches('\n').split(' ').collect();
    assert_eq!(parts.len(), 8, "{stdout:?}");
    assert_eq!((parts[0], parts[1]), ("LANGAT", "v1"), "{stdout:?}");
    assert_eq!(parts[2], "lang=rust", "{stdout:?}");
    let field = |i: usize, key: &str| -> String {
        parts[i]
            .strip_prefix(key)
            .unwrap_or_else(|| panic!("{key} missing in {stdout:?}"))
            .to_string()
    };
    let order = field(6, "order=");
    assert!(hex16(&order), "{stdout:?}");
    assert!(hex16(&field(7, "rand=")), "{stdout:?}");
    LangAtLine {
        mode: field(3, "mode="),
        final_value: digits20(&field(4, "final=")).expect("final is 20 digits"),
        expect: digits20(&field(5, "expect=")).expect("expect is 20 digits"),
        order,
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Every finishing order of the four workers, written as four ASCII digits, hashed.
fn possible_orders() -> Vec<String> {
    let mut orders = Vec::new();
    for a in 0..4u8 {
        for b in (0..4u8).filter(|&b| b != a) {
            for c in (0..4u8).filter(|&c| c != a && c != b) {
                let d = 6 - a - b - c;
                let digits = [a, b, c, d].map(|n| b'0' + n);
                orders.push(format!("{:016x}", fnv1a64(&digits)));
            }
        }
    }
    orders
}

fn fixture(mode: &str) -> Run {
    run(FIXTURE, &[mode], &[])
}

/// The assert line of one property id.
fn assert_line(r: &Run, id: &str) -> String {
    let ev = events(&r.stderr);
    of_kind(&ev, "assert")
        .into_iter()
        .find(|l| l.contains(&format!("\"id\":\"{id}\"")))
        .unwrap_or_else(|| panic!("{id} is reported"))
        .to_string()
}

fn counter_line(r: &Run) -> String {
    assert_line(r, "rust.counter_exact")
}

/// Run `mode` until the property is false once; `None` means it never was.
fn first_hit(mode: &str) -> Option<(Run, String)> {
    (0..RACY_ATTEMPTS).find_map(|_| {
        let r = fixture(mode);
        assert!(r.status.success(), "exit 0 whatever the property says");
        let line = counter_line(&r);
        (number(&line, "falses") > 0).then_some((r, line))
    })
}

#[test]
fn the_buggy_mode_loses_updates_and_the_report_says_the_property_was_false() {
    let (r, line) = first_hit("buggy").expect("the planted race never fired natively in 200 runs");
    let out = parse_line(&r.stdout);
    assert_eq!((number(&line, "hits"), number(&line, "falses")), (1, 1));
    assert!(
        out.final_value < EXPECTED,
        "a false property means a lost update: {}",
        r.stdout
    );
    assert!(
        line.contains(&format!("\"vals\":[{},{EXPECTED},4,1000]", out.final_value)),
        "four witness values [final, 4000, 4, 1000]: {line}"
    );
}

#[test]
fn the_wide_mode_is_the_buggy_program_with_a_longer_window_and_also_loses_updates() {
    let (r, _) = first_hit("wide").expect("the wide arm never fired natively in 200 runs");
    let out = parse_line(&r.stdout);
    assert_eq!(out.mode, "wide");
    assert!(out.final_value < out.expect, "{}", r.stdout);
}

#[test]
fn the_fixed_mode_never_loses_an_update() {
    for i in 0..FIXED_RUNS {
        let r = fixture("fixed");
        assert!(r.status.success());
        let line = counter_line(&r);
        assert_eq!(number(&line, "falses"), 0, "run {i}: {line}");
        assert_eq!(number(&line, "trues"), 1);
        let out = parse_line(&r.stdout);
        assert_eq!(
            (out.final_value, out.expect),
            (EXPECTED, EXPECTED),
            "run {i}"
        );
        assert!(
            line.contains(&format!("\"vals\":[{EXPECTED},{EXPECTED},4,1000]")),
            "{line}"
        );
    }
}

#[test]
fn a_property_false_and_a_short_count_always_go_together() {
    for _ in 0..20 {
        let r = fixture("buggy");
        let property_false = number(&counter_line(&r), "falses") > 0;
        let out = parse_line(&r.stdout);
        assert_eq!(
            property_false,
            out.final_value != out.expect,
            "{}",
            r.stdout
        );
    }
}

#[test]
fn stdout_is_the_langat_line_naming_the_mode_for_every_mode() {
    for mode in MODES {
        let r = fixture(mode);
        assert!(r.status.success(), "{mode}");
        let out = parse_line(&r.stdout);
        assert_eq!(out.mode, mode);
        assert_eq!(out.expect, EXPECTED);
        assert!(possible_orders().contains(&out.order), "{}", r.stdout);
    }
}

#[test]
fn each_property_is_evaluated_exactly_once_and_in_main() {
    for mode in MODES {
        let r = fixture(mode);
        let exact = counter_line(&r);
        let started = assert_line(&r, "rust.all_started");
        assert_eq!(number(&exact, "hits"), 1, "{mode}: {exact}");
        // Evaluated in each worker it would have four hits.
        assert_eq!(number(&started, "hits"), 1, "{mode}: {started}");
        assert_eq!(number(&started, "trues"), 1, "{mode}: {started}");
    }
}

#[test]
fn every_mode_reports_four_named_workers_the_work_counter_and_two_sites() {
    for mode in MODES {
        let r = fixture(mode);
        let ev = events(&r.stderr);
        let threads = of_kind(&ev, "thread");
        for w in 0..4 {
            assert!(
                threads
                    .iter()
                    .any(|l| l.contains(&format!("\"thread\":\"worker-{w}\""))),
                "{mode}: worker {w}"
            );
        }
        assert_eq!(number(of_kind(&ev, "work")[0], "ops"), EXPECTED, "{mode}");
        assert_eq!(number(of_kind(&ev, "end")[0], "sites"), 2, "{mode}");
    }
}

#[test]
fn a_missing_unknown_or_extra_argument_is_a_usage_error_with_exit_64() {
    let cases: [&[&str]; 6] = [
        &[],
        &["nope"],
        &["BUGGY"],
        &["buggy", "extra"],
        &["--switch=1.0"],
        &[""],
    ];
    for args in cases {
        let r = run(FIXTURE, args, &[]);
        assert_eq!(r.status.code(), Some(USAGE_EXIT), "{args:?}");
        assert!(r.stdout.is_empty(), "{args:?}: {:?}", r.stdout);
        assert!(r.stderr.contains("usage: "), "{args:?}: {}", r.stderr);
    }
}

#[test]
fn the_harness_seed_does_not_change_what_the_program_does() {
    let r = run(FIXTURE, &["fixed"], &[("HAIRSPRING_SEED", "12345")]);
    assert!(r.status.success());
    assert_eq!(parse_line(&r.stdout).final_value, EXPECTED);
}
