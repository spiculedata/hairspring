use crate::exec::{run_trial, ProcessSpec, SpawnError, Trial};
use crate::trace::{behaviour_hash, classify, steps, Verdict};
use crate::HermitSettings;
use anthill_core::bundle::Bundle;
use anthill_engine::backend::{Backend, Progress, RunRequest};
use anthill_engine::config::{Budget, TargetKind, TargetSpec};
use anthill_engine::report::{
    DoctorDivergence, DoctorReport, FailureReport, ReplayOutcome, RunOutcome,
};
use anthill_engine::EngineError;
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

pub struct HermitBackend {
    settings: HermitSettings,
}

impl HermitBackend {
    pub fn new(settings: HermitSettings) -> HermitBackend {
        HermitBackend { settings }
    }

    /// Uses `ANTHILL_HERMIT` or `hermit` from the PATH.
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
            } => Ok(ProcessSpec {
                command,
                env,
                cwd: cwd.as_deref(),
                hermit_args,
                trial_timeout_secs: *trial_timeout_secs,
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

    fn trial(&self, p: &ProcessSpec, seed: u64, timeout: Duration) -> Result<Trial, SpawnError> {
        run_trial(&self.settings, p, seed, timeout)
    }

    /// Re-runs a violating seed and counts how many runs show the same behaviour.
    fn confirm(&self, p: &ProcessSpec, seed: u64, reference: &str, timeout: Duration) -> u32 {
        (0..self.settings.repro_runs)
            .filter(|_| {
                self.trial(p, seed, timeout).is_ok_and(|t| {
                    matches!(classify(&t, timeout.as_secs()), Verdict::Violation(_))
                        && behaviour_hash(&t) == reference
                })
            })
            .count() as u32
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
        let bundle = Bundle::new(seed, BUILD, vec![])
            .with_target(req.target.clone())
            .with_backend("hermit")
            .with_param("sched_seed", seed.to_string())
            .with_param("epoch", self.settings.epoch.clone())
            .with_param("heuristic", "random")
            .with_param("output_hash", hash);
        let steps = steps(p.command, seed, &self.settings.epoch, t, None);
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
        for i in 0..trials {
            let seed = req.seed.wrapping_add(i);
            let remaining = wall.saturating_sub(started.elapsed());
            if i > 0 && remaining.is_zero() {
                return Ok(RunOutcome::passed(
                    i,
                    i,
                    started.elapsed().as_millis() as u64,
                ));
            }
            let allowed = remaining.max(Duration::from_secs(1));
            // True when the wall budget, not the target's own limit, is what cut the trial short.
            let clipped = allowed < self.trial_timeout(&p);
            let timeout = self.trial_timeout(&p).min(allowed);
            let trial = match self.trial(&p, seed, timeout) {
                Ok(t) => t,
                Err(e) => return Ok(infra(e.to_string())),
            };
            match classify(&trial, timeout.as_secs()) {
                Verdict::Infra(msg) => return Ok(infra(msg)),
                Verdict::Violation(_) if trial.timed_out && clipped => {
                    // Out of budget, not out of patience: inconclusive, never a finding.
                    return Ok(RunOutcome::passed(
                        i,
                        i,
                        started.elapsed().as_millis() as u64,
                    ));
                }
                Verdict::Violation(reason) => {
                    let confirmed = self.confirm(&p, seed, &behaviour_hash(&trial), timeout);
                    let failure = self.failure(&Found {
                        spec,
                        p: &p,
                        req,
                        seed,
                        trial: &trial,
                        reason: &reason,
                        confirmed,
                    });
                    return Ok(RunOutcome::failed(
                        i + 1,
                        i + 1,
                        started.elapsed().as_millis() as u64,
                        failure,
                    ));
                }
                Verdict::Pass => {}
            }
            if !progress(Progress {
                steps: i + 1,
                trials: i + 1,
                message: format!("{} of {trials} seeds explored", i + 1),
            }) {
                return Ok(RunOutcome::error(CANCELLED));
            }
        }
        Ok(RunOutcome::passed(
            trials,
            trials,
            started.elapsed().as_millis() as u64,
        ))
    }

    fn replay(&self, spec: &TargetSpec, bundle: &Bundle) -> Result<ReplayOutcome, EngineError> {
        let p = self.process(spec)?;
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
        let verdict = classify(&trial, timeout.as_secs());
        if let Verdict::Infra(m) = &verdict {
            return Err(EngineError::Backend(m.clone()));
        }
        let matches = bundle
            .params
            .get("output_hash")
            .map(|h| *h == behaviour_hash(&trial));
        let note = match matches {
            Some(true) => Some("output matches the recording: yes".to_string()),
            Some(false) => Some(
                "output matches the recording: no (the program or environment changed)".to_string(),
            ),
            None => None,
        };
        Ok(ReplayOutcome {
            reproduced: matches!(verdict, Verdict::Violation(_)) && matches != Some(false),
            steps: steps(p.command, seed, &settings.epoch, &trial, note),
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
