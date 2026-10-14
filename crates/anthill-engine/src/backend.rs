use crate::config::{Budget, TargetKind, TargetSpec};
use crate::report::{DoctorReport, ReplayOutcome, RunOutcome};
use crate::EngineError;
use anthill_core::bundle::Bundle;

#[derive(Debug, Clone)]
pub struct Progress {
    pub steps: u64,
    pub trials: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub target: String,
    pub seed: u64,
    pub budget: Option<Budget>,
}

impl RunRequest {
    pub fn new(target: impl Into<String>, seed: u64) -> Self {
        RunRequest {
            target: target.into(),
            seed,
            budget: None,
        }
    }
    pub fn with_budget(self, budget: Budget) -> Self {
        RunRequest {
            budget: Some(budget),
            ..self
        }
    }
}

/// An execution backend. `progress` is called periodically; returning `false` asks the backend to stop
/// (cancellation or shutdown) and it returns an error outcome with the message `cancelled`.
///
/// A backend: knows how to run and replay one or more target kinds.
pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, kind: &TargetKind) -> bool;
    fn run(
        &self,
        spec: &TargetSpec,
        req: &RunRequest,
        budget: Budget,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError>;
    fn replay(&self, spec: &TargetSpec, bundle: &Bundle) -> Result<ReplayOutcome, EngineError>;
    /// Determinism self-check. Backends that cannot do this in-process say so.
    fn doctor(
        &self,
        _spec: &TargetSpec,
        _seeds: u32,
        _steps: usize,
    ) -> Result<DoctorReport, EngineError> {
        Err(EngineError::BackendUnavailable(format!(
            "`doctor` is not supported by the {} backend",
            self.name()
        )))
    }
}
