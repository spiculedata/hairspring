//! What the service knows about sweep runs: a run's mode, its trial budget as the bound on its rows, and the aggregate
//! of a stored sweep, read back from the rows the server holds and never from what a worker said.
use crate::scan::Flow;
use crate::types::JobPayload;
use crate::{Result, RunCompletion, Service};
use hairspring_engine::config::MAX_TRIALS_CAP;
use hairspring_engine::mode::RunMode;
use hairspring_engine::sweep::SweepSummary;
use hairspring_engine::trial_log::ReportedVerdict;
use hairspring_store::{RunRecord, TrialVerdict};

/// The mode a job or run was started in; a payload that predates modes, or cannot be read, is a search.
pub(crate) fn mode_of(payload_json: &str) -> RunMode {
    serde_json::from_str::<JobPayload>(payload_json).map_or(RunMode::Search, |p| p.mode)
}

/// What the rows of a job must look like, from the job itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowRules {
    /// How many trial rows the job may hold: its budget's trial count (at least one trial always runs), and never more
    /// than the most any run may be asked for. A payload without a budget is held to the cap alone.
    pub limit: u64,
    /// For a sweep, the seed of trial 0. Every backend runs trial `n` at this seed plus `n` (wrapping), so a row that
    /// says otherwise did not come from this run, and a worker could use it to name an arbitrary failing seed. A search
    /// is not held to it (the rows of a search are optional extras, and older tests and tools send any seed).
    pub seed_base: Option<u64>,
}

impl RowRules {
    /// The seed trial `seq` must carry, when the job fixes it.
    pub(crate) fn seed_of(&self, seq: u64) -> Option<u64> {
        self.seed_base.map(|base| base.wrapping_add(seq))
    }
}

pub(crate) fn row_rules(payload_json: &str) -> RowRules {
    let payload = serde_json::from_str::<JobPayload>(payload_json).ok();
    let limit = payload
        .as_ref()
        .and_then(|p| p.budget)
        .map_or(MAX_TRIALS_CAP, |b| b.max_trials.clamp(1, MAX_TRIALS_CAP));
    let seed_base = payload.filter(|p| p.mode.is_sweep()).map(|p| p.seed);
    RowRules { limit, seed_base }
}

/// Whether the run holds fewer trials than its budget asked for. `completion_for` labels any run with a stand-in digest
/// "digest allowance used up", even one that also stopped short, so an incomplete run is held back here as well when it
/// is short: only a full-budget run whose sole problem is capped digests keeps its summary.
fn short_of_budget(run: &RunRecord) -> bool {
    serde_json::from_str::<JobPayload>(&run.config_json)
        .ok()
        .and_then(|p| p.budget)
        .is_some_and(|b| run.trials < b.max_trials.max(1))
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
    /// read). `None` for a run that was not a sweep, whose job stored no rows, or that ended early
    /// ([`RunCompletion::EndedEarly`]): it stopped short of its budget without reaching a limit (a worker that predates
    /// sweeps stops at the first failure), so what its rows hold depends on how it stopped and is not the figure of a
    /// sweep. A run that is incomplete only because digests were replaced by stand-ins
    /// ([`RunCompletion::DigestAllowanceUsedUp`]) keeps its summary: the summary reads verdicts and seeds, which the
    /// stand-ins leave alone, but only when the run also ran its whole trial budget (see [`short_of_budget`]).
    pub(crate) fn sweep_summary(&self, run: &RunRecord) -> Result<Option<SweepSummary>> {
        if !mode_of(&run.config_json).is_sweep() {
            return Ok(None);
        }
        if run.ingest_incomplete
            && (short_of_budget(run) || self.completion_for(run)? == RunCompletion::EndedEarly)
        {
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
        assert_eq!(row_rules(&with(7)).limit, 7);
        assert_eq!(row_rules(&with(0)).limit, 1);
        assert_eq!(row_rules(&with(u64::MAX)).limit, MAX_TRIALS_CAP);
        assert_eq!(
            row_rules(r#"{"target":"t","seed":1,"budget":null}"#).limit,
            MAX_TRIALS_CAP
        );
        assert_eq!(row_rules("not json").limit, MAX_TRIALS_CAP);
    }

    #[test]
    fn only_a_sweep_fixes_the_seed_of_each_row_and_it_wraps() {
        let search = row_rules(r#"{"target":"t","seed":5,"budget":null}"#);
        assert_eq!(search.seed_of(3), None);
        let sweep = row_rules(&format!(
            r#"{{"target":"t","seed":{},"budget":null,"mode":"sweep"}}"#,
            u64::MAX
        ));
        assert_eq!(sweep.seed_of(0), Some(u64::MAX));
        assert_eq!(sweep.seed_of(2), Some(1));
    }
}
