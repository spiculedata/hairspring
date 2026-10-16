use crate::events;
use crate::exec::{run_trial, scheduler_flags, Dirs, ProcessSpec, SpawnError, Trial};
use crate::property_verdict::{self, FailureKey};
use crate::summary_dir::PrivateDir;
use crate::tally::{Tally, TrialObserver};
use crate::trace::{behaviour_hash, classify, steps, Verdict};
use crate::HermitSettings;
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Backend, Progress, RunRequest};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::report::{
    DoctorDivergence, DoctorReport, FailureReport, ReplayOutcome, RunOutcome,
};
use hairspring_engine::EngineError;
use hairspring_props::AssertKind;
use std::sync::Arc;
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
    /// Where Hermit writes each trial's summary; `None` turns schedule fingerprints off (no directory configured, or
    /// one that could not be made private).
    summaries: Option<PrivateDir>,
    /// Where the events file's per-trial directories go when `events_dir` is set; otherwise `summaries` is used.
    events_scratch: Option<PrivateDir>,
    /// Told about every trial of every run; `None` does nothing.
    observer: Option<Arc<dyn TrialObserver>>,
}

impl HermitBackend {
    pub fn new(settings: HermitSettings) -> HermitBackend {
        let summaries = settings
            .summary_dir
            .as_deref()
            .and_then(|base| PrivateDir::create(base).ok());
        let events_scratch = settings
            .events_dir
            .as_deref()
            .and_then(|base| PrivateDir::create(base).ok());
        HermitBackend {
            settings,
            summaries,
            events_scratch,
            observer: None,
        }
    }

    /// Reports every trial of every run to `observer` (see [`TrialObserver`]).
    #[must_use]
    pub fn with_observer(self, observer: Arc<dyn TrialObserver>) -> HermitBackend {
        HermitBackend {
            observer: Some(observer),
            ..self
        }
    }

