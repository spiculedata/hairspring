//! Running totals for one run, and the hook that lets a search loop watch trials as they finish.
use crate::exec::Trial;
use crate::property_verdict;
use crate::schedule_set::ScheduleSet;
use crate::trace::Verdict;
use crate::trial_log::{self, Counted};
use hairspring_engine::report::{FailureReport, RunOutcome};
use hairspring_engine::sweep::{assemble, assemble_error, SweepEnd};
use hairspring_engine::trial_log::TrialReport;
use hairspring_props::limits::MAX_IDS_PER_RUN;
use hairspring_props::{
    PropertyStat, PropertyStatus, RunCatalog, RunProps, RunSummary, TrialProps,
};
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
    schedules: ScheduleSet,
    work: u64,
    /// The wall-clock budget, not the target's own limit, cut the last trial short.
    pub budget_cut: bool,
    timed_out: u64,
    props: RunProps,
    /// The ids the run has seen, which the trials' digests refer to.
    catalog: RunCatalog,
    /// One record per judged trial, in judging order.
    log: Vec<TrialReport>,
    observer: Option<Arc<dyn TrialObserver>>,
    /// What a sweep may hold in memory; `None` for a search, whose log is as long as the search lasted.
    bounds: Option<SweepBounds>,
    /// Bytes of kept detail and output, and of digests, the log holds so far.
    kept_bytes: u64,
    digest_bytes: u64,
}

/// The most bytes a sweep's trial log holds, so a long sweep cannot exhaust the host's memory.
#[derive(Debug, Clone, Copy)]
struct SweepBounds {
    kept: u64,
    digest: u64,
}

impl Tally {
    pub fn new(observer: Option<Arc<dyn TrialObserver>>) -> Tally {
        Tally {
            observer,
            ..Tally::default()
        }
    }

    /// A tally for a sweep, which keeps a record of every trial of its budget until it finishes, within `bounds`. Kept
    /// detail and output beyond `bounds.kept` are left off the records that follow (the records stay). Digests are
    /// never left off: they carry the property statistics, and a catalog update must reach the server before a digest
    /// refers to it. When they reach `bounds.digest` the sweep ends instead ([`Tally::digests_full`]), so it holds
    /// no more than the bound plus one batch of trials.
    pub fn for_sweep(observer: Option<Arc<dyn TrialObserver>>, kept: u64, digest: u64) -> Tally {
        Tally {
            bounds: Some(SweepBounds { kept, digest }),
            ..Tally::new(observer)
        }
    }

    /// Whether a sweep's digests have reached their bound, so it should stop after the trials it has.
    pub fn digests_full(&self) -> bool {
        self.bounds.is_some_and(|b| self.digest_bytes >= b.digest)
    }

    /// Applies a sweep's bound on kept detail and output to the record of one more trial, and counts its digest.
    fn bounded(&mut self, report: TrialReport) -> TrialReport {
        let Some(bounds) = self.bounds else {
            return report;
        };
        self.digest_bytes = self.digest_bytes.saturating_add(report.digest.len() as u64);
        let kept = (report.detail.as_ref().map_or(0, String::len)
            + report.output.as_ref().map_or(0, String::len)) as u64;
        if self.kept_bytes.saturating_add(kept) > bounds.kept {
            return TrialReport {
                detail: None,
                output: None,
                ..report
            };
        }
        self.kept_bytes = self.kept_bytes.saturating_add(kept);
        report
    }

    /// Adds `note` to the reason of the trial numbered `seq`, which must be the one judged last (a note is written right
    /// after its trial is judged, so nothing earlier in the log is ever searched or rebuilt).
    pub fn note_confirmation(&mut self, seq: u64, note: &str) {
        if let Some(last) = self.log.last_mut().filter(|r| r.seq == seq) {
            last.reason = trial_log::with_note(&last.reason, note);
        }
    }

    /// Records a finished trial: tells the observer, folds its properties into the run and logs a record of it for the
    /// store. Called once per trial, in seed order, with the verdict the run reached for it. Trials whose events the
    /// host could not collect are logged but not folded in (their data cannot be trusted); the observer still sees
    /// them.
    pub fn on_trial(&mut self, seq: u64, seed: u64, trial: &Trial, verdict: &Verdict) {
        let counted = property_verdict::for_run(trial);
        if let Some(observer) = &self.observer {
            observer.on_trial(seed, trial, &counted.clone().unwrap_or_default());
        }
        let record = counted.as_ref().map(|props| {
            // The report of a killed trial decides nothing: its failures are kept visible but are not findings.
            let recorded =
                std::mem::take(&mut self.catalog).record(props, trial.timed_out, MAX_IDS_PER_RUN);
            self.catalog = recorded.catalog;
            self.props =
                std::mem::take(&mut self.props).add_digest(seed, &recorded.digest, &self.catalog);
            Counted {
                props,
                digest: recorded.digest,
                updates: recorded.updates,
            }
        });
        if let Some(schedule) = &trial.schedule {
            self.schedules.insert(&schedule.fingerprint);
            self.work = self.work.saturating_add(schedule.syscalls);
        }
        let report = trial_log::build(seq, seed, trial, verdict, record);
        let report = self.bounded(report);
        self.log.push(report);
    }

