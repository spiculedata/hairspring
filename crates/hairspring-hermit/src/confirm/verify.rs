//! Check 4: Hermit's strict determinism verify.
//!
//! `hermit run --verify --verify-allow=failure --verify-strict --verify-json=F` runs the guest twice and compares the
//! two runs bit for bit; the guest's own failure is allowed, since that is the finding. The check passes only if the
//! result file says `verified`, `bitwise_parity` and a `matched` verdict. It costs two runs, so it is asked for.
use super::command;
use super::context::{now_ms, Ctx};
use super::pace::Pace;
use crate::exec::{run_trial_with, Dirs};
use crate::trace::Verdict;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus};
use serde_json::Value;
use std::io::Read;
use std::path::Path;

/// The result file is a small JSON object; anything larger is not one and is not read.
const MAX_RESULT_BYTES: u64 = 64 * 1024;
/// The name the stored command gives the result file, which really is a private file chosen per run.
const DISPLAY_FILE: &str = "verify.json";
const VERIFY_FLAGS: [&str; 3] = ["--verify", "--verify-allow=failure", "--verify-strict"];

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
    let read = read_result(&result_path);
    let _ = std::fs::remove_file(&result_path);
    let trial = match run {
        Ok(t) => t,
        Err(_) => return inconclusive("Hermit could not be started (host fault)"),
    };
    if matches!(
        ctx.backend.verdict(ctx.p, &trial, timeout),
        Verdict::Infra(_) | Verdict::Hang(_)
    ) {
        return inconclusive("the verify run failed in Hermit or timed out, so it says nothing");
    }
    match read {
        Err(why) => inconclusive(why),
        Ok(v) => {
            let ok = passes(&v);
            let verdict = v
                .get("verdict")
                .and_then(Value::as_str)
                .unwrap_or("none")
                .to_string();
            ctx.result(
                kind,
                if ok {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                },
                started,
            )
            .with_counts(u32::from(ok), u32::from(!ok), 0)
            .with_command(command.clone())
            .with_detail("verdict", verdict)
            .with_detail(
                "bitwise_parity",
                (v.get("bitwise_parity").and_then(Value::as_bool) == Some(true)).to_string(),
            )
        }
    }
}