    fn dirs(&self) -> Dirs<'_> {
        Dirs {
            summaries: self.summaries.as_ref(),
            events: self.events_scratch.as_ref().or(self.summaries.as_ref()),
        }
    }

    /// The events transport this backend uses, as recorded in bundles.
    fn events_transport(&self) -> &'static str {
        if self.settings.events_file {
            "file"
        } else {
            "stream"
        }
    }

    /// Refuses to start a run or replay whose events file cannot work, before any trial is spent.
    fn check_events_setup(&self, p: &ProcessSpec) -> Result<(), EngineError> {
        match events::config_problem(
            self.settings.events_file,
            p.hermit_args,
            self.dirs().events.is_some(),
        ) {
            Some(problem) => Err(EngineError::Backend(problem)),
            None => Ok(()),
        }
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

    /// Runs one trial of a process target at `seed` and returns everything captured, without judging it. This is what
    /// the engine's run loop does per seed, exposed for tooling and tests that need the raw outcome (output, exit
    /// status, schedule, guest events).
    pub fn run_one(&self, spec: &TargetSpec, seed: u64) -> Result<Trial, EngineError> {
        let p = self.process(spec)?;
        self.trial(&p, seed, self.trial_timeout(&p))
            .map_err(|e| EngineError::Backend(e.to_string()))
    }

    fn trial(&self, p: &ProcessSpec, seed: u64, timeout: Duration) -> Result<Trial, SpawnError> {
        run_trial(&self.settings, p, seed, timeout, self.dirs())
    }

    /// Re-runs a violating seed and counts how many runs reproduce it: the same behaviour hash (which includes the
    /// guest's reported events) and the same failing properties, by id and kind. A rerun that fails a different
    /// property is a different bug, not a reproduction.
    fn confirm(&self, p: &ProcessSpec, seed: u64, reference: &Trial, timeout: Duration) -> u32 {
        let hash = behaviour_hash(reference);
        let failing = property_verdict::failing_keys(reference);
        (0..self.settings.repro_runs)
            .filter(|_| {
                self.trial(p, seed, timeout).is_ok_and(|t| {
                    matches!(self.verdict(p, &t, timeout), Verdict::Violation(_))
                        && behaviour_hash(&t) == hash
                        && property_verdict::failing_keys(&t) == failing
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
        tally.on_trial(j.seed, j.trial);
        match self.verdict(j.p, j.trial, j.timeout) {
            Verdict::Infra(msg) => Some(infra(msg)),
            Verdict::Inconclusive(_) => {
                tally.unusable();
                None
            }
            Verdict::Hang(_) | Verdict::Violation(_) if j.trial.timed_out && j.clipped => {
                // Out of budget, not out of patience: inconclusive, never a finding. The rest of the batch is still
                // judged first, because a violation among trials that already ran must not be thrown away.
                tally.cut_by_budget();
                None
            }
            Verdict::Hang(_) => {
                tally.hang();
                None
            }
            Verdict::Violation(reason) => {
                let confirmed = self.confirm(j.p, j.seed, j.trial, j.timeout);
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
                    tally.with_properties(
                        RunOutcome::failed(j.index + 1, j.index + 1, ms, failure)
                            .with_inconclusive(tally.inconclusive)
                            .with_schedule_stats(distinct, work),
                    ),
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
            .with_param("hermit_args", p.hermit_args.join(" "))
            .with_param("hang_is_violation", p.hang_is_violation.to_string())
            .with_param("events_transport", self.events_transport())
            .with_param("output_hash", hash);
        if let Some(schedule) = &t.schedule {
            bundle = bundle.with_param("schedule_fingerprint", schedule.fingerprint.clone());
        }
        if let Some(key) = property_verdict::primary_key(t) {
            // What replay must see fail again; bundles without these (older ones, legacy violations) skip the check.
            bundle = bundle
                .with_param("property", key.id)
                .with_param("property_kind", key.kind.as_str());
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
fn replay_notes(
    bundle: &Bundle,
    trial: &Trial,
    verdict: &Verdict,
    hang_relaxed: bool,
) -> Option<String> {
    let yes_no = |same: bool| if same { "yes" } else { "no" };
    let mut notes = Vec::new();
    if hang_relaxed {
        notes.push(
            "timed out again; this target no longer treats a hang as a violation".to_string(),
        );
    }
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
    if let Some(same) = property_matches(bundle, trial) {
        notes.push(format!(
            "the recorded property failed again: {}",
            yes_no(same)
        ));
    }
    if let Verdict::Inconclusive(why) = verdict {
        notes.push(format!("the rerun was inconclusive: {why}"));
    }
    (!notes.is_empty()).then(|| notes.join("; "))
}

/// Whether the rerun failed the property the bundle recorded; `None` for bundles that recorded none (older ones, and
/// failures that were an exit status or a legacy violation line).
fn property_matches(bundle: &Bundle, trial: &Trial) -> Option<bool> {
    let id = bundle.params.get("property")?;
    let kind = bundle
        .params
        .get("property_kind")
        .and_then(|k| AssertKind::from_wire(k));
    let failing = property_verdict::failing_keys(trial);
    Some(
        failing
            .iter()
            .any(|k: &FailureKey| k.id == *id && kind.is_none_or(|expected| expected == k.kind)),
    )
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
    if recorded != current {
        return Err(EngineError::Backend(format!(
            "bundle was recorded with scheduler flags `{recorded}` but target uses `{current}`; replay needs the same scheduler"
        )));
    }
    // Extra Hermit arguments can change the schedule too (for example `--chaos-target-races`).
    let current_args = p.hermit_args.join(" ");
    let recorded_args = bundle
        .params
        .get("hermit_args")
        .cloned()
        .unwrap_or_default();
    if recorded_args != current_args {
        return Err(EngineError::Backend(format!(
            "bundle was recorded with hermit_args `{recorded_args}` but target uses `{current_args}`; replay needs the same hermit_args"
        )));
    }
    Ok(())
}

impl HermitBackend {
    /// Replaying through the other transport would change what the guest sees (its `/tmp`, its environment), so the
    /// schedule would not reproduce. Bundles from before the setting existed used the stream transport.
    fn check_same_events_transport(&self, bundle: &Bundle) -> Result<(), EngineError> {
        let recorded = bundle
            .params
            .get("events_transport")
            .map_or("stream", String::as_str);
        let current = self.events_transport();
        if recorded == current {
            return Ok(());
        }
        Err(EngineError::Backend(format!(
            "events transport differs: the bundle was recorded with the `{recorded}` transport but this backend uses `{current}`; replay needs the same `events_file` setting"
        )))
    }
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
        self.check_events_setup(&p)?;
        let started = Instant::now();
        let wall = Duration::from_secs(budget.wall_secs);
        let trials = budget.max_trials.max(1);
        let width = u64::from(p.parallelism.max(1));
        let mut tally = Tally::new(self.observer.clone());
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
            if tally.budget_cut {
                return Ok(tally.finish(started));
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
        self.check_events_setup(&p)?;
        self.check_same_events_transport(bundle)?;
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
        let trial = run_trial(&settings, &p, seed, timeout, self.dirs())
            .map_err(|e| EngineError::Backend(e.to_string()))?;
        let verdict = self.verdict(&p, &trial, timeout);
        if let Verdict::Infra(m) = &verdict {
            return Err(EngineError::Backend(m.clone()));
        }
        let hang_relaxed = matches!(verdict, Verdict::Hang(_))
            && bundle.params.get("hang_is_violation").map(String::as_str) == Some("true");
        let notes = replay_notes(bundle, &trial, &verdict, hang_relaxed);
        let matches = bundle
            .params
            .get("output_hash")
            .map(|h| *h == behaviour_hash(&trial));
        Ok(ReplayOutcome {
            reproduced: matches!(verdict, Verdict::Violation(_))
                && matches != Some(false)
                && property_matches(bundle, &trial) != Some(false),
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
                if let Verdict::Infra(m) = self.verdict(&p, t, timeout) {
                    return Err(EngineError::Backend(m));
                }
                if t.timed_out {
                    // Two kills at wall-clock deadlines capture different output; that is not nondeterminism.
                    return Err(EngineError::Backend(format!(
                        "a doctor trial timed out after {}s; raise trial_timeout_secs so both runs can finish",
                        timeout.as_secs()
                    )));
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
