//! The property sections of the human-readable run report.
use crate::report::{RunOutcome, RunStatus};
use hairspring_props::{PropertyStat, PropertyStatus};

fn label(status: PropertyStatus) -> &'static str {
    match status {
        PropertyStatus::Pass => "PASS",
        PropertyStatus::Violation => "VIOLATION",
        PropertyStatus::Unmet => "UNMET",
        PropertyStatus::Unrated => "UNRATED",
    }
}

fn first_failure_text(stat: &PropertyStat) -> String {
    let Some(first) = &stat.first_failure else {
        return String::new();
    };
    match &first.witness {
        Some(w) => format!(
            "; first failure at seed {} (proc {}, thread {}, ordinal {})",
            first.seed, w.proc, w.thread, w.ord
        ),
        None => format!("; first failure at seed {}", first.seed),
    }
}

fn inconclusive_text(stat: &PropertyStat) -> String {
    match stat.trials_inconclusive_with_failures {
        0 => String::new(),
        n => format!("; failure seen in an inconclusive trial ({n}x), not counted as a finding"),
    }
}

fn row(stat: &PropertyStat) -> String {
    format!(
        "    {:<9} {} '{}'  {}{}{}\n",
        label(stat.status),
        stat.kind.as_str(),
        stat.id,
        stat.rate_text(),
        first_failure_text(stat),
        inconclusive_text(stat)
    )
}

fn table(outcome: &RunOutcome) -> String {
    let mut out = String::new();
    let run = outcome.property_run.clone().unwrap_or_default();
    out.push_str(&format!(
        "  properties ({} trials with events: {} degraded, {} perturbing, {} without an end report):\n",
        run.trials, run.degraded_trials, run.perturbing_trials, run.report_missing_trials
    ));
    for stat in &outcome.properties {
        out.push_str(&row(stat));
    }
    out
}

fn unmet_section(outcome: &RunOutcome) -> String {
    let unmet = outcome.unmet();
    if unmet.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "  UNMET (run-level coverage gaps, not trial failures; judged over clean, unperturbed trials):\n",
    );
    for stat in unmet {
        out.push_str(&format!(
            "    {} '{}' was never satisfied in {} claimable trial(s): {}\n",
            stat.kind.as_str(),
            stat.id,
            stat.trials_claimable,
            stat.msg
        ));
    }
    out
}

/// The property table and the UNMET section; empty for a run whose guest reported no properties. UNMET is a claim
/// about a finished run, so a run that stopped at its first failure does not make it.
pub fn sections(outcome: &RunOutcome) -> String {
    if outcome.properties.is_empty() {
        return String::new();
    }
    let unmet = if outcome.status == RunStatus::Passed {
        unmet_section(outcome)
    } else {
        String::new()
    };
    table(outcome) + &unmet
}
