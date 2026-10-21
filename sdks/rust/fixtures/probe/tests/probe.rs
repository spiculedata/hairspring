//! Native tests of the probe binary: the output format, what is deterministic, the exit statuses.
//!
//! Nothing here runs under Hermit. Natively the thread order and the random values differ from run to run, which is the
//! point of the program; what must hold on every run is the shape of the line and the structure of the hashes.

use std::process::{Command, Output};

const PROBE: &str = env!("CARGO_BIN_EXE_probe-rust");
const THREADS: u64 = 4;
const EXIT_INTERNAL_FAILURE: i32 = 3;
const EXIT_USAGE: i32 = 64;
const NATIVE_RUNS: usize = 20;

fn run(args: &[&str]) -> Output {
    Command::new(PROBE)
        .args(args)
        .output()
        .expect("the probe starts")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The hashes of every order in which four threads can take the lock or finish.
fn permutation_hashes() -> Vec<u64> {
    let mut all = Vec::new();
    for a in 0..THREADS {
        for b in (0..THREADS).filter(|&b| b != a) {
            for c in (0..THREADS).filter(|&c| c != a && c != b) {
                let d = THREADS * (THREADS - 1) / 2 - a - b - c;
                all.push(fnv1a64(format!("{a}{b}{c}{d}").as_bytes()));
            }
        }
    }
    all
}

/// One `key=value` field of the line, checked for its exact width and alphabet.
fn field<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
    name: &str,
    width: usize,
    hex: bool,
) -> &'a str {
    let part = parts.next().unwrap_or_else(|| panic!("no field {name}"));
    let value = part
        .strip_prefix(&format!("{name}="))
        .unwrap_or_else(|| panic!("field {name} expected, got {part}"));
    assert_eq!(value.len(), width, "{name}={value}");
    let ok = if hex {
        value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    } else {
        value.bytes().all(|b| b.is_ascii_digit())
    };
    assert!(ok, "{name}={value}");
    value
}

struct Line {
    lock_order: u64,
    arrival: u64,
    rand: u64,
    vtime_ns: u64,
    extra: u64,
}

fn parse(output: &Output) -> Line {
    let text = std::str::from_utf8(&output.stdout).expect("ASCII on stdout");
    assert!(text.ends_with('\n'), "{text:?}");
    assert_eq!(text.matches('\n').count(), 1, "exactly one line: {text:?}");
    let mut parts = text.trim_end_matches('\n').split(' ');
    assert_eq!(parts.next(), Some("PROBE"));
    assert_eq!(parts.next(), Some("v1"));
    assert_eq!(parts.next(), Some("lang=rust"));
    let hex = |s: &str| u64::from_str_radix(s, 16).expect("hex");
    let line = Line {
        lock_order: hex(field(&mut parts, "lock_order", 16, true)),
        arrival: hex(field(&mut parts, "arrival", 16, true)),
        rand: hex(field(&mut parts, "rand", 16, true)),
        vtime_ns: field(&mut parts, "vtime_ns", 20, false)
            .parse()
            .expect("decimal"),
        extra: hex(field(&mut parts, "extra", 16, true)),
    };
    assert_eq!(parts.next(), None, "exactly six fields: {text:?}");
    line
}

#[test]
fn a_run_prints_one_probe_line_in_the_six_field_form_and_exits_zero() {
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    parse(&output);
}

#[test]
fn the_lock_order_and_the_arrival_are_hashes_of_a_permutation_of_the_four_threads() {
    let allowed = permutation_hashes();
    assert_eq!(allowed.len(), 24);
    for _ in 0..NATIVE_RUNS {
        let line = parse(&run(&[]));
        assert!(
            allowed.contains(&line.lock_order),
            "{:016x}",
            line.lock_order
        );
        assert!(allowed.contains(&line.arrival), "{:016x}", line.arrival);
    }
}

#[test]
fn the_elapsed_time_is_positive() {
    for _ in 0..NATIVE_RUNS {
        assert!(parse(&run(&[])).vtime_ns > 0);
    }
}

#[test]
fn the_random_value_comes_from_the_runtime_and_differs_between_native_runs() {
    let mut seen: Vec<u64> = (0..NATIVE_RUNS).map(|_| parse(&run(&[])).rand).collect();
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() > 1, "{seen:?}");
}

#[test]
fn the_map_order_hash_is_never_the_hash_of_nothing() {
    let empty = fnv1a64(b"");
    assert_ne!(parse(&run(&[])).extra, empty);
}

#[test]
fn any_argument_is_a_usage_error_with_nothing_on_stdout() {
    for args in [
        vec!["buggy"],
        vec!["--switch=1.0"],
        vec![""],
        vec!["a", "b"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(EXIT_USAGE), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with("usage: probe-rust"), "{stderr}");
    }
}

#[test]
fn a_stdout_that_cannot_take_the_line_is_an_internal_failure_not_a_silent_success() {
    if !std::path::Path::new("/dev/full").exists() {
        return;
    }
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full opens");
    let output = Command::new(PROBE)
        .stdout(full)
        .output()
        .expect("the probe starts");
    assert_eq!(output.status.code(), Some(EXIT_INTERNAL_FAILURE));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("internal invariant failed"), "{stderr}");
}
