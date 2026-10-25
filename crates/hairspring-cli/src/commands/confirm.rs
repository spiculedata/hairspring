use super::confirm_text::{exit_code, write_confirmation, CheckLine};
use crate::cli::ConfirmArgs;
use crate::ctx::{CmdResult, Ctx};
use hairspring_core::bundle::Bundle;
use hairspring_engine::confirm::{environment_pinned, finding_status, CheckResult, ConfirmRequest};
use hairspring_service::{CheckView, FailureChecks, Service, MAX_BUNDLE_BYTES};
use std::io::{Read, Write};
use std::path::Path;

const RUN_ID_PREFIX: &str = "run_";
const FAILURE_ID_PREFIX: &str = "fail_";

fn request(args: &ConfirmArgs) -> ConfirmRequest {
    ConfirmRequest {
        reruns: args.reruns,
        control: args.control.clone(),
        strict_verify: args.strict_verify,
        allow_environment_change: args.allow_environment_change,
        wall_secs: args.wall_secs,
    }
}

fn line_of_result(c: &CheckResult) -> CheckLine {
    CheckLine {
        kind: c.kind.as_str().to_owned(),
        status: c.status.as_str().to_owned(),
        passed: c.counts.passed,
        failed: c.counts.failed,
        inconclusive: c.counts.inconclusive,
        command: c.command.clone(),
        detail: c.detail.clone(),
    }
}

fn line_of_view(c: &CheckView) -> CheckLine {
    CheckLine {
        kind: c.kind.clone(),
        status: c.status.clone(),
        passed: c.counts.passed,
        failed: c.counts.failed,
        inconclusive: c.counts.inconclusive,
        command: c.guest.command.clone(),
        detail: c.guest.detail.clone(),
    }
}

/// The checks of the confirmation the finding is derived from, which is the one just stored unless that one did not
/// finish (its wall budget ran out): then the finding comes from an earlier one, and those are its lines.
fn source_confirmation(checks: &FailureChecks) -> Vec<CheckLine> {
    checks
        .checks
        .iter()
        .filter(|c| Some(&c.confirmation_id) == checks.confirmation_id.as_ref())
        .map(line_of_view)
        .collect()
}

/// Whether the newest stored confirmation is not the one the finding comes from.
fn newest_is_set_aside(checks: &FailureChecks) -> bool {
    match (checks.checks.last(), &checks.confirmation_id) {
        (Some(last), Some(source)) => &last.confirmation_id != source,
        _ => false,
    }
}

/// Confirms one stored failure and returns what its checks now say.
fn confirm_stored(
    ctx: &Ctx,
    svc: &Service,
    failure_id: &str,
    req: &ConfirmRequest,
) -> CmdResult<FailureChecks> {
    Ok(svc.confirm_failure(&ctx.engine, failure_id, req, &mut |_| true)?)
}

fn write_stored(out: &mut dyn Write, checks: &FailureChecks) -> CmdResult<i32> {
    writeln!(out, "failure {}", checks.failure_id)?;
    if newest_is_set_aside(checks) {
        writeln!(
            out,
            "this confirmation did not finish (cancelled, or out of time), so the finding comes from the last one that did; its checks are shown"
        )?;
    }
    write_confirmation(
        out,
        &source_confirmation(checks),
        &checks.finding,
        checks.environment_pinned,
    )?;
    Ok(exit_code(&checks.finding))
}

fn bundle_text(ctx: &Ctx, input: &str) -> CmdResult<String> {
    if input == "-" {
        let mut s = String::new();
        std::io::stdin()
            .take(MAX_BUNDLE_BYTES as u64 + 1)
            .read_to_string(&mut s)?;
        return Ok(s);
    }
    if Path::new(input).is_file() {
        let size = std::fs::metadata(input)?.len() as usize;
        if size > MAX_BUNDLE_BYTES {
            return Err(format!("bundle file exceeds {MAX_BUNDLE_BYTES} bytes").into());
        }
        return Ok(std::fs::read_to_string(input)?);
    }
    if input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Some(text) = ctx.service()?.get_bundle(input)? {
            return Ok(text);
        }
    }
    Err(format!(
        "`{input}` is not a run or failure id, a readable bundle file or a stored bundle hash"
    )
    .into())
}

/// Confirms a bundle that is not a stored failure. Nothing is stored.
fn confirm_bundle(
    ctx: &Ctx,
    svc: &Service,
    input: &str,
    req: &ConfirmRequest,
    json: bool,
    out: &mut dyn Write,
) -> CmdResult<i32> {
    let text = bundle_text(ctx, input)?;
    let checks = svc.confirm_bundle(&ctx.engine, &text, req, &mut |_| true)?;
    let finding = finding_status(&checks).as_str();
    if json {
        let value = serde_json::json!({ "finding": finding, "stored": false, "checks": checks });
        writeln!(out, "{value}")?;
    } else {
        let bundle = Bundle::decode(&text).map_err(|e| format!("invalid bundle: {e}"))?;
        writeln!(
            out,
            "bundle of target `{}` (not stored: it is not a stored failure; confirm its run id or failure id to keep the checks)",
            bundle.target
        )?;
        let lines: Vec<CheckLine> = checks.iter().map(line_of_result).collect();
        write_confirmation(out, &lines, finding, environment_pinned(&checks))?;
    }
    Ok(exit_code(finding))
}

pub fn run(ctx: &Ctx, args: &ConfirmArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let req = request(args);
    let svc = ctx.service()?;
    let input = args.input.as_str();
    if input.starts_with(FAILURE_ID_PREFIX) {
        let checks = confirm_stored(ctx, &svc, input, &req)?;
        if args.json {
            writeln!(out, "{}", serde_json::to_string(&checks)?)?;
            return Ok(exit_code(&checks.finding));
        }
        return write_stored(out, &checks);
    }
    if input.starts_with(RUN_ID_PREFIX) {
        let detail = svc
            .get_run(input)?
            .ok_or_else(|| format!("no stored run `{input}`"))?;
        if detail.failures.is_empty() {
            return Err(format!("run `{input}` has no failure to confirm").into());
        }
        let mut all = Vec::new();
        for failure in &detail.failures {
            all.push(confirm_stored(ctx, &svc, &failure.id, &req)?);
        }
        if args.json {
            // One document however many failures the run has: an array with an entry per failure.
            writeln!(out, "{}", serde_json::to_string(&all)?)?;
            return Ok(all
                .iter()
                .map(|c| exit_code(&c.finding))
                .fold(crate::EXIT_OK, i32::max));
        }
        let mut code = crate::EXIT_OK;
        for checks in &all {
            code = code.max(write_stored(out, checks)?);
        }
        return Ok(code);
    }
    confirm_bundle(ctx, &svc, input, &req, args.json, out)
}
