//! The aggregate of a sweep: every trial of the budget counted by how it ended, and how a backend turns a finished
//! sweep into a [`RunOutcome`].
//!
//! Rules, each with a test:
//! - `trials` is the sum of the verdict counts, always.
//! - Rated trials are the passes and the violations. Timeouts, cut-short trials and unusable trials are listed apart and
//!   are never in a denominator; infrastructure failures are listed apart as well. None is ever a pass.
//! - A sweep names at most [`MAX_FIRST_FAILING_SEEDS`] failing seeds, lowest first; the count says how many there were.
//! - A sweep that rated nothing is an error, never a pass.
use crate::report::{FailureReport, RunOutcome};
use crate::trial_log::{decimal_list, ReportedVerdict, TrialReport};
use hairspring_props::run_rate::RunCounts;
use serde::{Deserialize, Serialize};

/// Most failing seeds a summary names.
pub const MAX_FIRST_FAILING_SEEDS: usize = 10;
/// Most violating trials of one sweep that are confirmed by re-running their seed. Further violations are counted and
/// stored like the rest, with a note on their row that they were not confirmed: a sweep over a flaky target would
/// otherwise spend most of its budget re-running failures.
pub const MAX_CONFIRMED_VIOLATIONS: usize = 10;
/// Consecutive infrastructure failures after which a sweep gives up: the host, not the target, is broken.
pub const MAX_CONSECUTIVE_INFRA: u32 = 3;
/// Most bytes of kept detail and output a sweep holds in memory before its remaining trials keep none.
pub const MAX_KEPT_BYTES_IN_MEMORY: u64 = 64 * 1024 * 1024;
/// Most bytes of trial digests a sweep holds in memory. Equal to the server's per-run allowance of stored digests.
pub const MAX_DIGEST_BYTES_IN_MEMORY: u64 = 64 * 1024 * 1024;

/// How the trials of a sweep ended, counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepSummary {
    /// Every judged trial.
    pub trials: u64,
    pub passes: u64,
    pub violations: u64,
    /// Timed out, or cut short by the wall-clock budget.
    pub inconclusive: u64,
    /// Finished, but their events could not be used.
    pub unusable: u64,
    /// The host or Hermit failed.
    pub infra: u64,
    /// The first failing seeds in judging order, at most [`MAX_FIRST_FAILING_SEEDS`]. Trial n runs at the first seed plus
    /// n (wrapping), so this is also ascending seed order unless the seeds pass the largest one and wrap to 0; "lowest
    /// failing seed" means the first one judged. The CLI refuses a sweep that would wrap.
    #[serde(with = "decimal_list")]
    pub first_failing_seeds: Vec<u64>,
}

impl SweepSummary {
    /// The summary with one more trial counted.
    pub fn with_trial(self, verdict: ReportedVerdict, seed: u64) -> SweepSummary {
        let is = |v: ReportedVerdict| u64::from(v == verdict);
        let mut first_failing_seeds = self.first_failing_seeds;
        if verdict == ReportedVerdict::Violation
            && first_failing_seeds.len() < MAX_FIRST_FAILING_SEEDS
        {
            first_failing_seeds.push(seed);
        }
        SweepSummary {
            trials: self.trials.saturating_add(1),
            passes: self.passes.saturating_add(is(ReportedVerdict::Pass)),
            violations: self
                .violations
                .saturating_add(is(ReportedVerdict::Violation)),
            inconclusive: self
                .inconclusive
                .saturating_add(is(ReportedVerdict::Inconclusive)),
            unusable: self.unusable.saturating_add(is(ReportedVerdict::Unusable)),
            infra: self.infra.saturating_add(is(ReportedVerdict::Infra)),
            first_failing_seeds,
        }
    }

    /// The summary of a log of trial records.
    pub fn from_trials(rows: &[TrialReport]) -> SweepSummary {
        rows.iter().fold(SweepSummary::default(), |summary, row| {
            summary.with_trial(row.verdict, row.seed)
        })
    }

    /// Passes and violations: the trials a rate is computed over.
    pub fn rated(&self) -> u64 {
        self.passes.saturating_add(self.violations)
    }

    /// Trials that ended without a verdict about the program (timeouts and unusable trials).
    pub fn not_rated(&self) -> u64 {
        self.inconclusive.saturating_add(self.unusable)
    }

