//! Sweep mode for process targets: every trial of the budget runs, and a violation is confirmed but does not end the run.
//!
//! The loop is the search loop's, trial for trial, so batches, seeds, timeouts and the wall-clock budget behave as they
//! do there. What differs:
//! - a violating trial is confirmed by re-running its seed (the same check as in search), the note goes on its row, and
//!   the sweep goes on; only the first [`MAX_CONFIRMED_VIOLATIONS`] violations are confirmed, so a flaky target cannot
//!   spend its budget on reruns;
//! - the report (bundle and trace) is built for the lowest failing seed only;
//! - a trial that failed in the host is stored as an `infra` row and the sweep goes on, unless
//!   [`MAX_CONSECUTIVE_INFRA`] of them follow one another, which means the host is broken;
//! - nothing but a pass or a violation is ever rated.
use super::{Confirmation, FailureKind, Found, HermitBackend, Judged, CANCELLED};
use crate::tally::Tally;
use crate::trace::Verdict;
use hairspring_engine::backend::{Progress, RunRequest};
use hairspring_engine::config::{Budget, TargetSpec};
use hairspring_engine::report::{FailureReport, RunOutcome};
use hairspring_engine::sweep::{MAX_CONFIRMED_VIOLATIONS, MAX_CONSECUTIVE_INFRA};
use hairspring_engine::EngineError;
use std::time::{Duration, Instant};

/// What a sweep has seen so far, besides its tally.
struct Findings {
    /// The report of the lowest failing seed.
    failure: Option<FailureReport>,
    /// Violations confirmed by a rerun so far.
    confirmed: usize,
    /// Trials in a row that failed in the host.
    infra_run: u32,
    /// The sweep's wall-clock budget, which confirmations are held to as well.
    wall: Duration,
}

impl Findings {
    fn new(wall: Duration) -> Findings {
        Findings {
            failure: None,
            confirmed: 0,
            infra_run: 0,
            wall,
        }
    }
}

const OUT_OF_BUDGET_NOTE: &str = "not confirmed: out of budget";
const CANCELLED_NOTE: &str = "not confirmed: cancelled";

/// What stands in for the reruns of a violation that was not confirmed, so its report does not call it
/// non-deterministic: none ran, and `why` is said where a host problem would be.
fn skipped(total: u32, why: &str) -> Confirmation {
    Confirmation {
        reproduced: 0,
        inconclusive: total,
        first_error: Some(format!("not rerun: {why}")),
    }
}

/// What a row says about the reruns of its seed: how many gave the same output and failed the same properties.
fn confirmation_note(c: &Confirmation, total: u32) -> String {
    let inconclusive = match c.inconclusive {
        0 => String::new(),
        n => format!(", {n} inconclusive"),
    };
    format!("same-output reruns {}/{total}{inconclusive}", c.reproduced)
}

fn not_confirmed_note() -> String {
    format!("not confirmed: a sweep reruns only its first {MAX_CONFIRMED_VIOLATIONS} violations")
}

impl HermitBackend {
    pub(super) fn run_sweep(
        &self,
        spec: &TargetSpec,
        req: &RunRequest,
        budget: Budget,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        let p = self.process(spec)?;
        self.check_events_setup(&p)?;
        let environment = self.environment(&p);
        let started = Instant::now();
        let wall = Duration::from_secs(budget.wall_secs);
        let trials = budget.max_trials.max(1);
        let width = u64::from(p.parallelism.max(1));
        let mut tally = Tally::for_sweep(
            self.observer.clone(),
            self.settings.sweep_kept_bytes,
            self.settings.sweep_digest_bytes,
        );
        let mut found = Findings::new(wall);
        let mut next = 0u64;
        while next < trials {
            let remaining = wall.saturating_sub(started.elapsed());
            if next > 0 && remaining.is_zero() {
                break;
            }
            let allowed = remaining.max(Duration::from_secs(1));
            // True when the wall budget, not the target's own limit, is what cuts the trials short.
            let clipped = allowed < self.trial_timeout(&p);
            let timeout = self.trial_timeout(&p).min(allowed);
            let batch = next..trials.min(next + width);
            let results = self.run_batch(&p, req.seed, batch.clone(), timeout);
            // Judged in seed order, so rows, confirmations and the reported failure follow the seeds.
            for (i, result) in batch.clone().zip(results) {
                let trial = match result {
                    Ok(t) => t,
                    Err(e) => return Ok(tally.sweep_error(started, e.to_string())),
                };
                let judged = Judged {
                    spec,
                    p: &p,
                    req,
                    index: i,
                    seed: req.seed.wrapping_add(i),
                    trial: &trial,
                    timeout,
                    clipped,
                    started,
                    environment: &environment,
                };
                if let Some(why) = self.judge_sweep(&judged, &mut tally, &mut found, progress) {
                    return Ok(tally.sweep_error(started, why));
                }
            }
            // Out of wall-clock budget, or holding as many digests as a sweep may: the trials judged are the sweep.
            if tally.budget_cut || tally.digests_full() {
                break;
            }
            next = batch.end;
            let keep_going = progress(Progress {
                steps: next,
                trials: next,
                message: tally.progress_message(trials),
            });
            if !keep_going {
                return Ok(tally.sweep_error(started, CANCELLED));
            }
        }
        Ok(tally.finish_sweep(started, found.failure))
    }

