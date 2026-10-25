//! Check 2: the bundle replayed, with its environment compared against the host.
//!
//! A replay that does not reproduce the failure on a host that differs from the recording says nothing about the
//! program, so it is recorded as inconclusive and the note names the environment. Only a miss on an unchanged host is a
//! failure of the check.
use super::command;
use super::context::{now_ms, Ctx, EnvState};
use super::pace::Pace;
use crate::backend::REPLAY_CLIPPED;
use hairspring_engine::backend::ReplayOptions;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus, ENVIRONMENT_CHANGED_KEY};
use hairspring_engine::EngineError;

pub(super) fn check(ctx: &Ctx, pace: &mut Pace) -> CheckResult {
    let started = now_ms();
    let kind = CheckKind::BundleReplay;
    let mut settings = ctx.backend.settings.clone();
    if let Some(epoch) = ctx.bundle.params.get("epoch") {
        settings.epoch = epoch.clone();
    }
    let command = command::line(&settings, ctx.p, ctx.seed, &[]);
    if let EnvState::Refused(_) = ctx.env {
        return ctx
            .result(kind, CheckStatus::Inconclusive, started)
            .with_command(command)
            .with_detail("inconclusive", ctx.environment_note())
            .with_detail(ENVIRONMENT_CHANGED_KEY, "true");
    }
    let options = ReplayOptions {
        allow_environment_change: ctx.req.allow_environment_change,
    };
    // The replay is given what is left of the budget, and is a run the caller may cancel after like any other.
    let cap = pace.run_timeout(ctx.backend.trial_timeout(ctx.p));
    let outcome = ctx
        .backend
        .replay_capped(ctx.spec, ctx.bundle, options, Some(cap));
    pace.ran("replay".to_string());
    let result = |status| {
        ctx.result(kind, status, started)
            .with_command(command.clone())
    };
    match outcome {
        Ok(replay) => {
            let changed = replay.environment_changed;
            let base = result(if replay.reproduced {
                CheckStatus::Pass
            } else if changed {
                CheckStatus::Inconclusive
            } else {
                CheckStatus::Fail
            })
            .with_counts(
                u32::from(replay.reproduced),
                u32::from(!replay.reproduced && !changed),
                u32::from(!replay.reproduced && changed),
            )
            .with_detail(ENVIRONMENT_CHANGED_KEY, changed.to_string());
            let base = match replay.warnings.first() {
                Some(warning) if changed => {
                    base.with_detail("environment_differs", warning.clone())
                }
                _ => base,
            };
            if !replay.reproduced && changed {
                base.with_detail("inconclusive", ctx.environment_note())
            } else if !replay.reproduced {
                base.with_detail(
                    "mismatch",
                    "the bundle no longer fails on an unchanged environment",
                )
            } else {
                base
            }
        }
        Err(EngineError::Backend(why)) if why == REPLAY_CLIPPED => {
            result(CheckStatus::Inconclusive)
                .with_counts(0, 0, 1)
                .with_detail(
                    "inconclusive",
                    "the wall budget ended the replay before it could finish, so it says nothing",
                )
                .with_detail("stopped", "wall budget exhausted")
        }
        Err(EngineError::EnvironmentMismatch(_)) => result(CheckStatus::Inconclusive)
            .with_detail("inconclusive", ctx.environment_note())
            .with_detail(ENVIRONMENT_CHANGED_KEY, "true"),
        Err(_) => result(CheckStatus::Inconclusive)
            .with_counts(0, 0, 1)
            .with_detail(
                "inconclusive",
                "the replay could not run (host or Hermit fault)",
            ),
    }
}