    /// Whether the counts agree with each other: `trials` is their sum, and no more failing seeds are named than there
    /// were violations or than a summary may name.
    pub fn is_consistent(&self) -> bool {
        let sum = [
            self.passes,
            self.violations,
            self.inconclusive,
            self.unusable,
            self.infra,
        ]
        .iter()
        .map(|n| u128::from(*n))
        .sum::<u128>();
        let named = self.first_failing_seeds.len();
        sum == u128::from(self.trials)
            && named <= MAX_FIRST_FAILING_SEEDS
            && (named as u128) <= u128::from(self.violations)
    }

    /// The counts a rate is stated from, `None` when nothing was judged or the counts contradict each other.
    pub fn counts(&self) -> Option<RunCounts> {
        if self.trials == 0 || !self.is_consistent() {
            return None;
        }
        RunCounts::new(self.violations, self.rated(), self.not_rated(), self.infra).ok()
    }
}

/// A finished sweep, as a backend hands it over.
pub struct SweepEnd {
    /// One record per judged trial, in judging order.
    pub rows: Vec<TrialReport>,
    /// The report of the lowest failing seed; `Some` exactly when a trial was a violation.
    pub failure: Option<FailureReport>,
    pub steps_executed: u64,
    pub elapsed_ms: u64,
}

/// The outcome of a sweep: `Failed` when any trial was a violation, `Error` when no trial was rated, `Passed`
/// otherwise. The outcome never says it stopped at its first failure, and carries the summary and the trial records.
pub fn assemble(end: SweepEnd) -> RunOutcome {
    let summary = SweepSummary::from_trials(&end.rows);
    let trials = summary.trials;
    let base = match end.failure {
        Some(failure) => RunOutcome::failed(end.steps_executed, trials, end.elapsed_ms, failure),
        None if summary.rated() == 0 => RunOutcome::error(nothing_tested(&summary))
            .with_trials_attempted(trials)
            .with_elapsed_ms(end.elapsed_ms),
        None => RunOutcome::passed(end.steps_executed, trials, end.elapsed_ms),
    };
    base.with_inconclusive(summary.not_rated())
        .with_trial_log(end.rows)
        .with_sweep(summary)
}

/// The outcome of a sweep that had to end in an error (the host failed, or the sweep was told to stop): the trials
/// judged so far are kept.
pub fn assemble_error(message: impl Into<String>, end: SweepEnd) -> RunOutcome {
    let summary = SweepSummary::from_trials(&end.rows);
    RunOutcome::error(message)
        .with_trials_attempted(summary.trials)
        .with_elapsed_ms(end.elapsed_ms)
        .with_inconclusive(summary.not_rated())
        .with_trial_log(end.rows)
        .with_sweep(summary)
}

