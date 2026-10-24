//! Check 1: the failing seed, run again `reruns` times.
//!
//! A rerun passes only if it fails the same way the recording did: the same kind of failure, the same property, the same
//! behaviour hash and the same schedule fingerprint. Equal output on a different schedule is not a reproduction.
//! Output of a recording that has no hash, or a schedule with no fingerprint, is compared with the first rerun instead,
//! and the detail says so. A rerun that cannot be judged is counted apart, and never as a match or a miss.
use super::attempt::{self, Attempt};
use super::context::{now_ms, Ctx, EnvState};
use super::{command, pace::Pace};
use crate::backend::{failure_kind_matches, output_matches, property_matches};
use crate::exec::Trial;
use crate::trace::{behaviour_hash, Verdict};
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus};

const NO_FINGERPRINT: &str =
    "no schedule summary for this rerun, so its schedule could not be compared";

/// What one rerun showed.
enum Rerun {
    Match,
    Mismatch(&'static str),
    Inconclusive(&'static str),
}

/// What the first rerun showed, for the comparisons a recording cannot supply.
struct First {
    hash: String,
    fingerprint: Option<String>,
}

fn skid_text(a: &Attempt) -> Option<&'static str> {
    a.doubled.then_some(
        "a rerun at a higher skid margin is a different schedule, so it does not count toward the reruns",
    )
}

/// Whether the rerun finished and was judged (it failed, or it passed): a hung or infrastructure-failed run has nothing
/// to be compared with, so it is never the first rerun the others are measured against.
fn judged_failure_or_pass(ctx: &Ctx, trial: &Trial) -> bool {
    matches!(
        ctx.backend
            .verdict(ctx.p, trial, ctx.backend.trial_timeout(ctx.p)),
        Verdict::Pass | Verdict::Violation(_) | Verdict::Deadlock(_)
    )
}

/// What the reruns were compared with where the recording has nothing to compare, or `None` when it had it all.
fn baseline_note(ctx: &Ctx) -> Option<&'static str> {
    let params = &ctx.bundle.params;
    let no_output = !params.contains_key("output_hash");
    let no_schedule =
        !params.contains_key("schedule_fingerprint") && ctx.backend.dirs().summaries.is_some();
    match (no_output, no_schedule) {
        (false, false) => None,
        (true, false) => Some("the recording has no output hash, so each rerun's output was compared with the first rerun's"),
        (false, true) => Some("the recording has no schedule fingerprint, so each rerun's schedule was compared with the first rerun's"),
        (true, true) => Some("the recording has no output hash or schedule fingerprint, so each rerun was compared with the first rerun"),
    }
}

/// Compares a finished rerun with the recording (or with the first rerun where the recording has nothing to compare).
fn judge(ctx: &Ctx, trial: &Trial, first: &Option<First>) -> Rerun {
    let verdict = ctx
        .backend
        .verdict(ctx.p, trial, ctx.backend.trial_timeout(ctx.p));
    match &verdict {
        Verdict::Pass => return Rerun::Mismatch("the rerun did not fail"),
        Verdict::Infra(_) => {
            return Rerun::Inconclusive("Hermit failed (infrastructure, not the program)")
        }
        Verdict::Hang(_) | Verdict::Inconclusive(_) => {
            return Rerun::Inconclusive("the rerun could not be judged (timeout or events fault)")
        }
        Verdict::Violation(_) | Verdict::Deadlock(_) => {}
    }
    if failure_kind_matches(ctx.bundle, &verdict) == Some(false) {
        return Rerun::Mismatch("a different kind of failure");
    }
    if property_matches(ctx.bundle, trial) == Some(false) {
        return Rerun::Mismatch("a different property failed");
    }
    let hash = behaviour_hash(trial);
    let same_output =
        output_matches(ctx.bundle, trial).or_else(|| first.as_ref().map(|f| f.hash == hash));
    if same_output == Some(false) {
        return Rerun::Mismatch("output hash differs");
    }
    match (
        ctx.bundle.params.get("schedule_fingerprint"),
        &trial.schedule,
    ) {
        (Some(recorded), Some(now)) if *recorded != now.fingerprint => {
            Rerun::Mismatch("schedule fingerprint differs")
        }
        (Some(_), None) => Rerun::Inconclusive(NO_FINGERPRINT),
        (None, Some(now)) => match first.as_ref().and_then(|f| f.fingerprint.as_ref()) {
            Some(earlier) if *earlier != now.fingerprint => {
                Rerun::Mismatch("schedule fingerprint differs")
            }
            _ => Rerun::Match,
        },
        _ => Rerun::Match,
    }
}

#[derive(Default)]
struct Tally {
    matched: u32,
    mismatched: u32,
    inconclusive: u32,
    retries: u32,
    doubled: bool,
    first_mismatch: Option<&'static str>,
    first_inconclusive: Option<&'static str>,
    artefacts: Vec<String>,
}

