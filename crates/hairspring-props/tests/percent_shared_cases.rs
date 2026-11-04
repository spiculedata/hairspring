//! Runs the cases that the web client's tests also run (see `percent_cases.txt`), so the CLI and the web UI cannot
//! show different bounds for the same counts.

use hairspring_props::percent::{lower, nearest, upper};

const CASES: &str = include_str!("percent_cases.txt");

#[test]
fn every_shared_case_gives_the_expected_text() {
    let mut checked = 0;
    for line in CASES
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [direction, fraction, expected] = parts[..] else {
            panic!("malformed case line: {line}");
        };
        let x: f64 = fraction
            .parse()
            .unwrap_or_else(|e| panic!("bad fraction in {line}: {e}"));
        let got = match direction {
            "upper" => upper(x),
            "lower" => lower(x),
            "nearest" => nearest(x),
            other => panic!("unknown direction {other} in {line}"),
        };
        assert_eq!(got, expected, "{line}");
        checked += 1;
    }
    assert!(checked >= 15, "only {checked} cases were read");
}
