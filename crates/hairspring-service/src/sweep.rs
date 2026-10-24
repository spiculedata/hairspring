//! What the service knows about sweep runs: a run's mode, its trial budget as the bound on its rows, and the aggregate
//! of a stored sweep, read back from the rows the server holds and never from what a worker said.
use crate::scan::Flow;
use crate::types::JobPayload;
use crate::{Result, Service};
use hairspring_engine::config::MAX_TRIALS_CAP;
use hairspring_engine::mode::RunMode;
use hairspring_engine::sweep::SweepSummary;
use hairspring_engine::trial_log::ReportedVerdict;
use hairspring_store::{RunRecord, TrialVerdict};

/// The mode a job or run was started in; a payload that predates modes, or cannot be read, is a search.
pub(crate) fn mode_of(payload_json: &str) -> RunMode {
    serde_json::from_str::<JobPayload>(payload_json).map_or(RunMode::Search, |p| p.mode)
}

/// How many trial rows a job may hold: its budget's trial count (at least one trial always runs), and never more than
/// the most any run may be asked for. A payload without a budget is held to the cap alone.
pub(crate) fn trial_limit(payload_json: &str) -> u64 {
    let budget = serde_json::from_str::<JobPayload>(payload_json)
        .ok()
        .and_then(|p| p.budget);
    budget.map_or(MAX_TRIALS_CAP, |b| b.max_trials.clamp(1, MAX_TRIALS_CAP))
}

fn reported(v: TrialVerdict) -> ReportedVerdict {
    match v {
        TrialVerdict::Pass => ReportedVerdict::Pass,
        TrialVerdict::Violation => ReportedVerdict::Violation,
        TrialVerdict::Inconclusive => ReportedVerdict::Inconclusive,
        TrialVerdict::Unusable => ReportedVerdict::Unusable,
        TrialVerdict::Infra => ReportedVerdict::Infra,
    }
}

impl Service {
    /// The aggregate of a stored sweep, counted from its trial rows (their verdicts and seeds only: no digest or text is
    /// read). `None` for a run that was not a sweep, or whose job stored no rows.
    pub(crate) fn sweep_summary(&self, run: &RunRecord) -> Result<Option<SweepSummary>> {
        if !mode_of(&run.config_json).is_sweep() {
            return Ok(None);
        }
        let mut summary = SweepSummary::default();
        self.scan_strip_rows(&run.id, None, |row| {
            summary = std::mem::take(&mut summary).with_trial(reported(row.verdict), row.seed);
            Ok(Flow::Continue)
        })?;
        Ok((summary.trials > 0).then_some(summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_without_a_mode_is_a_search_and_one_that_cannot_be_read_is_too() {
        let old = r#"{"target":"t","seed":1,"budget":null}"#;
        assert_eq!(mode_of(old), RunMode::Search);
        assert_eq!(mode_of("not json"), RunMode::Search);
        let new = r#"{"target":"t","seed":1,"budget":null,"mode":"sweep"}"#;
        assert_eq!(mode_of(new), RunMode::Sweep);
    }

    #[test]
    fn the_row_limit_is_the_budget_clamped_to_at_least_one_and_at_most_the_cap() {
        let with = |trials: u64| {
            format!(
                r#"{{"target":"t","seed":1,"budget":{{"max_steps":1,"max_trials":{trials},"wall_secs":1}}}}"#
            )
        };
        assert_eq!(trial_limit(&with(7)), 7);
        assert_eq!(trial_limit(&with(0)), 1);
        assert_eq!(trial_limit(&with(u64::MAX)), MAX_TRIALS_CAP);
        assert_eq!(
            trial_limit(r#"{"target":"t","seed":1,"budget":null}"#),
            MAX_TRIALS_CAP
        );
        assert_eq!(trial_limit("not json"), MAX_TRIALS_CAP);
    }
}
