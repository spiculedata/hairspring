//! Check 3: the same seed on the fixed build.
//!
//! The control must differ from the failing target in its program and nothing else that shapes a run: the scheduler
//! flags, the extra Hermit arguments and the guest environment are the same, or it is not a control and nothing is run.
//! It passes if the fixed build is clean at the seed. That is all it says: "the fixed build does not fail at this
//! seed", not "the same interleaving", because a different build is a different schedule. A control that still fails is
//! flagged. One that could not be judged (a timeout, a Hermit fault) is inconclusive and never read as clean.
//!
//! What is not compared: the working directory, the `--tmp` pre-seed and the events transport. The control target
//! is trusted to be the same program in the same setting, with the fix as the only change.
use super::attempt;
use super::context::{now_ms, Ctx};
use super::{command, pace::Pace};
use crate::backend::HermitBackend;
use crate::exec::scheduler_flags;
use crate::exec::ProcessSpec;
use crate::trace::{behaviour_hash, Verdict};
use hairspring_engine::config::TargetSpec;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus};

const NOTE_CLEAN: &str = "the fixed build does not fail at this seed; a different build is a different schedule, so this says nothing about the original's schedule";
const FLAG_STILL_FAILS: &str = "control failed: the fixed build still fails at this seed";

/// Why `control` is not a control for the target, if it is not.
fn mismatch(ctx: &Ctx, control: &ProcessSpec) -> Option<&'static str> {
    if scheduler_flags(control.scheduler) != scheduler_flags(ctx.p.scheduler) {
        return Some(
            "the control's scheduler flags differ from the target's, so it is not a control",
        );
    }
    if control.hermit_args != ctx.p.hermit_args {
        return Some("the control's Hermit flags differ from the target's, so it is not a control");
    }
    if control.env != ctx.p.env {
        return Some(
            "the control's guest environment differs from the target's, so it is not a control",
        );
    }
    None
}

pub(super) fn check(
    ctx: &Ctx,
    spec: &TargetSpec,
    control: &ProcessSpec,
    pace: &mut Pace,
) -> CheckResult {
    let started = now_ms();
    let kind = CheckKind::FixedControl;
    let named = |result: CheckResult| result.with_detail("control_target", spec.name.clone());
    let command = command::line(&ctx.backend.settings, control, ctx.seed, &[]);
    if let Some(why) = mismatch(ctx, control) {
        return named(
            ctx.result(kind, CheckStatus::Inconclusive, started)
                .with_command(command)
                .with_detail("inconclusive", why),
        );
    }
    let environment = ctx.backend.environment(control);
    let own_limit = ctx.backend.trial_timeout(control);
    let a = attempt::run(ctx.backend, control, ctx.seed, own_limit, pace);
    let clipped = a.clipped_by_budget(own_limit);
    pace.ran("control".to_string());
    let result = ctx
        .result(kind, CheckStatus::Inconclusive, started)
        .with_environment_fields(
            environment
                .fields()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        )
        .with_command(command);
    let result = named(result);
    let result = if a.retries > 0 {
        result.with_detail("skid_retries", a.retries.to_string())
    } else {
        result
    };
    let result = if a.doubled {
        result
            .with_detail("skid_margin_doubled", "true")
            .with_detail(
                "caveat",
                "this run used double the skid margin, a different schedule; it counts for the control only with this caveat",
            )
    } else {
        result
    };
    if clipped {
        return result.with_counts(0, 0, 1).with_detail(
            "inconclusive",
            "the wall budget ended the control before it finished, so it says nothing",
        );
    }
    judge(ctx.backend, control, a.timeout, a.trial, result)
}

fn judge(
    backend: &HermitBackend,
    control: &ProcessSpec,
    timeout: std::time::Duration,
    trial: Result<crate::exec::Trial, crate::exec::SpawnError>,
    result: CheckResult,
) -> CheckResult {
    let Ok(t) = trial else {
        return result
            .with_counts(0, 0, 1)
            .with_detail("inconclusive", "Hermit could not be started (host fault)");
    };
    let result = result.with_artefact(behaviour_hash(&t));
    let result = match &t.schedule {
        Some(s) => result.with_artefact(s.fingerprint.clone()),
        None => result,
    };
    match backend.verdict(control, &t, timeout) {
        Verdict::Pass => result
            .with_status(CheckStatus::Pass)
            .with_counts(1, 0, 0)
            .with_detail("note", NOTE_CLEAN),
        Verdict::Violation(_) | Verdict::Deadlock(_) => result
            .with_status(CheckStatus::Fail)
            .with_counts(0, 1, 0)
            .with_detail("flag", FLAG_STILL_FAILS),
        Verdict::Infra(_) => result.with_counts(0, 0, 1).with_detail(
            "inconclusive",
            "Hermit failed (infrastructure, not the program), so the control says nothing",
        ),
        Verdict::Hang(_) | Verdict::Inconclusive(_) => result.with_counts(0, 0, 1).with_detail(
            "inconclusive",
            "the control could not be judged (timeout or events fault), so it says nothing",
        ),
    }
}
