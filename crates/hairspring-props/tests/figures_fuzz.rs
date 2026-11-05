//! Generated-input tests for the figures a run summary prints: Wilson intervals and rounded percentages. They are built
//! from counts, so the counts can be as large as a 64-bit integer; the figures must stay finite, ordered, and never
//! look better than the evidence. `HAIRSPRING_FUZZ_CASES` raises the case count (see docs/testing.md).

mod common;

use common::fuzz_rng::{base_seed, cases, Rng};
use hairspring_props::{percent, stats};

fn count(rng: &mut Rng) -> u64 {
    match rng.below(6) {
        0 => u64::MAX - rng.below(3) as u64,
        1 => 1 << rng.below(64),
        2 => rng.below(5) as u64,
        _ => rng.next() >> rng.below(64),
    }
}

#[test]
fn wilson_intervals_stay_ordered_inside_zero_to_one_and_around_the_observed_rate() {
    for case in 0..cases(20_000) {
        let mut rng = Rng::new(base_seed() + case as u64 + 1);
        let n = count(&mut rng).max(1);
        let k = if rng.chance(3) {
            [0, n][rng.below(2)]
        } else {
            rng.next() % n.saturating_add(1)
        };
        let (lo, hi) = stats::wilson(k, n, stats::Z_95).expect("valid counts");
        assert!(lo.is_finite() && hi.is_finite(), "k={k} n={n}");
        assert!(
            (0.0..=1.0).contains(&lo) && (0.0..=1.0).contains(&hi) && lo <= hi,
            "k={k} n={n}: {lo} {hi}"
        );
        // A lower bound above the observed rate, or an upper bound below it, would overstate the evidence.
        let p = k as f64 / n as f64;
        assert!(lo <= p && p <= hi, "k={k} n={n}: {lo} {p} {hi}");
        if let Some(bound) = stats::zero_event_upper_bound(n) {
            assert!((0.0..=1.0).contains(&bound), "n={n}: {bound}");
        }
    }
}

#[test]
fn wilson_refuses_counts_and_quantiles_that_make_no_sense() {
    assert_eq!(stats::wilson(1, 0, stats::Z_95), None);
    assert_eq!(stats::wilson(2, 1, stats::Z_95), None);
    for z in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0] {
        assert_eq!(stats::wilson(1, 2, z), None);
    }
}

/// Reads `12.34%` or `<0.01%` back as a fraction of one; `<0.01%` reads as its own bound.
fn shown_fraction(s: &str) -> f64 {
    let number = s.trim_start_matches('<').trim_end_matches('%');
    number.parse::<f64>().expect("a percentage") / 100.0
}

#[test]
fn percentages_round_away_from_overstating() {
    let specials = [
        0.0,
        -0.0,
        1.0,
        0.5,
        f64::MIN_POSITIVE,
        1e-300,
        1e-5,
        0.0001,
        0.00009999,
        0.9999,
        0.99995,
        1.0 - f64::EPSILON,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for case in 0..cases(20_000) {
        let mut rng = Rng::new(base_seed() + 0x2_0000 + case as u64);
        let x = if case < specials.len() {
            specials[case]
        } else {
            match rng.below(4) {
                0 => f64::from_bits(rng.next()),
                1 => (rng.next() % 1_000_001) as f64 / 1_000_000.0,
                2 => (rng.next() % 10_001) as f64 / 10_000.0,
                _ => (rng.next() >> 11) as f64 / (1u64 << 53) as f64,
            }
        };
        let (up, low, near) = (percent::upper(x), percent::lower(x), percent::nearest(x));
        for s in [&up, &low, &near] {
            assert!(s.ends_with('%'), "{x}: {s}");
            assert!((0.0..=1.0).contains(&shown_fraction(s)), "{x}: {s}");
        }
        if x.is_nan() {
            // An unreadable number must not be shown as a good one: an upper bound reads as everything, a lower
            // bound as nothing.
            assert_eq!(up, "100.00%");
            assert_eq!(low, "0.00%");
            continue;
        }
        let clamped = x.clamp(0.0, 1.0);
        let noise = 1e-9 / 10_000.0;
        if !up.starts_with('<') {
            assert!(shown_fraction(&up) + noise >= clamped, "upper({x}) = {up}");
        }
        if !low.starts_with('<') {
            assert!(
                shown_fraction(&low) <= clamped + noise,
                "lower({x}) = {low}"
            );
        }
        if clamped > 0.0 && clamped < 0.0001 {
            assert_eq!(up, "<0.01%", "{x}");
        }
    }
}
