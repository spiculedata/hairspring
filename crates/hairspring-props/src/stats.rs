//! Binomial interval helpers as pure functions.

/// z for a two-sided 95% interval.
pub const Z_95: f64 = 1.959_963_984_540_054;

/// Wilson score interval `(lower, upper)` for `k` successes in `n` trials at normal quantile `z`.
/// `None` when `n == 0`, `k > n` or `z` is not a positive finite number.
pub fn wilson(k: u64, n: u64, z: f64) -> Option<(f64, f64)> {
    if n == 0 || k > n || !z.is_finite() || z <= 0.0 {
        return None;
    }
    let nf = n as f64;
    let p = k as f64 / nf;
    let z2 = z * z;
    let denom = 1.0 + z2 / nf;
    let centre = (p + z2 / (2.0 * nf)) / denom;
    let margin = z * (p * (1.0 - p) / nf + z2 / (4.0 * nf * nf)).sqrt() / denom;
    let lower = if k == 0 {
        0.0
    } else {
        (centre - margin).max(0.0)
    };
    let upper = if k == n {
        1.0
    } else {
        (centre + margin).min(1.0)
    };
    Some((lower, upper))
}

/// Rule of three: the approximate 95% upper bound `3 / n` on a rate after `n` trials with no events.
/// `None` when `n == 0`.
pub fn rule_of_three(n: u64) -> Option<f64> {
    if n == 0 {
        return None;
    }
    Some(3.0 / n as f64)
}
