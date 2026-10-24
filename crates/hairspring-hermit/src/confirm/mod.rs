//! The confirmation pipeline on the Hermit backend.
//!
//! The checks run in a fixed order, each one recorded: same-seed reruns, the bundle replay, the fixed-version control
//! and the strict verify (the last two only when asked for). Before anything runs, the request is checked against what a
//! confirmation may carry: no Hermit logging, since logging changed a guest's path. A check that could not run because
//! the caller cancelled, or the wall budget was used up, is recorded as skipped and says why.
mod attempt;
mod command;
mod context;
mod control;
mod pace;
mod replay;
mod rerun;
mod verify;

use crate::backend::HermitBackend;
use crate::exec::ProcessSpec;
use crate::replay_environment;
use context::{seed_of, Ctx, EnvState};
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::Progress;
use hairspring_engine::config::TargetSpec;
use hairspring_engine::confirm::{forbidden_logging, CheckKind, CheckResult, ConfirmRequest};
use hairspring_engine::EngineError;
use pace::Pace;
use std::collections::BTreeMap;
use std::time::Duration;

/// Refuses a target whose Hermit arguments or environment turn on logging. Every environment Hermit's process is given
/// counts: the backend's own `extra_env` and the target's `env`, which a run passes on to the guest through Hermit.
fn refuse_logging(backend: &HermitBackend, p: &ProcessSpec) -> Result<(), EngineError> {
    let environment: BTreeMap<String, String> = backend
        .settings
        .extra_env
        .iter()
        .cloned()
        .chain(p.env.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
    match forbidden_logging(p.hermit_args, &environment) {
        Some(why) => Err(EngineError::InvalidRequest(why)),
        None => Ok(()),
    }
}

/// How the host compares with the bundle's recorded environment.
fn environment_state(
    bundle: &Bundle,
    identity: &hairspring_core::environment::EnvironmentIdentity,
    allow_change: bool,
) -> Result<EnvState, EngineError> {
    match replay_environment::check(bundle, identity, allow_change) {
        Ok(check) => Ok(EnvState::Open {
            differs: check.differs,
            summary: check
                .warnings
                .into_iter()
                .find(|w| w.contains("environment differs")),
        }),
        Err(EngineError::EnvironmentMismatch(why)) => Ok(EnvState::Refused(why)),
        Err(other) => Err(other),
    }
}

pub(crate) fn run(
    backend: &HermitBackend,
    spec: &TargetSpec,
    control: Option<&TargetSpec>,
    bundle: &Bundle,
    req: &ConfirmRequest,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<Vec<CheckResult>, EngineError> {
    let p = backend.process(spec)?;
    refuse_logging(backend, &p)?;
    let control_process = control
        .map(|c| {
            backend
                .process(c)
                .and_then(|cp| refuse_logging(backend, &cp).map(|()| cp))
        })
        .transpose()?;
    backend.check_events_setup(&p)?;
    crate::backend::check_same_scheduler(&p, bundle)?;
    backend.check_same_events_transport(bundle)?;
    let identity = backend.environment(&p);
    let ctx = Ctx {
        backend,
        spec,
        p: &p,
        bundle,
        req,
        seed: seed_of(bundle),
        environment: identity
            .fields()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        env: environment_state(bundle, &identity, req.allow_environment_change)?,
    };
    let mut pace = Pace::new(Duration::from_secs(req.wall_secs), progress);
    let mut checks = Vec::new();
    checks.push(step(&ctx, &mut pace, CheckKind::SameSeedRerun, |pace| {
        rerun::check(&ctx, pace)
    }));
    checks.push(step(&ctx, &mut pace, CheckKind::BundleReplay, |pace| {
        replay::check(&ctx, pace)
    }));
    if let (Some(spec), Some(cp)) = (control, &control_process) {
        checks.push(step(&ctx, &mut pace, CheckKind::FixedControl, |pace| {
            control::check(&ctx, spec, cp, pace)
        }));
    }
    if req.strict_verify {
        checks.push(step(&ctx, &mut pace, CheckKind::StrictVerify, |pace| {
            verify::check(&ctx, pace)
        }));
    }
    Ok(checks.into_iter().map(CheckResult::bounded).collect())
}

/// Runs one check, or records it as skipped when the confirmation was cancelled or ran out of time.
fn step(
    ctx: &Ctx,
    pace: &mut Pace,
    kind: CheckKind,
    check: impl FnOnce(&mut Pace) -> CheckResult,
) -> CheckResult {
    match pace.stop() {
        Some(stop) => ctx.skipped(kind, stop.text()),
        None => check(pace),
    }
}
