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
mod verify_report;
mod verify_staging;

use crate::backend::HermitBackend;
use crate::exec::ProcessSpec;
use crate::interrupt;
use crate::replay_environment;
use context::{seed_of, Ctx, EnvState};
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::Progress;
use hairspring_engine::config::TargetSpec;
use hairspring_engine::confirm::{forbidden_logging, CheckKind, CheckResult, ConfirmRequest};
use hairspring_engine::EngineError;
use pace::{Pace, Stop};
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

/// Every warning that says the environment differs, joined, so a concrete change is not hidden behind another line
/// that also says so (for example a field this build does not know).
fn environment_summary(warnings: &[String]) -> Option<String> {
    let lines: Vec<&str> = warnings
        .iter()
        .map(String::as_str)
        .filter(|w| w.contains(DIFFERS))
        .collect();
    (!lines.is_empty()).then(|| lines.join("; "))
}

const DIFFERS: &str = "environment differs";

/// How the host compares with the bundle's recorded environment.
fn environment_state(
    bundle: &Bundle,
    identity: &hairspring_core::environment::EnvironmentIdentity,
    allow_change: bool,
) -> Result<EnvState, EngineError> {
    match replay_environment::check(bundle, identity, allow_change) {
        Ok(check) => Ok(EnvState::Open {
            differs: check.differs,
            summary: environment_summary(&check.warnings),
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
///
/// An interrupt (Ctrl-C) kills the run in progress, and the server's stop can too (a service manager signals the Hermit
/// process with the server), so a check that was running when either came did not finish and says
/// nothing about the program: it is recorded as skipped for cancellation, whatever it made of the killed run, so that
/// the newest confirmation never lifts or withdraws what an earlier one found.
fn step(
    ctx: &Ctx,
    pace: &mut Pace,
    kind: CheckKind,
    check: impl FnOnce(&mut Pace) -> CheckResult,
) -> CheckResult {
    if interrupt::requested() {
        return ctx.skipped(kind, Stop::Cancelled.text());
    }
    match pace.stop() {
        Some(stop) => ctx.skipped(kind, stop.text()),
        None => {
            let result = check(pace);
            // The same for a check whose run the server's own stop cut short (see `Pace::mark_cut`): whatever it made
            // of the killed run, the check did not finish.
            if interrupt::requested() || pace.was_cut() {
                return ctx.skipped(kind, Stop::Cancelled.text());
            }
            result
        }
    }
}

#[cfg(test)]
mod summary_tests {
    use super::environment_summary;

    #[test]
    fn every_line_that_says_the_environment_differs_is_kept_in_order() {
        let warnings = vec![
            "malformed recorded environment fields, not compared: x".to_string(),
            "environment differs: host.kernel_release (recorded a, now b)".to_string(),
            "environment differs: this bundle records environment fields this build does not know (h.x)".to_string(),
        ];
        let summary = environment_summary(&warnings).unwrap();
        assert!(
            summary.starts_with("environment differs: host.kernel_release"),
            "{summary}"
        );
        assert!(
            summary.contains("; environment differs: this bundle records"),
            "{summary}"
        );
        assert!(!summary.contains("malformed"), "{summary}");
    }

    #[test]
    fn no_such_line_gives_no_summary() {
        assert_eq!(environment_summary(&["something else".to_string()]), None);
        assert_eq!(environment_summary(&[]), None);
    }
}
