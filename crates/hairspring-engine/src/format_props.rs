//! The property sections of the human-readable run report.
use crate::report::{RunOutcome, RunStatus};
use hairspring_props::honesty::rate_cell;
use hairspring_props::{limits, PropertyStat, PropertyStatus};

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

/// `stopped` is true for a run that stopped at its first failure: no property of such a run has a rate.
fn row(stat: &PropertyStat, stopped: bool) -> String {
    format!(
        "    {:<9} {} '{}'  {}{}{}\n",
        label(stat.status),
        stat.kind.as_str(),
        stat.id,
        rate_cell(stat, stopped).text(),
        first_failure_text(stat),
        inconclusive_text(stat)
    )
}

fn table(outcome: &RunOutcome) -> String {
    let mut out = String::new();
    let run = outcome.property_run.clone().unwrap_or_default();
    let stopped = outcome.status == RunStatus::Failed;
    out.push_str(&format!(
        "  properties ({} trials with events: {} degraded, {} perturbing, {} without an end report, {} timed out):\n",
        run.trials,
        run.degraded_trials,
        run.perturbing_trials,
        run.report_missing_trials,
        run.inconclusive_trials
    ));
    if run.ids_dropped > 0 {
        out.push_str(&format!(
            "  WARNING: {} id(s) dropped: the run-wide id limit ({}) was reached, so some properties are not shown\n",
            run.ids_dropped,
            if run.id_cap == 0 {
                limits::MAX_IDS_PER_RUN
            } else {
                run.id_cap
            }
        ));
    }
    for stat in &outcome.properties {
        out.push_str(&row(stat, stopped));
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
