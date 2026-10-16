use hairspring_props::stats::{rule_of_three, wilson, Z_95};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn assert_interval(k: u64, n: u64, lo: f64, hi: f64) {
    let (l, h) = wilson(k, n, Z_95).expect("valid inputs give an interval");
    assert!(close(l, lo, 5e-4), "{k}/{n} lower {l} vs {lo}");
    assert!(close(h, hi, 5e-4), "{k}/{n} upper {h} vs {hi}");
}

#[test]
fn wilson_matches_the_part_d_rows() {
    assert_interval(7, 88, 0.039, 0.155);
    assert_interval(6, 90, 0.031, 0.138);
    assert_interval(0, 90, 0.0, 0.041);
    assert_interval(15, 226, 0.041, 0.107);
}

#[test]
fn wilson_at_zero_successes_has_exact_zero_lower_bound() {
    let (l, _) = wilson(0, 50, Z_95).unwrap();
    assert_eq!(l, 0.0);
}

#[test]
fn wilson_at_all_successes_has_exact_one_upper_bound() {
    let (_, h) = wilson(50, 50, Z_95).unwrap();
    assert_eq!(h, 1.0);
}

#[test]
fn wilson_rejects_degenerate_inputs() {
    assert_eq!(wilson(0, 0, Z_95), None);
    assert_eq!(wilson(5, 4, Z_95), None);
    assert_eq!(wilson(1, 10, 0.0), None);
    assert_eq!(wilson(1, 10, -1.0), None);
    assert_eq!(wilson(1, 10, f64::NAN), None);
    assert_eq!(wilson(1, 10, f64::INFINITY), None);
}

#[test]
fn wilson_survives_huge_counts() {
    let (l, h) = wilson(u64::MAX / 2, u64::MAX, Z_95).unwrap();
    assert!(l <= 0.5 && h >= 0.5 && l >= 0.0 && h <= 1.0);
}

#[test]
fn wilson_interval_contains_the_point_estimate_and_grows_with_k() {
    let n = 200;
    let mut prev_lo = -1.0;
    let mut prev_hi = -1.0;
    for k in 0..=n {
        let (l, h) = wilson(k, n, Z_95).unwrap();
        let p = k as f64 / n as f64;
        assert!(l <= p + 1e-12 && p <= h + 1e-12, "k={k}");
        assert!(l >= prev_lo && h >= prev_hi, "not monotone at k={k}");
        prev_lo = l;
        prev_hi = h;
    }
}

#[test]
fn rule_of_three_is_three_over_n() {
    assert!(close(rule_of_three(3000).unwrap(), 1e-3, 1e-12));
    assert!(close(rule_of_three(90).unwrap(), 3.0 / 90.0, 1e-12));
    assert_eq!(rule_of_three(0), None);
}
