//! Running one trial: spawn hermit, capture bounded output, enforce a timeout.
use crate::summary::{self, Summary};
use crate::{env::guest_env, HermitSettings};
use hairspring_engine::scheduler::{Heuristic, SchedulerSpec};
use std::collections::BTreeMap;
use std::io::Read;
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
    }
    flags
}

/// Arguments for `hermit`: `run <flags> -- <command...>`.
pub fn hermit_args(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    summary: Option<&std::path::Path>,
) -> Vec<String> {
    let mut args = vec!["run".to_string(), format!("--epoch={}", settings.epoch)];
    args.extend(scheduler_flags(spec.scheduler));
    args.push(format!("--sched-seed={seed}"));
    if let Some(path) = summary {
        args.push(format!("--summary-json={}", path.display()));
    }
    args.extend(spec.hermit_args.iter().cloned());
    args.push("--".to_string());
    args.extend(spec.command.iter().cloned());
    args
}

/// Reads a stream fully, keeping at most `cap` bytes, and sends what it kept when the stream ends.
fn drain(mut stream: impl Read + Send + 'static, cap: usize, tx: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&kept).into_owned());
    });
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

pub fn run_trial(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    timeout: Duration,
) -> Result<Trial, SpawnError> {
    let summary_path = settings
        .summary_dir
        .as_ref()
        .map(|dir| summary_file(dir, seed));
    let trial = run_trial_with_summary(settings, spec, seed, timeout, summary_path.as_deref());
    // Always remove the file, including when the trial failed to start or timed out.
    let schedule = summary_path.as_deref().and_then(|p| {
        let read = summary::read(p);
        let _ = std::fs::remove_file(p);
        read
    });
    trial.map(|t| Trial { schedule, ..t })
}

/// A path no other trial in this process (or another process) is using.
fn summary_file(dir: &std::path::Path, seed: u64) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!("hairspring-{}-{seed}-{n}.json", std::process::id()))
}

fn run_trial_with_summary(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    timeout: Duration,
    summary: Option<&std::path::Path>,
) -> Result<Trial, SpawnError> {
    let mut cmd = Command::new(resolve_bin(&settings.bin));
    cmd.args(hermit_args(settings, spec, seed, summary))
        .env_clear()
        .envs(guest_env(spec.command, spec.env))
        // The trial seed is an input like any other; tools such as hairspring-dapr-sim derive their faults from it.
        .env("HAIRSPRING_SEED", seed.to_string())
        .envs(settings.extra_env.iter().map(|(k, v)| (k, v)))
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
    let stdout = out_rx.recv_timeout(READER_GRACE).unwrap_or_default();
    let stderr = err_rx.recv_timeout(READER_GRACE).unwrap_or_default();
    Ok(Trial {
        exit_code: status.and_then(|s| s.code()),
        signal: status.and_then(|s| s.signal()),
        stdout,
        stderr,
        timed_out,
        schedule: None,
    })
}
