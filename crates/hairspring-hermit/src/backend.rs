use crate::exec::{run_trial, scheduler_flags, ProcessSpec, SpawnError, Trial};
use crate::trace::{behaviour_hash, classify, steps, Verdict};
use crate::HermitSettings;
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Backend, Progress, RunRequest};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::report::{
    DoctorDivergence, DoctorReport, FailureReport, ReplayOutcome, RunOutcome,
};
use hairspring_engine::EngineError;
use std::time::{Duration, Instant};

const BUILD: &str = env!("CARGO_PKG_VERSION");
const CANCELLED: &str = "cancelled";

/// Everything needed to describe one confirmed violation.
#[derive(Clone, Copy)]
struct Found<'a> {
    spec: &'a TargetSpec,
    p: &'a ProcessSpec<'a>,
    req: &'a RunRequest,
    seed: u64,
    trial: &'a Trial,
    reason: &'a str,
    confirmed: u32,
}

/// One finished trial and everything needed to judge it.
struct Judged<'a> {
    spec: &'a TargetSpec,
    p: &'a ProcessSpec<'a>,
    req: &'a RunRequest,
    index: u64,
    seed: u64,
    trial: &'a Trial,
    timeout: Duration,
    clipped: bool,
    started: Instant,
}

pub struct HermitBackend {
    settings: HermitSettings,
}

impl HermitBackend {
    pub fn new(settings: HermitSettings) -> HermitBackend {
        HermitBackend { settings }
    }

    /// Uses `HAIRSPRING_HERMIT` or `hermit` from the PATH.
    pub fn from_env() -> HermitBackend {
        HermitBackend::new(HermitSettings::default())
    }

