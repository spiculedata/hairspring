//! The session driver: one prepare, N restores given the checkpointed run's own input files, and M restores given another
//! seed file, on this host. It prints one JSON summary on standard output and exits 0 when the prepare worked and no
//! restore met an infrastructure error, 1 otherwise, 2 for a bad command line or missing setting.
//!
//! ```text
//! dhyve-session-driver --control N --other M [--seed S] [--deadline-secs D]
//! ```
//!
//! The restores run in a fixed order (two C, then one S, repeated) with a cap of 70 seconds each, group S is given the seed
//! one past the checkpointed run's, the session stops if the first restore is not counted, and a restore starts only if it
//! can end before the deadline. Every log and input file is kept in full, and a steps log and a record for each run are
//! written under `<state>/records`. A second prepare in one session is refused.
//!
//! The settings are the `HAIRSPRING_DHYVE_*` variables the backend reads. It runs unprivileged: the VM programs are
//! started through the pinned `sudo -n env -i LD_LIBRARY_PATH=...` shape. It is built only with the `driver` feature and
//! is no part of any default build.
use hairspring_dhyve::driver::{run_session, Plan};
use hairspring_dhyve::host::{HostConfig, Timings};
use hairspring_dhyve::logs::Bounds;
use hairspring_dhyve::session::Session;
use std::process::ExitCode;
use std::time::{Duration, Instant};

const USAGE: &str =
    "usage: dhyve-session-driver --control N --other M [--seed S] [--deadline-secs D]";
const DEFAULT_SEED: u64 = 1007;

fn number<T: std::str::FromStr>(args: &[String], flag: &str) -> Result<Option<T>, String> {
    match args.iter().position(|a| a == flag) {
        None => Ok(None),
        Some(i) => args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .map(Some)
            .ok_or_else(|| format!("{flag} needs a whole number")),
    }
}

fn plan(args: &[String]) -> Result<Plan, String> {
    let known = ["--control", "--other", "--seed", "--deadline-secs"];
    if let Some(bad) = args
        .iter()
        .step_by(2)
        .find(|a| !known.contains(&a.as_str()))
    {
        return Err(format!(
            "unknown argument `{}`",
            bad.chars().take(40).collect::<String>()
        ));
    }
    Ok(Plan {
        seed: number(args, "--seed")?.unwrap_or(DEFAULT_SEED),
        control_restores: number(args, "--control")?.ok_or("--control is required")?,
        other_seed_restores: number(args, "--other")?.ok_or("--other is required")?,
        deadline: number::<u64>(args, "--deadline-secs")?
            .map(|s| Instant::now() + Duration::from_secs(s)),
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let plan = match plan(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let host = match HostConfig::from_env() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let session = Session::new(host, Timings::standard(), Bounds::interim())
        .with_one_prepare()
        .with_keep_all();
    let summary = run_session(&session, &plan);
    println!("{}", summary.to_json());
    ExitCode::from(u8::from(!summary.clean()))
}