impl Tally {
    fn add_artefact(&mut self, value: String) {
        if !self.artefacts.contains(&value) {
            self.artefacts.push(value);
        }
    }

    fn record(&mut self, outcome: &Rerun, a: &Attempt) {
        self.retries += a.retries;
        self.doubled |= a.doubled;
        match outcome {
            Rerun::Match => self.matched += 1,
            Rerun::Mismatch(why) => {
                self.mismatched += 1;
                self.first_mismatch.get_or_insert(why);
            }
            Rerun::Inconclusive(why) => {
                self.inconclusive += 1;
                self.first_inconclusive.get_or_insert(why);
            }
        }
    }
}

pub(super) fn check(ctx: &Ctx, pace: &mut Pace) -> CheckResult {
    let started = now_ms();
    let kind = CheckKind::SameSeedRerun;
    let command = command::line(&ctx.backend.settings, ctx.p, ctx.seed, &[]);
    if let EnvState::Refused(_) = ctx.env {
        return ctx
            .result(kind, CheckStatus::Inconclusive, started)
            .with_command(command)
            .with_detail("inconclusive", ctx.environment_note())
            .with_detail("environment_changed", "true");
    }
    let mut tally = Tally::default();
    let mut first: Option<First> = None;
    let mut stopped = None;
    let own_limit = ctx.backend.trial_timeout(ctx.p);
    for i in 0..ctx.req.reruns {
        stopped = pace.stop();
        if stopped.is_some() {
            break;
        }
        let a = attempt::run(ctx.backend, ctx.p, ctx.seed, own_limit, pace);
        let outcome = match (&a.trial, skid_text(&a)) {
            (Err(_), _) => Rerun::Inconclusive("Hermit could not be started (host fault)"),
            (Ok(_), _) if a.clipped_by_budget(own_limit) => {
                Rerun::Inconclusive("the wall budget ended the rerun before it finished")
            }
            (Ok(_), Some(why)) => Rerun::Inconclusive(why),
            (Ok(t), None) => judge(ctx, t, &first),
        };
        if let Ok(t) = &a.trial {
            tally.add_artefact(behaviour_hash(t));
            if let Some(s) = &t.schedule {
                tally.add_artefact(s.fingerprint.clone());
            }
            if first.is_none()
                && !a.doubled
                && !a.clipped_by_budget(own_limit)
                && judged_failure_or_pass(ctx, t)
            {
                first = Some(First {
                    hash: behaviour_hash(t),
                    fingerprint: t.schedule.as_ref().map(|s| s.fingerprint.clone()),
                });
            }
        }
        tally.record(&outcome, &a);
        pace.ran(format!("rerun {}/{}", i + 1, ctx.req.reruns));
    }
    finish(ctx, &tally, stopped.map(|s| s.text()), started, command)
}

fn finish(
    ctx: &Ctx,
    tally: &Tally,
    stopped: Option<&'static str>,
    started: i64,
    command: String,
) -> CheckResult {
    let done = tally.matched + tally.mismatched + tally.inconclusive;
    let environment_differs = ctx.env.differs();
    let status = if tally.mismatched > 0 && !environment_differs {
        CheckStatus::Fail
    } else if tally.mismatched > 0
        || tally.inconclusive > 0
        || stopped.is_some()
        || done < ctx.req.reruns
    {
        CheckStatus::Inconclusive
    } else {
        CheckStatus::Pass
    };
    let mut result = ctx
        .result(CheckKind::SameSeedRerun, status, started)
        .with_counts(tally.matched, tally.mismatched, tally.inconclusive)
        .with_command(command)
        .with_detail("environment_changed", environment_differs.to_string())
        .with_detail("reruns_asked", ctx.req.reruns.to_string());
    if let Some(recorded) = ctx.bundle.params.get("output_hash") {
        result = result.with_artefact(recorded.clone());
    }
    if let Some(recorded) = ctx.bundle.params.get("schedule_fingerprint") {
        result = result.with_artefact(recorded.clone());
    }
    for artefact in &tally.artefacts {
        result = result.with_artefact(artefact.clone());
    }
    if let Some(note) = baseline_note(ctx) {
        result = result.with_detail("compared_with", note);
    }
    if let Some(why) = tally.first_mismatch {
        result = result.with_detail("mismatch", why);
    }
    if let Some(why) = tally.first_inconclusive {
        result = result.with_detail("inconclusive", why);
    }
    if tally.mismatched > 0 && environment_differs {
        result = result.with_detail("inconclusive", ctx.environment_note());
    }
    if tally.retries > 0 {
        result = result.with_detail("skid_retries", tally.retries.to_string());
    }
    if tally.doubled {
        result = result
            .with_detail("skid_margin_doubled", "true")
            .with_detail(
                "caveat",
                "a rerun at double the skid margin is a different schedule and was not counted",
            );
    }
    if let Some(stop) = stopped {
        result = result.with_detail("stopped", stop);
    }
    result
}
