//! The sweep section of the human-readable run report: how every trial ended, and which seeds failed.
//!
//! The rate itself, with its interval and the trials left out of it, is printed by `format_rate` from the same summary.
use crate::report::RunOutcome;
use crate::sweep::SweepSummary;

fn counts_line(s: &SweepSummary) -> String {
    format!(
        "sweep: {} trials: {} pass, {} violation, {} inconclusive, {} unusable, {} infra",
        s.trials, s.passes, s.violations, s.inconclusive, s.unusable, s.infra
    )
}

fn seeds_line(s: &SweepSummary) -> Option<String> {
    if s.first_failing_seeds.is_empty() {
        return None;
    }
    let seeds: Vec<String> = s.first_failing_seeds.iter().map(u64::to_string).collect();
    let named = s.first_failing_seeds.len() as u64;
    let cut = match s.violations > named {
        true => format!(" (first {named} of {} violations)", s.violations),
        false => String::new(),
    };
    Some(format!("failing seeds: {}{cut}", seeds.join(", ")))
}

/// The lines under a sweep's headline, indented; empty for a run that was not a sweep.
pub fn section(outcome: &RunOutcome) -> String {
    let Some(summary) = &outcome.sweep else {
        return String::new();
    };
    std::iter::once(counts_line(summary))
        .chain(seeds_line(summary))
        .map(|line| format!("  {line}\n"))
        .collect()
}
