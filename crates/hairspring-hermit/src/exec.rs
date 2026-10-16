//! Running one trial: spawn hermit, capture bounded output, enforce a timeout.
use crate::capture::{drain, Captured};
use crate::events;
use crate::summary::{self, Summary};
use crate::summary_dir::{PrivateDir, ScratchDir};
use crate::{env::guest_env, HermitSettings};
use hairspring_engine::scheduler::{Heuristic, SchedulerSpec};
use hairspring_props::TrialProps;
use std::collections::BTreeMap;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(10);
const READER_GRACE: Duration = Duration::from_secs(2);

/// What to run, taken from the target definition.
pub struct ProcessSpec<'a> {
    pub command: &'a [String],
    pub env: &'a BTreeMap<String, String>,
    pub cwd: Option<&'a str>,
    pub hermit_args: &'a [String],
    pub trial_timeout_secs: Option<u64>,
    pub scheduler: &'a SchedulerSpec,
    pub hang_is_violation: bool,
    pub parallelism: u32,
}

#[derive(Debug, Clone)]
pub struct Trial {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    /// The schedule the trial ran, when Hermit's summary could be read.
    pub schedule: Option<Summary>,
    /// What the guest reported through the event protocol, from stdout, stderr and the events file. `None` when it
    /// sent nothing. Event lines are not part of `stdout`/`stderr`.
    pub events: Option<TrialProps>,
    /// Why the events file could not be used, when it was asked for and something went wrong.
    pub events_error: Option<String>,
}

#[derive(Debug)]
pub enum SpawnError {
    NotFound(String),
    Other(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::NotFound(p) => write!(f, "hermit executable not found at `{p}`. Hermit runs only on x86-64 Linux: install it there, or set HAIRSPRING_HERMIT to its path"),
            SpawnError::Other(m) => write!(f, "cannot start hermit: {m}"),
        }
    }
}

/// Hermit flags that choose the scheduling strategy. Recorded in bundles, so replay can insist on the same strategy.
pub fn scheduler_flags(s: &SchedulerSpec) -> Vec<String> {
    let mut flags = Vec::new();
    let heuristic = s.effective_heuristic();
    if let Some(value) = heuristic.hermit_value() {
        flags.push(format!("--sched-heuristic={value}"));
    }
    if let (Heuristic::StickyRandom, Some(permille)) = (heuristic, s.sticky_permille) {
        flags.push(format!(
            "--sched-sticky-random-param={:.3}",
            f64::from(permille) / 1000.0
        ));
    }
    if s.chaos {
        flags.push("--chaos".to_string());
    }
    if let Some(ns) = s.max_timeslice_ns {
        flags.push(format!("--max-timeslice={ns}"));
    }
    if let Some(margin) = s.skid_margin {
        flags.push(format!("--skid-margin={margin}"));
        // An overshoot lands the preemption late and the schedule silently stops reproducing; make it loud.
        flags.push("--panic-on-rbc-overshoot".to_string());
    }
    flags
}

/// Arguments for `hermit`: `run <flags> -- <command...>`.
pub fn hermit_args(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    summary: Option<&std::path::Path>,
    events: Option<&ScratchDir>,
) -> Vec<String> {
    let mut args = vec!["run".to_string(), format!("--epoch={}", settings.epoch)];
    args.extend(scheduler_flags(spec.scheduler));
    args.push(format!("--sched-seed={seed}"));
    if let Some(path) = summary {
        args.push(format!("--summary-json={}", path.display()));
    }
    if let Some(dir) = events {
        args.push(events::tmp_arg(dir));
    }
    args.extend(spec.hermit_args.iter().cloned());
    args.push("--".to_string());
    args.extend(spec.command.iter().cloned());
    args
}

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

const SIGKILL: i32 = 9;

/// Kills every process in the guest's process group, so nothing it started outlives the trial.
fn kill_group(pid: u32) {
    // A negative pid addresses the whole group. This calls libc directly (already linked by std) so it works
    // without a `kill` executable on PATH and has no argument-parsing quirks between platforms.
    // SAFETY: `kill` has no memory-safety preconditions; an invalid pid just returns an error.
    unsafe {
        kill(-(pid as i32), SIGKILL);
    }
}

/// A bare program name is looked up on the *caller's* PATH. The guest gets a pinned PATH, and Rust would
/// otherwise search that one, missing e.g. `~/.cargo/bin/hermit`.
fn resolve_bin(bin: &std::path::Path) -> std::path::PathBuf {
    if bin.components().count() != 1 {
        return bin.to_path_buf();
    }
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(bin))
                .find(|candidate| candidate.is_file())
        })
        .unwrap_or_else(|| bin.to_path_buf())
}

