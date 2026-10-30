//! Check 4: Hermit's strict determinism verify.
//!
//! `hermit run --verify --verify-allow=both --verify-strict --verify-json=F` runs the guest twice and compares the
//! two runs bit for bit. Hermit's `--verify-allow` is the exit status the guest is *required* to have in both runs
//! (`success`, `failure` or `both`), not an allowance: `failure` would reject a guest that exits 0, which is how a
//! target that signals its finding through output behaves, and Hermit would then skip the second run. `both` accepts
//! either status; Hermit still compares the exit status of the two runs (a difference is a divergence), but nothing in
//! Hermit's result says the guest failed. So the check passes only if the result file says `verified`,
//! `bitwise_parity` and a `matched` verdict, and the verify run itself failed the way the recording did: the same
//! kind of failure and, where the recording names one, the same property. A match in which the guest did not fail is a
//! miss, handled as the rerun check handles one. It costs two runs, so it is asked for.
use super::command;
use super::context::{now_ms, Ctx};
use super::pace::Pace;
use super::verify_report as report;
use crate::backend::{failure_kind_matches, property_matches};
use crate::exec::{run_trial_with, Dirs, Trial};
use crate::trace::Verdict;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus};
use serde_json::Value;
use std::io::Read;
use std::path::Path;

/// The result file is a small JSON object; anything larger is not one and is not read.
const MAX_RESULT_BYTES: u64 = 64 * 1024;
/// The name the stored command gives the result file, which really is a private file chosen per run.
const DISPLAY_FILE: &str = "verify.json";
const VERIFY_FLAGS: [&str; 3] = ["--verify", "--verify-allow=both", "--verify-strict"];

fn flags(json: &str) -> Vec<String> {
    VERIFY_FLAGS
        .iter()
        .map(|f| f.to_string())
        .chain(std::iter::once(format!("--verify-json={json}")))
        .collect()
}

/// What Hermit's result file said, or why it said nothing.
fn read_result(path: &Path) -> Result<Value, &'static str> {
    let mut text = String::new();
    let opened = std::fs::File::open(path).map_err(|_| "Hermit left no result for the verify")?;
    opened
        .take(MAX_RESULT_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| "the verify's result could not be read")?;
    if text.len() as u64 > MAX_RESULT_BYTES {
        return Err("the verify's result is too large to be one");
    }
    serde_json::from_str(&text).map_err(|_| "the verify's result is not JSON")
}

fn passes(v: &Value) -> bool {
    v.get("verified").and_then(Value::as_bool) == Some(true)
        && v.get("bitwise_parity").and_then(Value::as_bool) == Some(true)
        && v.get("verdict").and_then(Value::as_str) == Some("matched")
}

