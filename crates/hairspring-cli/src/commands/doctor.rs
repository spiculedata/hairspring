use crate::cli::DoctorArgs;
use crate::ctx::{CmdResult, Ctx};
use hairspring_engine::config::TargetKind;
use std::io::Write;

pub fn run(ctx: &Ctx, args: &DoctorArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let names: Vec<String> = if args.targets.is_empty() {
        ctx.engine
            .config()
            .targets
            .values()
            .filter(|t| matches!(t.kind, TargetKind::Sim { .. }))
            .map(|t| t.name.clone())
            .collect()
    } else {
        args.targets.clone()
    };
    let mut code = crate::EXIT_OK;
    for name in names {
        let report = ctx.engine.doctor(&name, args.seeds, args.steps)?;
        match report.divergence {
            None => writeln!(
                out,
                "{name}: deterministic over {} seeds x {} steps",
                report.seeds_checked, report.steps_per_seed
            )?,
            Some(d) => {
                code = crate::EXIT_VIOLATION;
                writeln!(out, "{name}: NON-DETERMINISTIC (seed {}, diverged at step {}). Look for HashMap iteration, time, threads or randomness outside the simulation.", d.seed, d.step)?;
            }
        }
    }
    Ok(code)
}