    fn process<'a>(&self, spec: &'a TargetSpec) -> Result<ProcessSpec<'a>, EngineError> {
        match &spec.kind {
            TargetKind::Process {
                command,
                env,
                cwd,
                hermit_args,
                trial_timeout_secs,
                scheduler,
                hang_is_violation,
                parallelism,
            } => Ok(ProcessSpec {
                command,
                env,
                cwd: cwd.as_deref(),
                hermit_args,
                trial_timeout_secs: *trial_timeout_secs,
                scheduler,
                hang_is_violation: *hang_is_violation,
                parallelism: *parallelism,
            }),
            other => Err(EngineError::BackendUnavailable(other.name().to_string())),
        }
    }

    /// The target's own limit if it set one, otherwise the backend default.
    fn trial_timeout(&self, p: &ProcessSpec) -> Duration {
        p.trial_timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(self.settings.trial_timeout)
    }

    /// Classifies a trial. A timeout is only a violation when the target says a hang is the bug.
    fn verdict(&self, p: &ProcessSpec, t: &Trial, timeout: Duration) -> Verdict {
        match classify(t, timeout.as_secs()) {
            Verdict::Hang(reason) if p.hang_is_violation => Verdict::Violation(reason),
            other => other,
        }
    }

    fn trial(&self, p: &ProcessSpec, seed: u64, timeout: Duration) -> Result<Trial, SpawnError> {
        run_trial(&self.settings, p, seed, timeout)
    }

    /// Re-runs a violating seed and counts how many runs show the same behaviour.
    fn confirm(&self, p: &ProcessSpec, seed: u64, reference: &str, timeout: Duration) -> u32 {
        (0..self.settings.repro_runs)
            .filter(|_| {
                self.trial(p, seed, timeout).is_ok_and(|t| {
                    matches!(self.verdict(p, &t, timeout), Verdict::Violation(_))
                        && behaviour_hash(&t) == reference
                })
            })
            .count() as u32
    }

    /// Runs the trials for the seeds `base + range` at the same time, returning results in range order.
    fn run_batch(
        &self,
        p: &ProcessSpec,
        base: u64,
        range: std::ops::Range<u64>,
        timeout: Duration,
    ) -> Vec<Result<Trial, SpawnError>> {
        if range.end - range.start == 1 {
            return vec![self.trial(p, base.wrapping_add(range.start), timeout)];
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = range
                .map(|i| scope.spawn(move || self.trial(p, base.wrapping_add(i), timeout)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(SpawnError::Other("a trial thread panicked".to_string()))
                    })
                })
                .collect()
        })
    }

    /// Decides what one finished trial means. Returns the run's outcome when the run should end here.
    fn judge(&self, j: &Judged, tally: &mut Tally) -> Option<RunOutcome> {
        tally.record_schedule(j.trial);
        match self.verdict(j.p, j.trial, j.timeout) {
            Verdict::Infra(msg) => Some(infra(msg)),
            Verdict::Hang(_) | Verdict::Violation(_) if j.trial.timed_out && j.clipped => {
                // Out of budget, not out of patience: inconclusive, never a finding.
                tally.cut_by_budget();
                Some(tally.finish(j.started))
            }
            Verdict::Hang(_) => {
                tally.hang();
                None
            }
            Verdict::Violation(reason) => {
                let confirmed = self.confirm(j.p, j.seed, &behaviour_hash(j.trial), j.timeout);
                let failure = self.failure(&Found {
                    spec: j.spec,
                    p: j.p,
                    req: j.req,
                    seed: j.seed,
                    trial: j.trial,
                    reason: &reason,
                    confirmed,
                });
                let ms = j.started.elapsed().as_millis() as u64;
                let (distinct, work) = tally.schedule_stats();
                Some(
                    RunOutcome::failed(j.index + 1, j.index + 1, ms, failure)
                        .with_inconclusive(tally.inconclusive)
                        .with_schedule_stats(distinct, work),
                )
            }
            Verdict::Pass => {
                tally.pass();
                None
            }
        }
    }

    fn failure(&self, found: &Found) -> FailureReport {
        let Found {
            spec,
            p,
            req,
            seed,
            trial: t,
            reason,
            confirmed,
        } = *found;
        let hash = behaviour_hash(t);
        let total = self.settings.repro_runs;
        let mut message = format!(
            "property violated at sched-seed {seed} ({reason}); reproduced {confirmed}/{total}"
        );
        if confirmed < total {
            message.push_str(" - not reproducible: the program or its environment is non-deterministic under hermit");
        }
        let mut bundle = Bundle::new(seed, BUILD, vec![])
            .with_target(req.target.clone())
            .with_backend("hermit")
            .with_param("sched_seed", seed.to_string())
            .with_param("epoch", self.settings.epoch.clone())
            .with_param("heuristic", p.scheduler.effective_heuristic().name())
            .with_param("scheduler", scheduler_flags(p.scheduler).join(" "))
            .with_param("output_hash", hash);
        if let Some(schedule) = &t.schedule {
            bundle = bundle.with_param("schedule_fingerprint", schedule.fingerprint.clone());
        }
        let steps = steps(
            p.command,
            seed,
            &self.settings.epoch,
            p.scheduler.effective_heuristic().name(),
            t,
            None,
        );
        FailureReport {
            target: spec.name.clone(),
            backend: "hermit".to_string(),
            message,
            original_len: steps.len(),
            shrunk_len: steps.len(),
            steps,
            bundle: bundle.encode(),
        }
    }
}

/// What replay says about how closely the rerun matched the recording: the output, and the schedule when both are known.
fn replay_notes(bundle: &Bundle, trial: &Trial) -> Option<String> {
    let yes_no = |same: bool| if same { "yes" } else { "no" };
    let mut notes = Vec::new();
    if let Some(recorded) = bundle.params.get("output_hash") {
        let same = *recorded == behaviour_hash(trial);
        notes.push(if same {
            "output matches the recording: yes".to_string()
        } else {
            "output matches the recording: no (the program or environment changed)".to_string()
        });
    }
    if let (Some(recorded), Some(now)) =
        (bundle.params.get("schedule_fingerprint"), &trial.schedule)
    {
        notes.push(format!(
            "schedule matches the recording: {}",
            yes_no(*recorded == now.fingerprint)
        ));
    }
    (!notes.is_empty()).then(|| notes.join("; "))
}

