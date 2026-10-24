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

/// The checks of the confirmation that was just stored: the newest one.
fn newest_confirmation(checks: &FailureChecks) -> Vec<CheckLine> {
    let Some(last) = checks.checks.last() else {
        return Vec::new();
    };
    checks
        .checks
        .iter()
        .filter(|c| c.confirmation_id == last.confirmation_id)
        .map(line_of_view)
        .collect()
}

/// Confirms one stored failure and prints what its checks now say.
fn confirm_stored(
    ctx: &Ctx,
    svc: &Service,
    failure_id: &str,
    req: &ConfirmRequest,
    json: bool,
    out: &mut dyn Write,
) -> CmdResult<i32> {
    let checks = svc.confirm_failure(&ctx.engine, failure_id, req, &mut |_| true)?;
    if json {
        writeln!(out, "{}", serde_json::to_string(&checks)?)?;
    } else {
        writeln!(out, "failure {failure_id}")?;
        write_confirmation(
            out,
            &newest_confirmation(&checks),
            &checks.finding,
            checks.environment_pinned,
        )?;
    }
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
        return confirm_stored(ctx, &svc, input, &req, args.json, out);
    }
    if input.starts_with(RUN_ID_PREFIX) {
        let detail = svc
            .get_run(input)?
            .ok_or_else(|| format!("no stored run `{input}`"))?;
        if detail.failures.is_empty() {
            return Err(format!("run `{input}` has no failure to confirm").into());
        }
        let mut code = crate::EXIT_OK;
        for failure in &detail.failures {
            code = code.max(confirm_stored(
                ctx,
                &svc,
                &failure.id,
                &req,
                args.json,
                out,
            )?);
        }
        return Ok(code);
    }
    confirm_bundle(ctx, &svc, input, &req, args.json, out)
}
