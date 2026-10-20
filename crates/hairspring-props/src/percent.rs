//! Percentages for reports, rounded in the direction that never overstates what the counts show.
//!
//! An upper bound is rounded up and a lower bound down, so a shown bound is never tighter than the real one. A
//! non-zero figure below 0.01% is written `<0.01%` instead of being rounded to a number that is not true.

/// Hundredths of a percent in a fraction of 1.
const HUNDREDTHS_PER_UNIT: f64 = 10_000.0;
/// Absorbs float noise so that `0.0149` is `1.49%` in every direction.
const NOISE: f64 = 1e-9;
const LESS_THAN_SHOWN: &str = "<0.01%";

fn hundredths(x: f64) -> f64 {
    if x.is_nan() {
        return 0.0;
    }
    x.clamp(0.0, 1.0) * HUNDREDTHS_PER_UNIT
}

fn show(h: f64, round: fn(f64) -> f64) -> String {
    if h > 0.0 && h < 1.0 {
        return LESS_THAN_SHOWN.to_owned();
    }
    format!("{:.2}%", round(h) / 100.0)
}

/// An upper bound, rounded up.
pub fn upper(x: f64) -> String {
    show(hundredths(x), |h| (h - NOISE).ceil().max(0.0))
}

/// A lower bound, rounded down.
pub fn lower(x: f64) -> String {
    show(hundredths(x), |h| (h + NOISE).floor())
}

/// A point estimate, rounded to nearest.
pub fn nearest(x: f64) -> String {
    show(hundredths(x), f64::round)
}
