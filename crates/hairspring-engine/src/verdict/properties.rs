//! Naming failed guest properties in a verdict.
use hairspring_props::PropertyFailure;

/// Failures named in a verdict message before the rest are summarised as a count.
const MAX_NAMED_FAILURES: usize = 3;

/// A one-line reason naming the id, thread and ordinal of each failure (the first few).
pub fn describe_failures(failures: &[PropertyFailure]) -> String {
    let named: Vec<String> = failures
        .iter()
        .take(MAX_NAMED_FAILURES)
        .map(PropertyFailure::describe)
        .collect();
    match failures.len().saturating_sub(MAX_NAMED_FAILURES) {
        0 => named.join("; "),
        n => format!("{}; and {n} more", named.join("; ")),
    }
}
