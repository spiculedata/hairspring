//! What every check of one confirmation shares: the target, the bundle, and what the bundle's environment said.
use crate::backend::HermitBackend;
use crate::exec::ProcessSpec;
use hairspring_core::bundle::Bundle;
use hairspring_engine::config::TargetSpec;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus, ConfirmRequest};
use std::collections::BTreeMap;

/// The result of comparing the host with the environment the bundle recorded, made once for the whole confirmation.
pub(super) enum EnvState {
    /// The dynamic loader changed and the caller did not allow going on: no run was made.
    Refused(String),
    Open {
        differs: bool,
        /// The first line of the comparison, naming the changed fields.
        summary: Option<String>,
    },
}

impl EnvState {
    pub(super) fn differs(&self) -> bool {
        match self {
            EnvState::Refused(_) => true,
            EnvState::Open { differs, .. } => *differs,
        }
    }
}

pub(super) struct Ctx<'a> {
    pub backend: &'a HermitBackend,
    pub spec: &'a TargetSpec,
    pub p: &'a ProcessSpec<'a>,
    pub bundle: &'a Bundle,
    pub req: &'a ConfirmRequest,
    pub seed: u64,
    /// The host's environment identity as it is now, which every check of this confirmation ran in.
    pub environment: BTreeMap<String, String>,
    pub env: EnvState,
}

impl Ctx<'_> {
    /// A result of `kind` that carries the environment and the times it took. `started_ms` is when the check began.
    pub(super) fn result(
        &self,
        kind: CheckKind,
        status: CheckStatus,
        started_ms: i64,
    ) -> CheckResult {
        CheckResult::new(kind, status)
            .with_environment_fields(self.environment.clone())
            .with_times(started_ms, now_ms())
    }

    /// A check that was not run, and why.
    pub(super) fn skipped(&self, kind: CheckKind, why: &str) -> CheckResult {
        let now = now_ms();
        self.result(kind, CheckStatus::Skipped, now)
            .with_detail("skipped", why)
    }

    /// The `inconclusive` note for a check that could not tell, because the host differs from the recording.
    pub(super) fn environment_note(&self) -> String {
        match &self.env {
            EnvState::Refused(why) => format!(
                "environment differs: the dynamic loader changed since the recording, so no run was made ({why})"
            ),
            EnvState::Open { summary, .. } => format!(
                "environment differs: {}",
                summary.as_deref().unwrap_or("the host differs from the recording")
            ),
        }
    }
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The scheduling seed a bundle records, as a replay reads it.
pub(super) fn seed_of(bundle: &Bundle) -> u64 {
    bundle
        .params
        .get("sched_seed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(bundle.seed)
}