/// Running totals for one run. Timeouts are counted apart from passes so a run that tested nothing is never a pass.
#[derive(Default)]
struct Tally {
    trials: u64,
    inconclusive: u64,
    /// Fingerprints of the schedules the trials ran, and the total work they did.
    schedules: std::collections::HashSet<String>,
    work: u64,
    /// The wall-clock budget, not the target's own limit, cut the last trial short.
    budget_cut: bool,
}

impl Tally {
    fn pass(&mut self) {
        self.trials += 1;
    }

    fn record_schedule(&mut self, t: &Trial) {
        if let Some(schedule) = &t.schedule {
            self.schedules.insert(schedule.fingerprint.clone());
            self.work = self.work.saturating_add(schedule.syscalls);
        }
    }

    fn schedule_stats(&self) -> (u64, u64) {
        (self.schedules.len() as u64, self.work)
    }

    fn hang(&mut self) {
        self.trials += 1;
        self.inconclusive += 1;
    }

    fn cut_by_budget(&mut self) {
        self.hang();
        self.budget_cut = true;
    }

    fn progress_message(&self, of: u64) -> String {
        match self.inconclusive {
            0 => format!("{} of {of} seeds explored", self.trials),
            n => format!("{} of {of} seeds explored ({n} timed out)", self.trials),
        }
    }

    fn nothing_tested(&self) -> String {
        if self.budget_cut {
            return "the wall-clock budget ran out before any trial finished; nothing was tested (raise wall_secs or lower trial_timeout_secs)".to_string();
        }
        format!(
            "all {} trials timed out; nothing was tested (raise trial_timeout_secs, or set hang_is_violation = true if a hang is the bug)",
            self.trials
        )
    }

    fn finish(&self, started: Instant) -> RunOutcome {
        if self.inconclusive > 0 && self.inconclusive == self.trials {
            return RunOutcome::error(self.nothing_tested()).with_inconclusive(self.inconclusive);
        }
        let ms = started.elapsed().as_millis() as u64;
        let (distinct, work) = self.schedule_stats();
        RunOutcome::passed(self.trials, self.trials, ms)
            .with_inconclusive(self.inconclusive)
            .with_schedule_stats(distinct, work)
    }
}

/// Replay is only meaningful under the scheduling strategy the bundle was recorded with. Bundles from before scheduler
/// options existed recorded no flags and always used the default random strategy.
fn check_same_scheduler(p: &ProcessSpec, bundle: &Bundle) -> Result<(), EngineError> {
    let current = scheduler_flags(p.scheduler).join(" ");
    let recorded = bundle
        .params
        .get("scheduler")
        .cloned()
        .unwrap_or_else(|| "--sched-heuristic=random".to_string());
    if recorded == current {
        return Ok(());
    }
    Err(EngineError::Backend(format!(
        "bundle was recorded with scheduler flags `{recorded}` but target uses `{current}`; replay needs the same scheduler"
    )))
}

fn infra(msg: impl Into<String>) -> RunOutcome {
    RunOutcome::error(msg)
}

