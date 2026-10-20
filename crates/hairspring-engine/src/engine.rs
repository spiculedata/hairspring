use crate::backend::{Backend, Progress, ReplayOptions};
use crate::config::Config;
use crate::erased::Registry;
use crate::report::{DoctorReport, ReplayOutcome, RunOutcome};
use crate::sim_backend::SimBackend;
use crate::EngineError;
use hairspring_core::bundle::Bundle;
use std::sync::Arc;

pub use crate::backend::RunRequest;

#[derive(Clone)]
pub struct Engine {
    config: Arc<Config>,
    backends: Vec<Arc<dyn Backend>>,
}

impl Engine {
    pub fn new(registry: Arc<Registry>, config: Config) -> Engine {
        Engine {
            config: Arc::new(config),
            backends: vec![Arc::new(SimBackend::new(registry))],
        }
    }

    pub fn with_backend(self, backend: Arc<dyn Backend>) -> Engine {
        let mut backends = self.backends;
        backends.push(backend);
        Engine {
            config: self.config,
            backends,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn target_names(&self) -> Vec<String> {
        self.config.targets.keys().cloned().collect()
    }

    fn backend_for(
        &self,
        kind: &crate::config::TargetKind,
    ) -> Result<&Arc<dyn Backend>, EngineError> {
        self.backends
            .iter()
            .find(|b| b.supports(kind))
            .ok_or_else(|| EngineError::BackendUnavailable(kind.name().to_string()))
    }

    pub fn run(
        &self,
        req: &RunRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        let spec = self
            .config
            .targets
            .get(&req.target)
            .ok_or_else(|| EngineError::UnknownTarget(req.target.clone()))?;
        let backend = self.backend_for(&spec.kind)?;
        let budget = req.budget.unwrap_or(spec.budget).clamp();
        backend.run(spec, req, budget, progress)
    }

    pub fn replay(&self, bundle: &Bundle) -> Result<ReplayOutcome, EngineError> {
        self.replay_with(bundle, ReplayOptions::default())
    }

    pub fn replay_with(
        &self,
        bundle: &Bundle,
        options: ReplayOptions,
    ) -> Result<ReplayOutcome, EngineError> {
        let spec = self
            .config
            .targets
            .get(&bundle.target)
            .ok_or_else(|| EngineError::UnknownTarget(bundle.target.clone()))?;
        let backend = self.backend_for(&spec.kind)?;
        if backend.name() != bundle.backend {
            return Err(EngineError::InvalidBundle(format!(
                "bundle was produced by backend `{}` but target `{}` runs on `{}`",
                bundle.backend,
                bundle.target,
                backend.name()
            )));
        }
        backend.replay_with(spec, bundle, options)
    }

    /// Runs the determinism self-check for a target over `seeds` seeds.
    pub fn doctor(
        &self,
        target: &str,
        seeds: u32,
        steps: usize,
    ) -> Result<DoctorReport, EngineError> {
        let spec = self
            .config
            .targets
            .get(target)
            .ok_or_else(|| EngineError::UnknownTarget(target.to_string()))?;
        self.backend_for(&spec.kind)?
            .doctor(spec, seeds.min(1000), steps.min(100_000))
    }
}
