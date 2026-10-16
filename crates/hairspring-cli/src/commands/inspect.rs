use crate::cli::{BundleCommand, ResultsArgs, ShowArgs};
use crate::ctx::{CmdResult, Ctx};
use crate::timefmt::format_utc;
use hairspring_engine::format::to_text;
use hairspring_service::RunDetail;
use std::io::Write;

pub fn targets(ctx: &Ctx, out: &mut dyn Write) -> CmdResult<i32> {
    for t in ctx.engine.config().targets.values() {
        writeln!(out, "{:<24} {:<8} {}", t.name, t.kind.name(), t.description)?;
    }
    Ok(crate::EXIT_OK)
}

pub fn results(ctx: &Ctx, args: &ResultsArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let svc = ctx.service()?;
    for r in svc.list_runs(args.limit)? {
        writeln!(
            out,
            "{}  {:<9} {:<22} {:>10} steps  {}",
            r.id,
            r.state.as_str(),
            r.target,
            r.steps,
            format_utc(r.created_at)
        )?;
    }
    Ok(crate::EXIT_OK)
}

pub fn show(ctx: &Ctx, args: &ShowArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let svc = ctx.service()?;
    let detail = svc
        .get_run(&args.id)?
        .ok_or_else(|| format!("no stored run `{}`", args.id))?;
    let outcome = svc
        .outcome(&args.id)?
        .ok_or_else(|| format!("no stored run `{}`", args.id))?;
    write!(out, "{}", to_text(&detail.run.target, &outcome))?;
    write_property_labels(&detail, out)?;
    writeln!(
        out,
        "run {} seed {} created {}",
        detail.run.id,
        detail.run.seed,
        format_utc(detail.run.created_at)
    )?;
    Ok(crate::EXIT_OK)
}

/// What a reader needs to weigh the property figures: whose data they are, whether all of it arrived, and whether the
/// run had a fixed budget.
fn write_property_labels(detail: &RunDetail, out: &mut dyn Write) -> CmdResult<()> {
    let Some(p) = &detail.properties else {
        return Ok(());
    };
    if p.unverified_worker_data {
        writeln!(out, "  unverified worker data: some trials came from a worker that is not trusted, so these figures are not claims")?;
    }
    if p.ingest_incomplete {
        writeln!(out, "  trial data incomplete: the trials stored do not match the trials the worker ran, so no must-hit claim is made")?;
    }
    if p.stopped_at_first_failure {
        writeln!(out, "  the run stopped at its first failure, so its trial count is not a fixed budget and these are not failure rates")?;
    }
    Ok(())
}

pub fn bundle(ctx: &Ctx, cmd: &BundleCommand, out: &mut dyn Write) -> CmdResult<i32> {
    let svc = ctx.service()?;
    let (hash, output) = match cmd {
        BundleCommand::Show { hash } => (hash, None),
        BundleCommand::Export { hash, output } => (hash, Some(output)),
    };
    let text = svc
        .get_bundle(hash)?
        .ok_or_else(|| format!("no stored bundle `{hash}`"))?;
    match output {
        None => write!(out, "{text}")?,
        Some(path) => {
            std::fs::write(path, &text)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            writeln!(out, "wrote {}", path.display())?;
        }
    }
    Ok(crate::EXIT_OK)
}
