//! Fold a captured guest event stream and print the verdicts.
//!
//! `cargo run -q -p hairspring-props --example fold_events -- trial.events [more.events ...]`
//!
//! Each file is one trial (a process's stderr, or its `HAIRSPRING_EVENTS_FILE`). Prints one line per property with
//! its run-level status over the trials given, then the trial-level failures and the health of each trial.

use hairspring_props::{RunProps, TrialEvents};
use std::process::ExitCode;

fn main() -> ExitCode {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: fold_events <events-file>...");
        return ExitCode::from(2);
    }
    let mut run = RunProps::new();
    for (seed, path) in (0u64..).zip(&paths) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::from(2);
            }
        };
        let trial = TrialEvents::from_output(&bytes).fold();
        println!(
            "trial {path}: degraded={} perturbing={} report_missing={:?} procs={}",
            trial.is_degraded(),
            trial.perturbing,
            trial.report_missing(),
            trial.procs.len()
        );
        for failure in trial.verdict() {
            println!("  failure: {}", failure.describe());
        }
        run = run.add(seed, &trial);
    }
    for stat in run.finish() {
        println!(
            "{:<10} {} ({:?}, trials={}, hit={}, true={}, false={})",
            format!("{:?}", stat.status).to_uppercase(),
            stat.id,
            stat.kind,
            stat.trials,
            stat.trials_hit,
            stat.trials_true,
            stat.trials_false
        );
    }
    ExitCode::SUCCESS
}
