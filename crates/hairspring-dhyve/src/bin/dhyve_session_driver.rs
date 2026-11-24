//! The session driver: one prepare, N restores given the checkpointed run's own input files, and M restores given another
//! seed file, on this host. It prints one JSON summary on standard output and exits 0 when the prepare worked, no restore
//! met an infrastructure error and the session ran to its end, 1 otherwise, and 2 for a bad command line, a missing setting
//! or a build it will not run.
//!
//! ```text
//! dhyve-session-driver --control N --other M [--seed S] [--deadline-secs D] [--rdrand-accepted] [--allow-fake]
//! dhyve-session-driver --build-info
//! ```
//!
//! The restores run in a fixed order (two C, then one S, repeated) with a cap of 70 seconds each, group S is given the seed
//! one past the checkpointed run's, the session stops if the first restore is not counted, and a restore starts only if it
//! can end before the deadline. `--deadline-secs D` is seconds from this program's own start, so whoever starts it hands in
//! what is left of the session's time (the session's end minus the time now). Every log and input file is kept in full, and
//! a steps log and a record for each run are written under `<state>/records`. A second prepare in one session is refused.
//!
//! `--rdrand-accepted` lets the session go on although the prepare's standard error holds the warning that the CPU cannot
//! trap the random-number instruction; without it that warning ends the session (`END:RDRAND`). The machine check runs
//! first and ends the session (`END:PROBE`) when it fails; a build with the stand-in programs runs none.
//!
//! The settings are the `HAIRSPRING_DHYVE_*` variables the backend reads. A session needs its stamp and all three pins
//! (`HAIRSPRING_DHYVE_STAMP`, `HAIRSPRING_DHYVE_PIN_BHYVE`, `HAIRSPRING_DHYVE_PIN_BHYVECTL`, `HAIRSPRING_DHYVE_PIN_LIB`);
//! without them it exits 2 before anything is started. It runs unprivileged: the VM programs are started through the pinned
//! `sudo -n env -i LD_LIBRARY_PATH=...` shape.
//!
//! It is built only with the `driver` feature, which is no part of a default build. A build that also has the `fake-vmm`
//! feature (the stand-in programs, which re-open the program overrides) refuses to run without `--allow-fake`, and every
//! summary says whether the stand-in programs are built in (`fake_vmm`); `--build-info` prints only that.
use hairspring_dhyve::driver::{run_session, Plan};
use hairspring_dhyve::host::{
    HostConfig, Timings, ENV_PIN_BHYVE, ENV_PIN_BHYVECTL, ENV_PIN_LIB, ENV_STAMP,
};
use hairspring_dhyve::logs::Bounds;
use hairspring_dhyve::session::Session;
use serde_json::{json, Value};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: dhyve-session-driver --control N --other M [--seed S] [--deadline-secs D]\n       dhyve-session-driver --build-info";
const DEFAULT_SEED: u64 = 1007;
const FAKE_BUILT: bool = cfg!(feature = "fake-vmm");

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
    let mut plan = Plan::new(
        number(args, "--seed")?.unwrap_or(DEFAULT_SEED),
        number(args, "--control")?.ok_or("--control is required")?,
        number(args, "--other")?.ok_or("--other is required")?,
    );
    plan.deadline =
        number::<u64>(args, "--deadline-secs")?.map(|s| Instant::now() + Duration::from_secs(s));
    Ok(plan)
}

/// The setting a session needs that is not there, if any.
fn missing_for_a_session(host: &HostConfig) -> Option<&'static str> {
    if host.session_stamp.is_none() {
        Some(ENV_STAMP)
    } else if host.pins.bhyve.is_none() {
        Some(ENV_PIN_BHYVE)
    } else if host.pins.bhyvectl.is_none() {
        Some(ENV_PIN_BHYVECTL)
    } else if host.pins.lib_manifest.is_none() {
        Some(ENV_PIN_LIB)
    } else {
        None
    }
}

/// The machine check of the backend, read from this machine.
fn machine_check() -> hairspring_engine::probe::ProbeReport {
    hairspring_dhyve::assess(&hairspring_dhyve::gather())
}

fn refuse(text: &str) -> ExitCode {
    eprintln!("{text}\n{USAGE}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let all: Vec<String> = std::env::args().skip(1).collect();
    if all.iter().any(|a| a == "--build-info") {
        println!("{}", json!({"fake_vmm": FAKE_BUILT}));
        return ExitCode::SUCCESS;
    }
    let allow_fake = all.iter().any(|a| a == "--allow-fake");
    let rdrand_accepted = all.iter().any(|a| a == "--rdrand-accepted");
    let args: Vec<String> = all
        .into_iter()
        .filter(|a| a != "--allow-fake" && a != "--rdrand-accepted")
        .collect();
    if FAKE_BUILT && !allow_fake {
        return refuse(
            "this build has the stand-in programs built in; pass --allow-fake to run it",
        );
    }
    let mut plan = match plan(&args) {
        Ok(p) => p,
        Err(e) => return refuse(&e),
    };
    plan.rdrand_accepted = rdrand_accepted;
    if !FAKE_BUILT {
        plan.probe = Some(machine_check);
    }
    let host = match HostConfig::from_env() {
        Ok(h) => h,
        Err(e) => return refuse(&e.to_string()),
    };
    if let Some(name) = missing_for_a_session(&host) {
        return refuse(&format!("{name} is required for a session"));
    }
    let session = Session::new(host, Timings::standard(), Bounds::interim())
        .with_one_prepare()
        .with_keep_all();
    let summary = run_session(&session, &plan);
    let mut json: Value = serde_json::from_str(&summary.to_json()).unwrap_or(Value::Null);
    json["fake_vmm"] = json!(FAKE_BUILT);
    println!("{json}");
    ExitCode::from(u8::from(!summary.clean()))
}
