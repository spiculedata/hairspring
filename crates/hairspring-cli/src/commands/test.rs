use super::sweep_note;
use crate::cli::{Format, TestArgs};
use crate::ctx::{CmdResult, Ctx};
use crate::host::{self, HostFacts};
use hairspring_engine::config::Budget;
use hairspring_engine::format::{to_junit, to_sarif, to_text};
use hairspring_engine::mode::RunMode;
use hairspring_engine::report::{RunOutcome, RunStatus};
use hairspring_hermit::interrupt;
use hairspring_service::StartRun;
use hairspring_store::RunState;
use hairspring_store::{random_seed, sha256_hex};
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
        "{}-{}.hairspring",
        safe_file_stem(target),
        &sha256_hex(text.as_bytes())[..8]
    ));
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// Writes a bundle file for each failure of the outcome (a sweep has one for each property's first failing seed), the
/// lowest seed first, and returns their paths.
fn write_failure_bundles(
    dir: &Path,
    target: &str,
    outcome: &RunOutcome,
) -> CmdResult<Vec<std::path::PathBuf>> {
    outcome
        .failure
        .iter()
        .chain(&outcome.more_failures)
        .map(|f| write_bundle(dir, target, &f.bundle))
        .collect()
}

pub fn run(ctx: &Ctx, args: &TestArgs, out: &mut dyn Write, err: &mut dyn Write) -> CmdResult<i32> {
    let names: Vec<String> = if args.targets.is_empty() {
        ctx.engine.target_names()
    } else {
        args.targets.clone()
    };
    if host::involves_process_target(ctx.engine.config(), &names) {
        host::write_warnings(err, &HostFacts::probe())?;
    }
    let svc = ctx.service()?;
    if let Err(e) = interrupt::install_sigint_handler() {
        writeln!(
            err,
            "warning: cannot catch Ctrl-C ({e}); it will end hairspring without stopping Hermit"
        )?;
    }
    let (seed, requested_trials) = match args.seeds {
        Some(range) => (range.start, Some(range.trials())),
        None => (args.seed.unwrap_or_else(random_seed), args.trials),
    };
    let mode = if args.sweep {
        RunMode::Sweep
    } else {
        RunMode::Search
    };
    let text_mode = args.format == Format::Text;
    let mut results: Vec<(String, RunOutcome)> = Vec::new();
    let mut code = crate::EXIT_OK;

    for name in &names {
        if interrupt::requested() {
            writeln!(
                err,
                "hairspring: interrupted; the remaining targets were not run"
            )?;
            return Ok(crate::EXIT_INTERRUPTED);
        }
        let spec = ctx
            .engine
            .config()
            .targets
            .get(name)
            .ok_or_else(|| format!("unknown target `{name}`"))?;
        let budget = Budget {
            max_steps: args.budget_steps.unwrap_or(spec.budget.max_steps),
            max_trials: requested_trials.unwrap_or(spec.budget.max_trials),
            wall_secs: args.budget_secs.unwrap_or(spec.budget.wall_secs),
        };
        // Trial n runs at seed + n. Past the largest seed that would wrap round to 0, which a range written as
        // FIRST..END cannot say, so such a sweep is refused rather than printed as a range it did not run.
        if mode.is_sweep()
            && seed
                .checked_add(budget.max_trials.saturating_sub(1))
                .is_none()
        {
            return Err(format!(
                "a sweep of {} trials from seed {seed} would pass the largest seed ({}); start lower",
                budget.max_trials,
                u64::MAX
            )
            .into());
        }
        if text_mode {
            match mode {
                RunMode::Search => writeln!(out, "hairspring: exploring {name} (seed {seed})")?,
                RunMode::Sweep => writeln!(
                    out,
                    "hairspring: sweeping {name} (seeds {seed}..{})",
                    u128::from(seed) + u128::from(budget.max_trials)
                )?,
            }
        }
        let detail = svc.run_sync_until(
            &ctx.engine,
            &StartRun {
                target: name.clone(),
                seed: Some(seed),
                budget: Some(budget),
            },
            mode,
            interrupt::flag(),
        )?;
        if interrupt::requested() && detail.run.state == RunState::Cancelled {
            writeln!(
                err,
                "hairspring: interrupted; run {} is cancelled, the trials already judged are kept and its Hermit processes were stopped",
                detail.run.id
            )?;
            return Ok(crate::EXIT_INTERRUPTED);
        }
        let outcome = svc.outcome(&detail.run.id)?.ok_or("run vanished")?;
        let bundles = write_failure_bundles(&args.bundle_dir, name, &outcome)?;
        if text_mode {
            write!(out, "{}", to_text(name, &outcome))?;
        }
        for path in &bundles {
            let note = format!(
                "bundle written: {}  (replay with `hairspring replay {}`)",
                path.display(),
                path.display()
            );
            if text_mode {
                writeln!(out, "  {note}")?;
            } else {
                writeln!(err, "{name}: {note}")?;
            }
        }
        if mode.is_sweep() && text_mode {
            let completion = svc.completion_for(&detail.run)?;
            if let Some(note) = sweep_note::note(&completion, outcome.trials, budget.max_trials) {
                writeln!(out, "  {note}")?;
            }
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
    if interrupt::requested() {
        // Ctrl-C landed after the last run finished on its own. That run stands as stored; the command still says it
        // was told to stop.
        writeln!(
            err,
            "hairspring: interrupted after the last run had finished; its result stands"
        )?;
    }
    Ok(crate::status_after_interrupt(code, interrupt::requested()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_core::bundle::Bundle;
    use hairspring_engine::report::FailureReport;

    fn report(seed: u64) -> FailureReport {
        FailureReport {
            target: "svc".into(),
            backend: "hermit".into(),
            message: format!("seed {seed}"),
            original_len: 0,
            shrunk_len: 0,
            steps: vec![],
            bundle: Bundle::new(seed, "t", vec![]).with_target("svc").encode(),
        }
    }

    fn dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hairspring-cli-bundles-{name}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn a_bundle_is_written_for_every_failure_of_the_outcome_lowest_seed_first() {
        let out = RunOutcome::failed(1, 1, 1, report(3)).with_more_failures(vec![report(7)]);
        let d = dir("many");
        let paths = write_failure_bundles(&d, "svc", &out).unwrap();
        assert_eq!(paths.len(), 2);
        let seeds: Vec<u64> = paths
            .iter()
            .map(|p| {
                Bundle::decode(&std::fs::read_to_string(p).unwrap())
                    .unwrap()
                    .seed
            })
            .collect();
        assert_eq!(seeds, vec![3, 7]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_passing_outcome_writes_no_bundle() {
        let d = dir("none");
        let paths = write_failure_bundles(&d, "svc", &RunOutcome::passed(1, 1, 1)).unwrap();
        assert!(paths.is_empty());
        assert!(!d.exists());
    }
}