pub(super) fn check(ctx: &Ctx, pace: &mut Pace) -> CheckResult {
    let started = now_ms();
    let kind = CheckKind::StrictVerify;
    let command = command::line(&ctx.backend.settings, ctx.p, ctx.seed, &flags(DISPLAY_FILE));
    let inconclusive = |why: &str| {
        ctx.result(kind, CheckStatus::Inconclusive, started)
            .with_command(command.clone())
            .with_detail("inconclusive", why)
    };
    let dirs = ctx.backend.dirs();
    let Some(private) = dirs.summaries else {
        return inconclusive(
            "no private summary directory is configured, so the verify has nowhere to write its result",
        );
    };
    let result_path = private.file_for(ctx.seed);
    let extra = flags(&result_path.display().to_string());
    // The two runs get twice a single run's time; the result file replaces the summary file.
    let timeout = pace.run_timeout(ctx.backend.trial_timeout(ctx.p) * 2);
    let run = run_trial_with(
        &ctx.backend.settings,
        ctx.p,
        ctx.seed,
        timeout,
        Dirs {
            summaries: None,
            events: dirs.events,
            trials: dirs.trials,
        },
        &extra,
    );
    pace.ran("strict verify".to_string());
    // Read before the file is removed: its verdict and reason are what the detail has to show.
    let read = read_result(&result_path);
    let _ = std::fs::remove_file(&result_path);
    let trial = match run {
        Ok(t) => t,
        Err(_) => return inconclusive("Hermit could not be started (host fault)"),
    };
    let verdict = ctx.backend.verdict(ctx.p, &trial, timeout);
    if trial.timed_out || matches!(verdict, Verdict::Hang(_)) {
        return inconclusive(&report::timed_out(timeout));
    }
    let error_line = trial.stderr_findings().error_line;
    let hermit_failed = match (&verdict, &read) {
        (_, Ok(v)) if report::is_no_comparison(v) => Some(report::no_comparison_text(v)),
        (Verdict::Infra(why), _) => Some(report::hermit_failure(why, None)),
        _ => None,
    };
    if let Some(text) = hermit_failed {
        return inconclusive(&report::with_error_line(&text, error_line.as_deref()));
    }
    let v = match read {
        Ok(v) => v,
        Err(why) => return inconclusive(&report::with_error_line(why, error_line.as_deref())),
    };
    let parity = passes(&v);
    let shown = if parity {
        finding_shown(ctx, &trial, &verdict)
    } else {
        Shown::Yes
    };
    let reported = ctx
        .result(kind, status_of(parity, &shown, ctx), started)
        .with_command(command.clone())
        .with_detail(
            "verdict",
            v.get("verdict")
                .and_then(Value::as_str)
                .unwrap_or("none")
                .to_string(),
        )
        .with_detail(
            "bitwise_parity",
            (v.get("bitwise_parity").and_then(Value::as_bool) == Some(true)).to_string(),
        );
    match (parity, shown) {
        (false, _) | (true, Shown::Yes) => {
            let ok = parity;
            reported.with_counts(u32::from(ok), u32::from(!ok), 0)
        }
        (true, Shown::Missed(why)) if !ctx.env.differs() => {
            reported.with_counts(0, 1, 0).with_detail("mismatch", why)
        }
        (true, Shown::Missed(_)) => reported
            .with_counts(0, 0, 1)
            .with_detail("inconclusive", ctx.environment_note()),
        (true, Shown::Unjudged(why)) => reported
            .with_counts(0, 0, 1)
            .with_detail("inconclusive", why),
    }
}

/// Whether the guest failed in the verify the way the recording says it failed. Hermit's `--verify-allow=both` accepts
/// any exit status, so matching runs say nothing about the finding: a guest that does not fail in the verify (a
/// failure that depends on how many runs came before, say) would otherwise pass.
enum Shown {
    Yes,
    /// The guest ran and was judged, and did not fail the recorded way.
    Missed(&'static str),
    /// The verify's run could not be judged.
    Unjudged(&'static str),
}

fn finding_shown(ctx: &Ctx, trial: &Trial, verdict: &Verdict) -> Shown {
    match verdict {
        Verdict::Violation(_) | Verdict::Deadlock(_) => {}
        Verdict::Pass => return Shown::Missed("the guest did not fail in the verify"),
        _ => return Shown::Unjudged("the guest's verify run could not be judged"),
    }
    if failure_kind_matches(ctx.bundle, verdict) == Some(false) {
        return Shown::Missed("the guest failed in a different kind of way in the verify");
    }
    if property_matches(ctx.bundle, trial) == Some(false) {
        return Shown::Missed("a different property failed in the verify");
    }
    Shown::Yes
}

/// A result follows the rerun check: a miss is a failure of the check on an unchanged environment and inconclusive on
/// a changed one; a run that could not be judged is inconclusive.
fn status_of(parity: bool, shown: &Shown, ctx: &Ctx) -> CheckStatus {
    match (parity, shown) {
        (false, _) => CheckStatus::Fail,
        (true, Shown::Yes) => CheckStatus::Pass,
        (true, Shown::Missed(_)) if !ctx.env.differs() => CheckStatus::Fail,
        (true, _) => CheckStatus::Inconclusive,
    }
}
