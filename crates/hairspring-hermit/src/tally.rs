//! Running totals for one run, and the hook that lets a search loop watch trials as they finish.
use crate::exec::Trial;
use crate::property_verdict;
use hairspring_engine::report::RunOutcome;
use hairspring_props::{PropertyStat, RunProps, RunSummary, TrialProps};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

/// Watches trials as the run judges them. The guided search (W2) implements this to learn from every trial.
pub trait TrialObserver: Send + Sync {
    /// Called once per trial, in seed order, before the trial is judged. `props` is what the run counts for the trial
    /// (empty when the guest sent no events, or when the host could not collect them). Confirmation reruns are not
    /// reported. The default does nothing.
    fn on_trial(&self, _seed: u64, _trial: &Trial, _props: &TrialProps) {}
}

/// Running totals for one run. Timeouts and unusable trials are counted apart from passes so a run that tested nothing
/// is never a pass.
#[derive(Default)]
pub struct Tally {
    pub trials: u64,
    /// Trials that were neither a pass nor a finding: timeouts, and trials whose events could not be used.
    pub inconclusive: u64,
    /// Fingerprints of the schedules the trials ran, and the total work they did.
    schedules: HashSet<String>,
    work: u64,
    /// The wall-clock budget, not the target's own limit, cut the last trial short.
    pub budget_cut: bool,
    timed_out: u64,
    props: RunProps,
    observer: Option<Arc<dyn TrialObserver>>,
}

impl Tally {
    pub fn new(observer: Option<Arc<dyn TrialObserver>>) -> Tally {
        Tally {
            observer,
            ..Tally::default()
        }
    }

    /// Records a finished trial: tells the observer and folds its properties into the run. Called once per trial, in
    /// seed order. Trials whose events the host could not collect are not folded in (their data cannot be trusted),
    /// but the observer still sees them.
    pub fn on_trial(&mut self, seed: u64, trial: &Trial) {
        let counted = property_verdict::for_run(trial);
        if let Some(observer) = &self.observer {
            observer.on_trial(seed, trial, &counted.clone().unwrap_or_default());
        }
        if let Some(props) = counted {
            self.props = std::mem::take(&mut self.props).add(seed, &props);
        }
        if let Some(schedule) = &trial.schedule {
            self.schedules.insert(schedule.fingerprint.clone());
            self.work = self.work.saturating_add(schedule.syscalls);
        }
    }

    pub fn pass(&mut self) {
        self.trials += 1;
    }

    pub fn schedule_stats(&self) -> (u64, u64) {
        (self.schedules.len() as u64, self.work)
    }

    /// The property statistics so far, and the run-wide counts behind them.
    pub fn properties(&self) -> (Vec<PropertyStat>, RunSummary) {
        (self.props.clone().finish(), self.props.summary())
    }

    pub fn hang(&mut self) {
        self.trials += 1;
        self.inconclusive += 1;
        self.timed_out += 1;
    }

    /// A trial whose events could not be used: no pass, no finding.
    pub fn unusable(&mut self) {
        self.trials += 1;
        self.inconclusive += 1;
    }

    pub fn cut_by_budget(&mut self) {
        self.hang();
        self.budget_cut = true;
    }

    pub fn progress_message(&self, of: u64) -> String {
        match self.inconclusive {
            0 => format!("{} of {of} seeds explored", self.trials),
            n if n == self.timed_out => {
                format!("{} of {of} seeds explored ({n} timed out)", self.trials)
            }
            n => format!("{} of {of} seeds explored ({n} inconclusive)", self.trials),
        }
    }

    fn nothing_tested(&self) -> String {
        if self.budget_cut {
            return "the wall-clock budget ran out before any trial finished; nothing was tested (raise wall_secs or lower trial_timeout_secs)".to_string();
        }
        if self.timed_out < self.trials {
            return format!(
                "all {} trials were inconclusive ({} timed out, {} had unusable guest events); nothing was tested",
                self.trials,
                self.timed_out,
                self.trials - self.timed_out
            );
        }
        format!(
            "all {} trials timed out; nothing was tested (raise trial_timeout_secs, or set hang_is_violation = true if a hang is the bug)",
            self.trials
        )
    }

    /// The outcome of a run that ended without a finding.
    pub fn finish(&self, started: Instant) -> RunOutcome {
        if self.inconclusive > 0 && self.inconclusive == self.trials {
            return RunOutcome::error(self.nothing_tested()).with_inconclusive(self.inconclusive);
        }
        let ms = started.elapsed().as_millis() as u64;
        let (distinct, work) = self.schedule_stats();
        self.with_properties(
            RunOutcome::passed(self.trials, self.trials, ms)
                .with_inconclusive(self.inconclusive)
                .with_schedule_stats(distinct, work),
        )
    }

    /// Adds the run's property statistics to an outcome, when the guest reported any.
    pub fn with_properties(&self, outcome: RunOutcome) -> RunOutcome {
        let (stats, summary) = self.properties();
        match summary.trials {
            0 => outcome,
            _ => outcome.with_properties(stats, summary),
        }
    }
}
