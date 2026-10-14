//! `anthill-dapr-sim`: run an app against a seeded simulated Dapr sidecar.
//!
//!     anthill-dapr-sim [--scenario FILE] [--seed N] [-- APP COMMAND...]
//!
//! The seed defaults to `$ANTHILL_SEED` (set by the Hermit backend) and then 1. Exit codes:
//! 0 all invariants held, 1 a violation was found (printed as `ANTHILL_VIOLATION: ...`), 2 error.
use anthill_dapr::{run_simulation, ProcessApp, Scenario, Verdict};
use std::process::ExitCode;

const DEFAULT_SCENARIO: &str = "dapr-scenario.toml";

fn parse_args() -> Result<(String, Option<u64>, Vec<String>), String> {
    let mut scenario = DEFAULT_SCENARIO.to_string();
    let mut seed = None;
    let mut command = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--scenario" => scenario = args.next().ok_or("--scenario needs a file")?,
            "--seed" => {
                seed = Some(
                    args.next()
                        .ok_or("--seed needs a number")?
                        .parse::<u64>()
                        .map_err(|_| "--seed must be a non-negative integer")?,
                )
            }
            "--" => {
                command = args.collect();
                break;
            }
            "-h" | "--help" => {
                return Err(
                    "usage: anthill-dapr-sim [--scenario FILE] [--seed N] [-- APP COMMAND...]"
                        .to_string(),
                )
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok((scenario, seed, command))
}

fn main() -> ExitCode {
    let (path, seed, command) = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("anthill-dapr-sim: {e}");
            return ExitCode::from(2);
        }
    };
    let seed = seed
        .or_else(|| {
            std::env::var("ANTHILL_SEED")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(1);
    let mut scenario = match std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {path}: {e}"))
        .and_then(|t| Scenario::parse(&t))
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("anthill-dapr-sim: {e}");
            return ExitCode::from(2);
        }
    };
    if !command.is_empty() {
        scenario.app.command = command;
    }
    let mut app = ProcessApp::new(scenario.app.clone());
    match run_simulation(seed, &scenario, &mut app) {
        Verdict::Ok(r) => {
            println!(
                "anthill-dapr-sim: ok (seed {seed}): {} delivered, {} redelivered, {} crashes",
                r.delivered, r.redelivered, r.crashes
            );
            if !r.invariants_checked {
                println!("anthill-dapr-sim: warning: the app has no /anthill/check endpoint, so only delivery was checked, not your invariants");
            }
            ExitCode::SUCCESS
        }
        Verdict::Violation(m) => {
            println!("ANTHILL_VIOLATION: {m}");
            ExitCode::from(1)
        }
        Verdict::Error(m) => {
            eprintln!("anthill-dapr-sim: error: {m}");
            ExitCode::from(2)
        }
    }
}