    /// The records of the trials judged so far.
    #[cfg(test)]
    pub fn trial_log(&self) -> &[TrialReport] {
        &self.log
    }

    /// Attaches the trial records to an outcome that ends the run without going through the property methods.
    pub fn with_log(&self, outcome: RunOutcome) -> RunOutcome {
        // An outcome that already carries the log (a sweep's) is not given a second copy of it.
        match outcome.trial_log.is_empty() {
            true => outcome.with_trial_log(self.log.clone()),
            false => outcome,
        }
    }

    pub fn pass(&mut self) {
        self.trials += 1;
    }

    /// A trial that was a violation, in a sweep, which goes on after it.
    pub fn violation(&mut self) {
        self.trials += 1;
    }

    /// A trial that failed in the host, in a sweep, which goes on after it (up to a limit).
    pub fn infra_trial(&mut self) {
        self.trials += 1;
    }

    /// The outcome of a sweep that ran to the end of its budget (or of its wall-clock time): `Failed` with the report
    /// of its lowest failing seed when any trial violated, `Passed` otherwise, and an error when it rated nothing.
    /// Every judged trial is in the outcome's log, and the properties are those of the trials that ran.
    ///
    /// The tally is consumed and its log moves into the outcome, so the log is never copied (a sweep's log is the
    /// largest thing it holds).
    pub fn finish_sweep(mut self, started: Instant, failure: Option<FailureReport>) -> RunOutcome {
        let (distinct, work) = self.schedule_stats();
        let end = self.sweep_end(started, failure);
        let outcome = assemble(end).with_schedule_stats(distinct, work);
        match outcome.status {
            hairspring_engine::report::RunStatus::Passed => self.with_properties(outcome),
            _ => self.with_partial_properties(outcome),
        }
    }

    /// The outcome of a sweep that had to stop with an error: the trials judged so far are kept.
    pub fn sweep_error(mut self, started: Instant, message: impl Into<String>) -> RunOutcome {
        let (distinct, work) = self.schedule_stats();
        let end = self.sweep_end(started, None);
        let outcome = assemble_error(message, end).with_schedule_stats(distinct, work);
        self.with_partial_properties(outcome)
    }

