use crate::config::{Budget, TargetKind, TargetSpec};
use crate::confirm::{CheckResult, ConfirmRequest};
use crate::mode::RunMode;
use crate::report::{DoctorReport, ReplayOutcome, RunOutcome};
use crate::EngineError;
use hairspring_core::bundle::Bundle;

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
    /// Whether the run stops at its first confirmed violation (the default) or runs its whole trial budget.
    pub mode: RunMode,
}

impl RunRequest {
    pub fn new(target: impl Into<String>, seed: u64) -> Self {
        RunRequest {
            target: target.into(),
            seed,
            budget: None,
            mode: RunMode::Search,
        }
    }
    pub fn with_budget(self, budget: Budget) -> Self {
        RunRequest {
            budget: Some(budget),
            ..self
        }
    }
    pub fn with_mode(self, mode: RunMode) -> Self {
        RunRequest { mode, ..self }
    }
}

/// Choices a caller makes about one replay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplayOptions {
    /// Replay even when the recorded environment differs in a way that makes the schedule unreliable (for the Hermit
    /// backend, a changed loader). Differences are still reported as warnings.
    pub allow_environment_change: bool,
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
    /// Replay with the caller's options. Backends that have no environment to check ignore them.
    fn replay_with(
        &self,
        spec: &TargetSpec,
        bundle: &Bundle,
        _options: ReplayOptions,
    ) -> Result<ReplayOutcome, EngineError> {
        self.replay(spec, bundle)
    }
    /// Runs the confirmation checks of `req` on the failure `bundle` records, returning each one that was run or skipped,
    /// in order. `control` is the target that runs the fixed build, when the request names one. `progress` is called
    /// between checks and reruns; returning `false` stops the confirmation, and the checks not yet run are returned as
    /// skipped. Backends that have nothing to rerun say so.
    fn confirm(
        &self,
        _spec: &TargetSpec,
        _control: Option<&TargetSpec>,
        _bundle: &Bundle,
        _req: &ConfirmRequest,
        _progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<Vec<CheckResult>, EngineError> {
        Err(EngineError::BackendUnavailable(format!(
            "`confirm` is not supported by the {} backend",
            self.name()
        )))
    }
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