/// Why a sweep that judged trials, none of them a pass or a violation, tested nothing.
fn nothing_tested(summary: &SweepSummary) -> String {
    format!(
        "no trial of the sweep was rated ({} judged: {} timed out or were cut short, {} had unusable output, {} failed in the host); nothing was tested",
        summary.trials, summary.inconclusive, summary.unusable, summary.infra
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::RunStatus;

    fn row(seq: u64, seed: u64, verdict: ReportedVerdict) -> TrialReport {
        TrialReport::without_events(seq, seed, verdict, "", 1)
    }

    fn rows(verdicts: &[ReportedVerdict]) -> Vec<TrialReport> {
        verdicts
            .iter()
            .enumerate()
            .map(|(i, v)| row(i as u64, 100 + i as u64, *v))
            .collect()
    }

    fn failure() -> FailureReport {
        FailureReport {
            target: "svc".into(),
            backend: "sim".into(),
            message: "boom".into(),
            original_len: 1,
            shrunk_len: 1,
            steps: Vec::new(),
            bundle: "b".into(),
        }
    }

    fn end(rows: Vec<TrialReport>, failure: Option<FailureReport>) -> SweepEnd {
        SweepEnd {
            steps_executed: rows.len() as u64,
            rows,
            failure,
            elapsed_ms: 5,
        }
    }

    use ReportedVerdict::{Inconclusive, Infra, Pass, Unusable, Violation};

    #[test]
    fn every_verdict_is_counted_apart_and_the_trials_are_their_sum() {
        let s = SweepSummary::from_trials(&rows(&[
            Pass,
            Pass,
            Violation,
            Inconclusive,
            Unusable,
            Infra,
            Pass,
        ]));
        assert_eq!(
            (s.passes, s.violations, s.inconclusive, s.unusable, s.infra),
            (3, 1, 1, 1, 1)
        );
        assert_eq!(s.trials, 7);
        assert!(s.is_consistent());
    }

    #[test]
    fn rated_trials_are_passes_and_violations_and_nothing_else() {
        let s = SweepSummary::from_trials(&rows(&[Pass, Violation, Inconclusive, Unusable, Infra]));
        assert_eq!((s.rated(), s.not_rated()), (2, 2));
        let counts = s.counts().unwrap();
        assert_eq!(
            (counts.hits(), counts.rated(), counts.excluded()),
            (1, 2, 3)
        );
    }

    #[test]
    fn failing_seeds_are_listed_in_judging_order_and_capped() {
        let verdicts = vec![Violation; MAX_FIRST_FAILING_SEEDS + 5];
        let s = SweepSummary::from_trials(&rows(&verdicts));
        assert_eq!(s.violations, (MAX_FIRST_FAILING_SEEDS + 5) as u64);
        assert_eq!(s.first_failing_seeds.len(), MAX_FIRST_FAILING_SEEDS);
        assert_eq!(s.first_failing_seeds[0], 100);
        assert!(s.is_consistent());
    }

    #[test]
    fn a_summary_that_contradicts_itself_is_not_consistent_and_has_no_counts() {
        let summaries = [
            SweepSummary {
                trials: 5,
                passes: 1,
                ..SweepSummary::default()
            },
            SweepSummary {
                trials: 1,
                violations: 1,
                first_failing_seeds: vec![1, 2],
                ..SweepSummary::default()
            },
            SweepSummary {
                trials: u64::MAX,
                passes: u64::MAX,
                violations: 1,
                ..SweepSummary::default()
            },
        ];
        for s in summaries {
            assert!(!s.is_consistent(), "{s:?}");
            assert!(s.counts().is_none(), "{s:?}");
        }
    }

    #[test]
    fn an_empty_sweep_has_no_counts() {
        assert!(SweepSummary::default().counts().is_none());
    }

    #[test]
    fn a_summary_round_trips_with_seeds_as_decimal_strings() {
        let s = SweepSummary {
            trials: 1,
            violations: 1,
            first_failing_seeds: vec![u64::MAX],
            ..SweepSummary::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains(&format!("\"first_failing_seeds\":[\"{}\"]", u64::MAX)),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<SweepSummary>(&json).unwrap(), s);
    }

    #[test]
    fn a_sweep_with_a_violation_is_failed_but_did_not_stop_at_its_first_failure() {
        let log = rows(&[Pass, Pass, Violation, Pass]);
        let out = assemble(end(log.clone(), Some(failure())));
        assert_eq!(out.status, RunStatus::Failed);
        assert!(!out.stopped_at_first_failure);
        assert_eq!((out.trials, out.steps_executed, out.elapsed_ms), (4, 4, 5));
        assert_eq!(out.trial_log, log);
        assert_eq!(out.sweep.as_ref().unwrap().violations, 1);
        assert!(out.failure.is_some());
    }

    #[test]
    fn a_clean_sweep_passes_and_counts_unrated_trials_as_inconclusive() {
        let out = assemble(end(rows(&[Pass, Pass, Inconclusive, Unusable]), None));
        assert_eq!(out.status, RunStatus::Passed);
        assert_eq!((out.trials, out.inconclusive), (4, 2));
        assert!(out.failure.is_none() && out.error.is_none());
    }

    #[test]
    fn a_sweep_that_rated_nothing_is_an_error_that_keeps_its_records() {
        let out = assemble(end(rows(&[Inconclusive, Unusable, Infra]), None));
        assert_eq!(out.status, RunStatus::Error);
        assert!(out.error.as_deref().unwrap().contains("nothing was tested"));
        assert_eq!((out.trials, out.inconclusive), (3, 2));
        assert_eq!(out.trial_log.len(), 3);
        assert_eq!(out.sweep.as_ref().unwrap().infra, 1);
    }

    #[test]
    fn a_sweep_with_no_trials_is_an_error_not_a_pass() {
        let out = assemble(end(Vec::new(), None));
        assert_eq!(out.status, RunStatus::Error);
    }

    #[test]
    fn an_error_ending_keeps_the_trials_judged_so_far() {
        let out = assemble_error("hermit failed", end(rows(&[Pass, Infra]), None));
        assert_eq!(out.status, RunStatus::Error);
        assert_eq!(out.error.as_deref(), Some("hermit failed"));
        assert_eq!((out.trials, out.trial_log.len()), (2, 2));
        assert_eq!(out.sweep.as_ref().unwrap().passes, 1);
    }
}