    /// Takes the log out of the tally.
    fn sweep_end(&mut self, started: Instant, failure: Option<FailureReport>) -> SweepEnd {
        SweepEnd {
            rows: std::mem::take(&mut self.log),
            failure,
            steps_executed: self.trials,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }

    /// The distinct schedules seen and the work done. Once the run has seen more schedules than it remembers, the
    /// first figure is a lower bound ("more than" it): [`Tally::progress_message`] says so.
    pub fn schedule_stats(&self) -> (u64, u64) {
        (self.schedules.distinct(), self.work)
    }

    /// Remembers at most `cap` distinct schedules instead of the default.
    #[cfg(test)]
    fn with_schedule_cap(self, cap: usize) -> Tally {
        Tally {
            schedules: ScheduleSet::with_cap(cap),
            ..self
        }
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
        let base = self.trial_progress(of);
        match self.schedules.is_capped() {
            true => format!(
                "{base}; {} (the run remembers at most {}, and {} trials ran others, so the true number is higher)",
                self.schedules.describe(),
                self.schedules.distinct(),
                self.schedules.unremembered()
            ),
            false => base,
        }
    }

    fn trial_progress(&self, of: u64) -> String {
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
        let ms = started.elapsed().as_millis() as u64;
        let (distinct, work) = self.schedule_stats();
        if self.trials == 0 {
            // The backend always starts a first trial, so this is a guard: a run that judged nothing is never a pass.
            return self.with_partial_properties(
                RunOutcome::error("no trial was judged; nothing was tested").with_elapsed_ms(ms),
            );
        }
        if self.inconclusive == self.trials {
            // Every trial that ran was judged once, so the distinct schedules cannot exceed the trials.
            return self.with_partial_properties(
                RunOutcome::error(self.nothing_tested())
                    .with_trials_attempted(self.trials)
                    .with_inconclusive(self.inconclusive)
                    .with_schedule_stats(distinct, work)
                    .with_elapsed_ms(ms),
            );
        }
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
        self.with_log(match summary.trials {
            0 => outcome,
            _ => outcome.with_properties(stats, summary),
        })
    }

    /// Adds the run's property statistics to an outcome, when the guest reported any.
    pub fn with_properties(&self, outcome: RunOutcome) -> RunOutcome {
        let (stats, summary) = self.properties();
        self.with_log(match summary.trials {
            0 => outcome,
            _ => outcome.with_properties(stats, summary),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventsFault;
    use hairspring_engine::report::RunStatus;
    use hairspring_props::TrialEvents;
    use std::time::Duration;

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

    fn scheduled(fingerprint: &str, syscalls: u64) -> Trial {
        Trial {
            schedule: Some(crate::summary::Summary {
                fingerprint: fingerprint.to_string(),
                syscalls,
                ..crate::summary::Summary::default()
            }),
            ..Trial::default()
        }
    }

    fn run_of(tally: &mut Tally, fingerprints: &[&str]) {
        for (i, fp) in fingerprints.iter().enumerate() {
            tally.on_trial(i as u64, i as u64, &scheduled(fp, 10), &Verdict::Pass);
            tally.pass();
        }
    }

    #[test]
    fn below_the_schedule_cap_the_stats_and_message_are_unchanged() {
        let mut tally = Tally::new(None).with_schedule_cap(4);
        run_of(&mut tally, &["a", "b", "a"]);
        assert_eq!(tally.schedule_stats(), (2, 30));
        assert_eq!(tally.progress_message(10), "3 of 10 seeds explored");
    }

    #[test]
    fn past_the_schedule_cap_the_count_is_a_lower_bound_and_the_message_says_so() {
        let mut tally = Tally::new(None).with_schedule_cap(3);
        run_of(&mut tally, &["a", "b", "c", "d", "e", "d"]);
        let (distinct, work) = tally.schedule_stats();
        assert_eq!(distinct, 3, "no more than the cap is remembered");
        assert_eq!(work, 60, "work keeps adding up past the cap");
        let message = tally.progress_message(10);
        assert!(message.starts_with("6 of 10 seeds explored; more than 3 distinct schedules"));
        assert!(
            message.contains("remembers at most 3, and 3 trials ran others"),
            "{message}"
        );
        assert!(!message.contains("at least"), "{message}");
    }

    #[test]
    fn repeats_of_remembered_schedules_never_trigger_the_at_least_wording() {
        let mut tally = Tally::new(None).with_schedule_cap(2);
        run_of(&mut tally, &["a", "b", "a", "b", "b"]);
        assert_eq!(tally.schedule_stats().0, 2);
        assert!(!tally.progress_message(5).contains("more than"));
    }

    #[test]
    fn a_trial_with_an_infrastructure_fault_is_left_out_of_the_run_counts() {
        let mut tally = Tally::new(None);
        let faulty = Trial {
            events_error: Some(EventsFault::Infra("disk".into())),
            ..trial(report("a.id", "always", 0, 2))
        };
        tally.on_trial(0, 1, &faulty, &Verdict::Inconclusive("x".into()));
        tally.on_trial(1, 2, &trial(report("a.id", "always", 3, 0)), &Verdict::Pass);
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
    fn a_run_where_every_trial_was_inconclusive_counts_them_in_its_error_outcome() {
        let tally = (0..3).fold(Tally::new(None), |mut t, i| {
            match i {
                0 => t.hang(),
                _ => t.unusable(),
            }
            t
        });
        let out = tally.finish(Instant::now());
        assert_eq!(out.status, RunStatus::Error);
        assert_eq!(
            (out.trials, out.inconclusive, out.steps_executed),
            (3, 3, 3)
        );
        let msg = out.error.unwrap();
        assert!(msg.starts_with("all 3 trials were inconclusive"), "{msg}");
    }

    #[test]
    fn an_all_inconclusive_error_keeps_the_schedule_stats_of_the_trials_it_ran() {
        let mut tally = Tally::new(None);
        for (i, (fp, work)) in [("a", 10), ("b", 5), ("a", 7)].into_iter().enumerate() {
            tally.on_trial(
                i as u64,
                i as u64,
                &scheduled(fp, work),
                &Verdict::Inconclusive("x".into()),
            );
            tally.unusable();
        }
        let out = tally.finish(Instant::now());
        assert_eq!(out.status, RunStatus::Error);
        assert_eq!((out.distinct_schedules, out.work), (2, 22));
        assert!(out.distinct_schedules <= out.trials, "{out:?}");
    }

    #[test]
    fn an_all_inconclusive_error_records_how_long_the_run_took() {
        let mut tally = Tally::new(None);
        tally.unusable();
        let started = Instant::now() - Duration::from_millis(250);
        let out = tally.finish(started);
        assert_eq!(out.status, RunStatus::Error);
        assert!(out.elapsed_ms >= 250, "{}", out.elapsed_ms);
    }

    fn failing(stdout_bytes: usize) -> Trial {
        Trial {
            exit_code: Some(1),
            stdout: "x".repeat(stdout_bytes),
            ..Trial::default()
        }
    }

    fn violation() -> Verdict {
        Verdict::Violation("boom".into())
    }

    #[test]
    fn a_search_keeps_every_output_whatever_its_size() {
        let mut tally = Tally::new(None);
        for i in 0..3 {
            tally.on_trial(i, i, &failing(600), &violation());
        }
        assert!(tally.trial_log().iter().all(|r| r.output.is_some()));
    }

    #[test]
    fn a_sweep_keeps_detail_and_output_only_up_to_its_memory_bound() {
        let mut tally = Tally::for_sweep(None, 1000, u64::MAX);
        for i in 0..3 {
            tally.on_trial(i, i, &failing(600), &violation());
        }
        let log = tally.trial_log();
        assert!(log[0].output.is_some(), "the first trial fits the bound");
        for later in &log[1..] {
            assert!(later.output.is_none() && later.detail.is_none());
            assert_eq!(
                later.verdict,
                hairspring_engine::trial_log::ReportedVerdict::Violation
            );
            assert_eq!(later.exit_code, Some(1), "the row itself is kept");
        }
    }

    #[test]
    fn a_sweep_says_when_its_digests_reach_their_bound_and_never_alters_a_digest() {
        let first = trial(report("a.id", "always", 3, 0));
        let mut probe = Tally::for_sweep(None, u64::MAX, u64::MAX);
        probe.on_trial(0, 1, &first, &Verdict::Pass);
        let size = probe.trial_log()[0].digest.len() as u64;
        assert!(
            !probe.digests_full(),
            "no bound is reached with room to spare"
        );

        let mut tally = Tally::for_sweep(None, u64::MAX, size + 1);
        tally.on_trial(0, 1, &first, &Verdict::Pass);
        assert!(!tally.digests_full(), "one digest is below the bound");
        tally.on_trial(1, 2, &first, &Verdict::Pass);
        assert!(tally.digests_full(), "two digests are past it");
        assert_eq!(tally.trial_log()[0], probe.trial_log()[0]);
    }

    #[test]
    fn a_search_never_reports_its_digests_as_full() {
        let mut tally = Tally::new(None);
        tally.on_trial(0, 1, &trial(report("a.id", "always", 3, 0)), &Verdict::Pass);
        assert!(!tally.digests_full());
    }

    #[test]
    fn a_confirmation_note_is_added_to_the_reason_of_the_trial_it_is_about() {
        let mut tally = Tally::for_sweep(None, u64::MAX, u64::MAX);
        tally.on_trial(0, 5, &failing(1), &violation());
        tally.on_trial(1, 6, &failing(1), &violation());
        tally.note_confirmation(1, "rerun reproduced 3/3");
        let log = tally.trial_log();
        assert_eq!(log[0].reason, "boom");
        assert_eq!(log[1].reason, "boom | rerun reproduced 3/3");
    }

    #[test]
    fn a_note_touches_only_the_trial_judged_last() {
        let mut tally = Tally::for_sweep(None, u64::MAX, u64::MAX);
        tally.on_trial(0, 5, &failing(1), &violation());
        tally.on_trial(1, 6, &failing(1), &violation());
        tally.note_confirmation(0, "stale");
        assert_eq!(tally.trial_log()[0].reason, "boom");
        assert_eq!(tally.trial_log()[1].reason, "boom");
    }

    #[test]
    fn finishing_a_sweep_hands_over_its_log_without_copying_it_twice() {
        let mut tally = Tally::for_sweep(None, u64::MAX, u64::MAX);
        tally.on_trial(0, 5, &failing(1), &violation());
        tally.violation();
        let out = tally.finish_sweep(Instant::now(), None);
        assert_eq!(out.trial_log.len(), 1);
    }

    #[test]
    fn a_run_in_which_no_trial_started_is_an_error_not_a_pass() {
        let out = Tally::new(None).finish(Instant::now());
        assert_eq!(out.status, RunStatus::Error, "{out:?}");
        let msg = out.error.unwrap();
        assert!(
            msg.contains("no trial was judged; nothing was tested"),
            "{msg}"
        );
        assert!(!msg.contains("budget"), "no cause is asserted: {msg}");
    }

    #[test]
    fn a_run_that_tested_nothing_still_carries_its_property_counts_without_unmet() {
        let mut tally = Tally::new(None);
        tally.on_trial(
            0,
            1,
            &trial(report("s.id", "sometimes", 0, 2)),
            &Verdict::Pass,
        );
        tally.unusable();
        let out = tally.finish(Instant::now());
        assert_eq!(out.status, RunStatus::Error);
        assert!(out.property_run.is_some());
        assert!(out.unmet().is_empty());
    }
}
