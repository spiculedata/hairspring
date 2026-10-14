use crate::cli::{BundleCommand, ResultsArgs, ShowArgs};
use crate::ctx::{CmdResult, Ctx};
use crate::timefmt::format_utc;
use anthill_engine::format::to_text;
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
    writeln!(
        out,
        "run {} seed {} created {}",
        detail.run.id,
        detail.run.seed,
        format_utc(detail.run.created_at)
    )?;
    Ok(crate::EXIT_OK)
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