/// `ETXTBSY` (os error 26): the executable was written moments ago and another thread's `fork` still holds
/// the write handle. It clears as soon as that child execs, so a short retry is the right response.
const TEXT_FILE_BUSY: i32 = 26;
const SPAWN_RETRIES: u32 = 10;
const SPAWN_RETRY_PAUSE: Duration = Duration::from_millis(25);

fn spawn_with_retry(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    let mut attempt = 0;
    loop {
        match cmd.spawn() {
            Err(e) if e.raw_os_error() == Some(TEXT_FILE_BUSY) && attempt < SPAWN_RETRIES => {
                attempt += 1;
                std::thread::sleep(SPAWN_RETRY_PAUSE);
            }
            other => return other,
        }
    }
}

/// Where the guest's events file lives for one trial, or why it cannot be offered.
fn events_scratch(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    scratch: Option<&PrivateDir>,
) -> (Option<ScratchDir>, Option<String>) {
    if !settings.events_file {
        return (None, None);
    }
    if events::claims_tmp(spec.hermit_args) {
        let why =
            "the events file needs hermit's `--tmp`, which this target's hermit_args already sets";
        return (None, Some(why.to_string()));
    }
    let Some(base) = scratch else {
        let why = "the events file needs a private scratch directory (`summary_dir`), and none is available";
        return (None, Some(why.to_string()));
    };
    match base.scratch_for(seed) {
        Ok(dir) => (Some(dir), None),
        Err(e) => (
            None,
            Some(format!("cannot create the events directory: {e}")),
        ),
    }
}

pub fn run_trial(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    timeout: Duration,
    summaries: Option<&PrivateDir>,
) -> Result<Trial, SpawnError> {
    let summary_path = summaries.map(|dir| dir.file_for(seed));
    let (events_dir, events_error) = events_scratch(settings, spec, seed, summaries);
    let trial = run_trial_with_summary(
        settings,
        spec,
        seed,
        timeout,
        summary_path.as_deref(),
        events_dir.as_ref(),
    );
    // Always remove the file, including when the trial failed to start or timed out.
    let schedule = summary_path.as_deref().and_then(|p| {
        let read = summary::read(p);
        let _ = std::fs::remove_file(p);
        read
    });
    trial.map(|t| Trial {
        schedule,
        events_error: t.events_error.or(events_error),
        ..t
    })
}

/// Reads the events file (when there is one) and folds it with the lines captured from the output.
fn collect_events(
    dir: Option<&ScratchDir>,
    out: &Captured,
    err: &Captured,
) -> (Option<TrialProps>, Option<String>) {
    let (file, error) = match dir.map(|d| events::read_events_file(d.path())) {
        Some(Ok(file)) => (file, None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let folded = events::fold_sources(file.as_ref(), &out.events, &err.events);
    (folded, error)
}

fn run_trial_with_summary(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    timeout: Duration,
    summary: Option<&std::path::Path>,
    events_dir: Option<&ScratchDir>,
) -> Result<Trial, SpawnError> {
    let mut cmd = Command::new(resolve_bin(&settings.bin));
    cmd.args(hermit_args(settings, spec, seed, summary, events_dir))
        .env_clear()
        .envs(guest_env(spec.command, spec.env))
        // The trial seed is an input like any other; tools such as hairspring-dapr-sim derive their faults from it.
        .env("HAIRSPRING_SEED", seed.to_string())
        .envs(settings.extra_env.iter().map(|(k, v)| (k, v)))
        .envs(events_dir.map(|_| (events::EVENTS_FILE_ENV, events::GUEST_EVENTS_FILE)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(dir) = spec.cwd {
        cmd.current_dir(dir);
    }
    let mut child = spawn_with_retry(&mut cmd).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => SpawnError::NotFound(settings.bin.display().to_string()),
        _ => SpawnError::Other(e.to_string()),
    })?;
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    if let Some(s) = child.stdout.take() {
        drain(s, settings.max_capture_bytes, out_tx);
    }
    if let Some(s) = child.stderr.take() {
        drain(s, settings.max_capture_bytes, err_tx);
    }
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // The guest is gone, but anything it left running (daemons, background jobs) is still in its group.
                kill_group(child.id());
                break (Some(status), false);
            }
            Ok(None) if Instant::now() >= deadline => {
                kill_group(child.id());
                let _ = child.kill();
                break (child.wait().ok(), true);
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(e) => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(SpawnError::Other(e.to_string()));
            }
        }
    };
    // After a kill a surviving grandchild could hold the pipes open; do not wait for it forever.
    let out = out_rx.recv_timeout(READER_GRACE).unwrap_or_default();
    let err = err_rx.recv_timeout(READER_GRACE).unwrap_or_default();
    let (events, events_error) = collect_events(events_dir, &out, &err);
    Ok(Trial {
        exit_code: status.and_then(|s| s.code()),
        signal: status.and_then(|s| s.signal()),
        stdout: out.text,
        stderr: err.text,
        timed_out,
        schedule: None,
        events,
        events_error,
    })
}
