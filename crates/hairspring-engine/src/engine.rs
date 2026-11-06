use crate::backend::{Backend, Progress, ReplayOptions};
use crate::backend_info::BackendInfo;
use crate::config::{Config, TargetSpec};
use crate::confirm::{CheckResult, ConfirmRequest};
use crate::erased::Registry;
use crate::report::{DoctorReport, ReplayOutcome, RunOutcome};
use crate::sim_backend::SimBackend;
use crate::EngineError;
use hairspring_core::bundle::Bundle;
use hairspring_core::point::BackendKind;
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

    /// What the registered backends say about themselves, in registration order.
    pub fn backend_infos(&self) -> Vec<BackendInfo> {
        self.backends.iter().map(|b| b.info()).collect()
    }

    /// What the backend that runs `spec` says about itself, or `None` when this engine has no backend registered for it.
    pub fn backend_info(&self, spec: &TargetSpec) -> Option<BackendInfo> {
        self.backend_for(spec).ok().map(|b| b.info())
    }

    /// The backend the target names, by id. A target never runs on another backend because the one it names is
    /// missing: that is an error, not a fallback.
    fn backend_for(&self, spec: &TargetSpec) -> Result<&Arc<dyn Backend>, EngineError> {
        let id = spec.kind.backend_id();
        // Only an id this build knows is repeated in a message: the id of a hand-built spec is not trusted text.
        let known = BackendKind::from_id(id).is_some();
        let shown = if known {
            format!("backend `{id}`")
        } else {
            "the named backend".to_string()
        };
        let registered_as = if known {
            format!("`{id}`")
        } else {
            "the named id".to_string()
        };
        let mut named = self.backends.iter().filter(|b| b.id() == id);
        let backend = named.next().ok_or_else(|| {
            EngineError::BackendChoice(format!("{shown} is not available in this build"))
        })?;
        if named.next().is_some() {
            return Err(EngineError::BackendChoice(format!(
                "more than one backend is registered as {registered_as}"
            )));
        }
        if !backend.supports(&spec.kind) {
            return Err(EngineError::BackendChoice(format!(
                "{shown} cannot run {} targets",
                spec.kind.name()
            )));
        }
        Ok(backend)
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
        let backend = self.backend_for(spec)?;
        let budget = req.budget.unwrap_or(spec.budget).clamp();
        backend.run(spec, req, budget, progress)
    }

    /// What the backend of `target` records about the inputs of a run of it (see [`Backend::run_inputs`]), or `None`
    /// for an unknown target or a backend that records none.
    pub fn run_inputs(&self, target: &str) -> Option<hairspring_core::point::IdentityDraft> {
        let spec = self.config.targets.get(target)?;
        self.backend_for(spec).ok()?.run_inputs(spec)
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
        let backend = self.backend_for(spec)?;
        if backend.id() != bundle.backend {
            return Err(EngineError::InvalidBundle(format!(
                "bundle was produced by backend `{}` but target `{}` runs on `{}`",
                bundle.backend,
                bundle.target,
                backend.id()
            )));
        }
        backend.replay_with(spec, bundle, options)
    }

    /// Runs the confirmation pipeline on the failure `bundle` records. The request is checked and every target it
    /// names is resolved before anything runs. See [`Backend::confirm`] for `progress`.
    pub fn confirm(
        &self,
        bundle: &Bundle,
        req: &ConfirmRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<Vec<CheckResult>, EngineError> {
        let req = req.clone().validated()?;
        let spec = self
            .config
            .targets
            .get(&bundle.target)
            .ok_or_else(|| EngineError::UnknownTarget(bundle.target.clone()))?;
        let backend = self.backend_for(spec)?;
        if backend.id() != bundle.backend {
            return Err(EngineError::InvalidBundle(format!(
                "bundle was produced by backend `{}` but target `{}` runs on `{}`",
                bundle.backend,
                bundle.target,
                backend.id()
            )));
        }
        let control = req
            .control
            .as_ref()
            .map(|name| {
                self.config
                    .targets
                    .get(name)
                    .ok_or_else(|| EngineError::UnknownTarget(name.clone()))
            })
            .transpose()?;
        backend.confirm(spec, control, bundle, &req, progress)
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
        self.backend_for(spec)?
            .doctor(spec, seeds.min(1000), steps.min(100_000))
    }
}
