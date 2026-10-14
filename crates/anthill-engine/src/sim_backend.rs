//! Tree-exploration backend for registry `Sim` targets.
use crate::backend::{Backend, Progress, RunRequest};
use crate::config::{Budget, TargetKind, TargetSpec};
use crate::erased::{Erased, ErasedAction, Registry};
use crate::report::{
    DoctorDivergence, DoctorReport, FailureReport, ReplayOutcome, RunOutcome, Step,
};
use crate::EngineError;
use anthill_core::bundle::Bundle;
use anthill_core::doctor::doctor;
use anthill_core::explore::{branching, Config as ExploreConfig};
use anthill_core::replay::find_action;
use anthill_core::shrink::shrink;
use anthill_core::{ActionId, Sim};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ROUND_STEPS: u64 = 100_000;
const BUILD: &str = env!("CARGO_PKG_VERSION");

pub struct SimBackend {
    registry: Arc<Registry>,
}

impl SimBackend {
    pub fn new(registry: Arc<Registry>) -> Self {
        SimBackend { registry }
    }

    fn root(&self, spec: &TargetSpec) -> Result<Erased, EngineError> {
        let TargetKind::Sim { sim } = &spec.kind else {
            return Err(EngineError::BackendUnavailable(
                spec.kind.name().to_string(),
            ));
        };
        self.registry
            .instantiate(sim)
            .ok_or_else(|| EngineError::UnknownTarget(sim.clone()))
    }
}

/// Replays `path` from `root`, recording a labelled step with the state after each action.
pub fn trace(root: &Erased, path: &[ActionId]) -> Result<Vec<Step>, EngineError> {
    let mut state = root.clone();
    let mut steps = Vec::with_capacity(path.len());
    for (index, &id) in path.iter().enumerate() {
        if state.violated() {
            break;
        }
        let label = state.label_of(id).ok_or_else(|| {
            EngineError::Replay(format!("action {id:016x} is not enabled at step {index}"))
        })?;
        state = state.apply(ErasedAction(id));
        steps.push(Step {
            index,
            action_id: format!("{id:016x}"),
            label,
            state: state.state_debug(),
        });
    }
    Ok(steps)
}

impl Backend for SimBackend {
    fn name(&self) -> &'static str {
        "sim"
    }

    fn supports(&self, kind: &TargetKind) -> bool {
        matches!(kind, TargetKind::Sim { .. })
    }

    fn run(
        &self,
        spec: &TargetSpec,
        req: &RunRequest,
        budget: Budget,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<RunOutcome, EngineError> {
        let root = self.root(spec)?;
        let started = Instant::now();
        let wall = Duration::from_secs(budget.wall_secs);
        let mut steps_total = 0u64;
        let mut round = 0u64;
        loop {
            let seed = req.seed.wrapping_add(round);
            let max_steps = ROUND_STEPS
                .min(budget.max_steps.saturating_sub(steps_total))
                .max(1);
            let cfg = ExploreConfig {
                seed,
                max_steps,
                ..ExploreConfig::default()
            };
            round += 1;
            if let Some(found) = branching(&root, &cfg) {
                steps_total += found.steps_executed;
                let shrunk = shrink(&root, &found.path);
                let steps = trace(&root, &shrunk)?;
                let bundle = Bundle::new(seed, BUILD, shrunk.clone())
                    .with_target(req.target.clone())
                    .with_backend("sim");
                let failure = FailureReport {
                    target: req.target.clone(),
                    backend: "sim".to_string(),
                    message: format!(
                        "invariant violated; shrunk from {} to {} steps",
                        found.path.len(),
                        shrunk.len()
                    ),
                    original_len: found.path.len(),
                    shrunk_len: shrunk.len(),
                    steps,
                    bundle: bundle.encode(),
                };
                progress(Progress {
                    steps: steps_total,
                    trials: round,
                    message: "violation found".to_string(),
                });
                return Ok(RunOutcome::failed(
                    steps_total,
                    round,
                    started.elapsed().as_millis() as u64,
                    failure,
                ));
            }
            steps_total += max_steps;
            progress(Progress {
                steps: steps_total,
                trials: round,
                message: format!("explored {steps_total} steps"),
            });
            if steps_total >= budget.max_steps
                || started.elapsed() >= wall
                || round >= budget.max_trials.max(1)
            {
                return Ok(RunOutcome::passed(
                    steps_total,
                    round,
                    started.elapsed().as_millis() as u64,
                ));
            }
        }
    }

    fn replay(&self, spec: &TargetSpec, bundle: &Bundle) -> Result<ReplayOutcome, EngineError> {
        let root = self.root(spec)?;
        let steps = trace(&root, &bundle.path)?;
        let mut state = root;
        for id in &bundle.path {
            if state.violated() {
                break;
            }
            let action = find_action(&state, *id)
                .ok_or_else(|| EngineError::Replay("action not enabled".to_string()))?;
            state = state.apply(action);
        }
        Ok(ReplayOutcome {
            reproduced: state.violated(),
            steps,
        })
    }

    fn doctor(
        &self,
        spec: &TargetSpec,
        seeds: u32,
        steps: usize,
    ) -> Result<DoctorReport, EngineError> {
        let root = self.root(spec)?;
        for i in 0..seeds.max(1) {
            let seed = u64::from(i) + 1;
            if let Err(d) = doctor(&root, seed, steps) {
                return Ok(DoctorReport {
                    ok: false,
                    seeds_checked: i + 1,
                    steps_per_seed: steps,
                    divergence: Some(DoctorDivergence { seed, step: d.step }),
                });
            }
        }
        Ok(DoctorReport {
            ok: true,
            seeds_checked: seeds.max(1),
            steps_per_seed: steps,
            divergence: None,
        })
    }
}