    /// Judges one finished trial of a sweep. `Some(why)` says the sweep cannot go on.
    fn judge_sweep(
        &self,
        j: &Judged,
        tally: &mut Tally,
        found: &mut Findings,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Option<String> {
        let cut_by_budget = j.trial.timed_out && j.clipped;
        let verdict = match self.verdict(j.p, j.trial, j.timeout) {
            // Out of budget, not out of patience: inconclusive, never a finding.
            Verdict::Hang(r) | Verdict::Violation(r) if cut_by_budget => Verdict::Hang(r),
            other => other,
        };
        tally.on_trial(j.index, j.seed, j.trial, &verdict);
        found.infra_run = match verdict {
            Verdict::Infra(_) => found.infra_run + 1,
            _ => 0,
        };
        match verdict {
            Verdict::Infra(msg) => {
                tally.infra_trial();
                (found.infra_run >= MAX_CONSECUTIVE_INFRA).then(|| {
                    format!("{MAX_CONSECUTIVE_INFRA} consecutive trials failed in the host; the last: {msg}")
                })
            }
            Verdict::Inconclusive(_) => {
                tally.unusable();
                None
            }
            Verdict::Hang(_) if cut_by_budget => {
                tally.cut_by_budget();
                None
            }
            Verdict::Hang(_) => {
                tally.hang();
                None
            }
            Verdict::Violation(reason) => {
                self.sweep_violation(j, tally, found, FailureKind::Violation, &reason, progress)
            }
            Verdict::Deadlock(reason) => {
                self.sweep_violation(j, tally, found, FailureKind::Deadlock, &reason, progress)
            }
            Verdict::Pass => {
                tally.pass();
                None
            }
        }
    }

    /// Counts a violation, confirms it by rerun when the sweep still confirms, and keeps the report of the first one.
    ///
    /// A confirmation is a few more trials, so before each one the sweep checks that it still has wall-clock time and
    /// has not been cancelled (it asks the progress callback, which is how a cancel is delivered). Without that, ten
    /// violations in one batch could run `repro_runs` timeouts each past the budget. `Some(why)` says the sweep was
    /// cancelled.
    fn sweep_violation(
        &self,
        j: &Judged,
        tally: &mut Tally,
        found: &mut Findings,
        kind: FailureKind,
        reason: &str,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Option<String> {
        tally.violation();
        let total = self.settings.repro_runs;
        let mut stop = None;
        let confirmed = if found.confirmed >= MAX_CONFIRMED_VIOLATIONS {
            tally.note_confirmation(j.index, &not_confirmed_note());
            return None;
        } else if j.started.elapsed() >= found.wall {
            tally.note_confirmation(j.index, OUT_OF_BUDGET_NOTE);
            skipped(total, "out of budget")
        } else if !progress(Progress {
            steps: j.index + 1,
            trials: j.index + 1,
            message: format!("confirming the violation at seed {}", j.seed),
        }) {
            tally.note_confirmation(j.index, CANCELLED_NOTE);
            stop = Some(CANCELLED.to_string());
            skipped(total, "cancelled")
        } else {
            found.confirmed += 1;
            let c = self.confirm_violation(j.p, j.seed, j.trial, kind, j.timeout);
            tally.note_confirmation(j.index, &confirmation_note(&c, total));
            c
        };
        if found.failure.is_none() {
            found.failure = Some(self.failure(&Found {
                spec: j.spec,
                p: j.p,
                req: j.req,
                seed: j.seed,
                trial: j.trial,
                kind,
                reason,
                confirmed: &confirmed,
                environment: j.environment,
            }));
        }
        stop
    }
}