impl Backend for HermitBackend {
    fn name(&self) -> &'static str {
        "hermit"
    }

    fn supports(&self, kind: &TargetKind) -> bool {
        matches!(kind, TargetKind::Process { .. })
    }

    fn run(
        &self,
        spec: &TargetSpec,
        req: &RunRequest,
        budget: Budget,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        let p = self.process(spec)?;
        let started = Instant::now();
        let wall = Duration::from_secs(budget.wall_secs);
        let trials = budget.max_trials.max(1);
        let width = u64::from(p.parallelism.max(1));
        let mut tally = Tally::default();
        let mut next = 0u64;
        while next < trials {
            let remaining = wall.saturating_sub(started.elapsed());
            if next > 0 && remaining.is_zero() {
                return Ok(tally.finish(started));
            }
            let allowed = remaining.max(Duration::from_secs(1));
            // True when the wall budget, not the target's own limit, is what cut the trials short.
            let clipped = allowed < self.trial_timeout(&p);
            let timeout = self.trial_timeout(&p).min(allowed);
            let batch = next..trials.min(next + width);
            let results = self.run_batch(&p, req.seed, batch.clone(), timeout);
            // Judge in seed order so the reported failure is the lowest failing seed, as in a sequential run.
            for (i, result) in batch.clone().zip(results) {
                let seed = req.seed.wrapping_add(i);
                let trial = match result {
                    Ok(t) => t,
                    Err(e) => return Ok(infra(e.to_string())),
                };
                let judged = Judged {
                    spec,
                    p: &p,
                    req,
                    index: i,
                    seed,
                    trial: &trial,
                    timeout,
                    clipped,
                    started,
                };
                if let Some(outcome) = self.judge(&judged, &mut tally) {
                    return Ok(outcome);
                }
            }
            next = batch.end;
            if !progress(Progress {
                steps: next,
                trials: next,
                message: tally.progress_message(trials),
            }) {
                return Ok(RunOutcome::error(CANCELLED));
            }
        }
        Ok(tally.finish(started))
    }

    fn replay(&self, spec: &TargetSpec, bundle: &Bundle) -> Result<ReplayOutcome, EngineError> {
        let p = self.process(spec)?;
        check_same_scheduler(&p, bundle)?;
        let seed = bundle
            .params
            .get("sched_seed")
            .and_then(|s| s.parse().ok())
            .unwrap_or(bundle.seed);
        let mut settings = self.settings.clone();
        if let Some(epoch) = bundle.params.get("epoch") {
            settings.epoch = epoch.clone();
        }
        let timeout = self.trial_timeout(&p);
        let trial = run_trial(&settings, &p, seed, timeout)
            .map_err(|e| EngineError::Backend(e.to_string()))?;
        let verdict = self.verdict(&p, &trial, timeout);
        if let Verdict::Infra(m) = &verdict {
            return Err(EngineError::Backend(m.clone()));
        }
        let notes = replay_notes(bundle, &trial);
        let matches = bundle
            .params
            .get("output_hash")
            .map(|h| *h == behaviour_hash(&trial));
        Ok(ReplayOutcome {
            reproduced: matches!(verdict, Verdict::Violation(_)) && matches != Some(false),
            steps: steps(
                p.command,
                seed,
                &settings.epoch,
                p.scheduler.effective_heuristic().name(),
                &trial,
                notes,
            ),
        })
    }

    fn doctor(
        &self,
        spec: &TargetSpec,
        seeds: u32,
        _steps: usize,
    ) -> Result<DoctorReport, EngineError> {
        let p = self.process(spec)?;
        let timeout = self.trial_timeout(&p);
        let backend_err = |e: SpawnError| EngineError::Backend(e.to_string());
        for i in 0..seeds.max(1) {
            let seed = u64::from(i) + 1;
            let (a, b) = (
                self.trial(&p, seed, timeout).map_err(backend_err)?,
                self.trial(&p, seed, timeout).map_err(backend_err)?,
            );
            for t in [&a, &b] {
                if let Verdict::Infra(m) = classify(t, timeout.as_secs()) {
                    return Err(EngineError::Backend(m));
                }
            }
            if behaviour_hash(&a) != behaviour_hash(&b) {
                return Ok(DoctorReport {
                    ok: false,
                    seeds_checked: i + 1,
                    steps_per_seed: 0,
                    divergence: Some(DoctorDivergence { seed, step: 0 }),
                });
            }
        }
        Ok(DoctorReport {
            ok: true,
            seeds_checked: seeds.max(1),
            steps_per_seed: 0,
            divergence: None,
        })
    }
}
