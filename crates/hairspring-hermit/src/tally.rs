//! Running totals for one run, and the hook that lets a search loop watch trials as they finish.
use crate::exec::Trial;
use crate::property_verdict;
use hairspring_engine::report::RunOutcome;
use hairspring_props::{PropertyStat, PropertyStatus, RunProps, RunSummary, TrialProps};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

/// Watches trials as the run judges them. The guided search (W2) implements this to learn from every trial.
pub trait TrialObserver: Send + Sync {
    /// Called once for every judged trial, in seed order, before the trial is judged. A run stops at its first
    /// failing trial, so trials after it in the same parallel batch are never reported. `props` is what the run counts
    /// for the trial (empty when the guest sent no events, or when the host could not collect them). Confirmation
    /// reruns are not reported. The default does nothing.
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
            let run = std::mem::take(&mut self.props);
            // The report of a killed trial decides nothing: its failures are kept visible but are not findings.
            self.props = match trial.timed_out {
                true => run.add_inconclusive(seed, &props),
                false => run.add(seed, &props),
            };
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

    /// Why the run tested nothing. Only called when every trial was inconclusive.
    fn nothing_tested(&self) -> String {
        if self.budget_cut && self.timed_out == self.trials {
            return "the wall-clock budget ran out before any trial finished; nothing was tested (raise wall_secs or lower trial_timeout_secs)".to_string();
        }
        if self.timed_out < self.trials {
            return format!(
                "all {} trials were inconclusive ({} timed out, {} had unusable events or output); nothing was tested",
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
            return self.with_partial_properties(
                RunOutcome::error(self.nothing_tested()).with_inconclusive(self.inconclusive),
            );
        }
        let ms = started.elapsed().as_millis() as u64;
        let (distinct, work) = self.schedule_stats();
        self.with_properties(
            RunOutcome::passed(self.trials, self.trials, ms)
                .with_inconclusive(self.inconclusive)
                .with_schedule_stats(distinct, work),
        )
    }

    /// Like [`Tally::with_properties`] for a run that did not finish (it stopped at its first failure, or tested
    /// nothing): the trials seen are not the whole run, so a must-hit property that has not been met yet is
    /// `Unrated`, never UNMET.
    pub fn with_partial_properties(&self, outcome: RunOutcome) -> RunOutcome {
        let (stats, summary) = self.properties();
        let stats = stats
            .into_iter()
            .map(|s| match s.status {
                PropertyStatus::Unmet => PropertyStat {
                    status: PropertyStatus::Unrated,
                    ..s
                },
                _ => s,
            })
            .collect();
        match summary.trials {
            0 => outcome,
            _ => outcome.with_properties(stats, summary),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventsFault;
    use hairspring_engine::report::RunStatus;
    use hairspring_props::TrialEvents;

    fn report(id: &str, kind: &str, trues: u64, falses: u64) -> TrialProps {
        let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
        let text = [
            ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
            ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"{kind}","must_hit":true}}"#)),
            ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":null,"first_true":null}}"#, trues + falses)),
            ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
        ]
        .concat();
        TrialEvents::from_output(text.as_bytes()).fold()
    }

    fn trial(props: TrialProps) -> Trial {
        Trial {
            exit_code: Some(0),
            events: Some(props),
            ..Trial::default()
        }
    }

    #[test]
    fn a_trial_with_an_infrastructure_fault_is_left_out_of_the_run_counts() {
        let mut tally = Tally::new(None);
        let faulty = Trial {
            events_error: Some(EventsFault::Infra("disk".into())),
            ..trial(report("a.id", "always", 0, 2))
        };
        tally.on_trial(1, &faulty);
        tally.on_trial(2, &trial(report("a.id", "always", 3, 0)));
        let (stats, summary) = tally.properties();
        assert_eq!(summary.trials, 1);
        assert_eq!(stats[0].status, PropertyStatus::Pass);
        assert_eq!((stats[0].trials, stats[0].trials_false), (1, 0));
    }

    #[test]
    fn a_budget_cut_does_not_hide_that_other_trials_were_unusable() {
        let mut tally = Tally::new(None);
        tally.unusable();
        tally.cut_by_budget();
        assert!(!tally.nothing_tested().contains("wall-clock"));
        let mut only_cut = Tally::new(None);
        only_cut.cut_by_budget();
        assert!(only_cut.nothing_tested().contains("wall-clock"));
    }

    #[test]
    fn a_run_that_tested_nothing_still_carries_its_property_counts_without_unmet() {
        let mut tally = Tally::new(None);
        tally.on_trial(1, &trial(report("s.id", "sometimes", 0, 2)));
        tally.unusable();
        let out = tally.finish(Instant::now());
        assert_eq!(out.status, RunStatus::Error);
        assert!(out.property_run.is_some());
        assert!(out.unmet().is_empty());
    }
}
