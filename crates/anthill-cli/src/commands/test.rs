use crate::cli::{Format, TestArgs};
use crate::ctx::{CmdResult, Ctx};
use anthill_engine::config::Budget;
use anthill_engine::format::{to_junit, to_sarif, to_text};
use anthill_engine::report::{RunOutcome, RunStatus};
use anthill_service::StartRun;
use anthill_store::{random_seed, sha256_hex};
use serde_json::json;
use std::io::Write;
use std::path::Path;

fn safe_file_stem(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn write_bundle(dir: &Path, target: &str, text: &str) -> CmdResult<std::path::PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(format!(
        "{}-{}.anthill",
        safe_file_stem(target),
        &sha256_hex(text.as_bytes())[..8]
    ));
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

pub fn run(ctx: &Ctx, args: &TestArgs, out: &mut dyn Write, err: &mut dyn Write) -> CmdResult<i32> {
    let names: Vec<String> = if args.targets.is_empty() {
        ctx.engine.target_names()
    } else {
        args.targets.clone()
    };
    let svc = ctx.service()?;
    let seed = args.seed.unwrap_or_else(random_seed);
    let text_mode = args.format == Format::Text;
    let mut results: Vec<(String, RunOutcome)> = Vec::new();
    let mut code = crate::EXIT_OK;

    for name in &names {
        let spec = ctx
            .engine
            .config()
            .targets
            .get(name)
            .ok_or_else(|| format!("unknown target `{name}`"))?;
        let budget = Budget {
            max_steps: args.budget_steps.unwrap_or(spec.budget.max_steps),
            wall_secs: args.budget_secs.unwrap_or(spec.budget.wall_secs),
            ..spec.budget
        };
        if text_mode {
            writeln!(out, "anthill: exploring {name} (seed {seed})")?;
        }
        let detail = svc.run_sync(
            &ctx.engine,
            &StartRun {
                target: name.clone(),
                seed: Some(seed),
                budget: Some(budget),
            },
        )?;
        let outcome = svc.outcome(&detail.run.id)?.ok_or("run vanished")?;
        if let Some(f) = &outcome.failure {
            let path = write_bundle(&args.bundle_dir, name, &f.bundle)?;
            let note = format!(
                "bundle written: {}  (replay with `anthill replay {}`)",
                path.display(),
                path.display()
            );
            if text_mode {
                write!(out, "{}", to_text(name, &outcome))?;
                writeln!(out, "  {note}")?;
            } else {
                writeln!(err, "{name}: {note}")?;
            }
        } else if text_mode {
            write!(out, "{}", to_text(name, &outcome))?;
        }
        code = code.max(match outcome.status {
            RunStatus::Passed => crate::EXIT_OK,
            RunStatus::Failed => crate::EXIT_VIOLATION,
            RunStatus::Error => crate::EXIT_ERROR,
        });
        results.push((name.clone(), outcome));
    }

    match args.format {
        Format::Text => {}
        Format::Json => {
            let items: Vec<_> = results
                .iter()
                .map(|(t, o)| json!({ "target": t, "outcome": o }))
                .collect();
            writeln!(out, "{}", serde_json::to_string_pretty(&items)?)?;
        }
        Format::Junit => write!(out, "{}", to_junit(&results))?,
        Format::Sarif => writeln!(out, "{}", to_sarif(&results))?,
    }
    Ok(code)
}
