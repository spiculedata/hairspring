//! Running one trial: spawn hermit, capture bounded output, enforce a timeout.
use crate::capture::{drain, Captured};
use crate::events::{self, EventsFault};
use crate::head_tail::Omitted;
use crate::interrupt;
use crate::line_scan::Findings;
use crate::summary::{self, Summary};
use crate::summary_dir::{PrivateDir, ScratchDir};
use crate::trial_tmp::hermit_temp_env;
use crate::{env::guest_env, HermitSettings};
use hairspring_engine::scheduler::{Heuristic, SchedulerSpec};
use hairspring_props::TrialProps;
use std::collections::BTreeMap;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The environment variable an SDK reads for its process tag, and the fixed tag the backend gives the root process.
const ROOT_PROC_ENV: &str = "HAIRSPRING_PROC";
const ROOT_PROC_TAG: &str = "p1";
/// Set by an SDK for the processes it forks; inheriting it would turn the root into `p1.s<pid>`.
const PROC_PID_ENV: &str = "HAIRSPRING_PROC_PID";
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

#[derive(Debug, Clone, Default)]
pub struct Trial {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// What the bounded capture left out of the middle of `stdout`, and of `stderr`. Zero bytes means kept whole.
    pub stdout_omitted: Omitted,
    pub stderr_omitted: Omitted,
    /// What the lines of each stream said, read as they arrived. `None` for a trial built from text alone, whose
    /// lines are read from the text.
    pub(crate) stdout_scan: Option<Findings>,
    pub(crate) stderr_scan: Option<Findings>,
    pub timed_out: bool,
    /// The schedule the trial ran, when Hermit's summary could be read.
    pub schedule: Option<Summary>,
    /// What the guest reported through the event protocol, from stdout, stderr and the events file. `None` when it
    /// sent nothing. Event lines are not part of `stdout`/`stderr`.
    pub events: Option<TrialProps>,
    /// A digest of the raw event lines, so that `behaviour_hash` sees property outcomes. `None` without events.
    pub events_digest: Option<u64>,
    /// Why the events file could not be used, when it was asked for and something went wrong.
    pub events_error: Option<EventsFault>,
    /// Wall-clock milliseconds from starting the guest to collecting its output. Informational: it differs between
    /// identical schedules and never enters a hash.
    pub elapsed_ms: u64,
}

impl Trial {
    /// The verdict-relevant lines of stdout, from the stream as read, or from the text when there is none.
    pub(crate) fn stdout_findings(&self) -> Findings {
        self.stdout_scan
            .clone()
            .unwrap_or_else(|| Findings::of_text(&self.stdout))
    }

    pub(crate) fn stderr_findings(&self) -> Findings {
        self.stderr_scan
            .clone()
            .unwrap_or_else(|| Findings::of_text(&self.stderr))
    }
}

/// The private directories a backend gives its trials.
#[derive(Clone, Copy, Default)]
pub struct Dirs<'a> {
    /// Hermit's `--summary-json` files.
    pub summaries: Option<&'a PrivateDir>,
    /// The per-trial directories mounted as the guest's `/tmp` for the events file.
    pub events: Option<&'a PrivateDir>,
    /// Where each trial's directory for Hermit's own temporary files goes. `None`: no such directory (a caller that
    /// has none). `Some(Err)`: it was wanted and could not be made, which fails the trial rather than running it with
    /// Hermit's temporary files somewhere the engine cannot clean.
    pub trials: Option<&'a Result<PrivateDir, String>>,
}

#[derive(Debug)]
pub enum SpawnError {
    NotFound(String),
    Other(String),
    /// The run was interrupted (Ctrl-C): the guest, if one had started, was killed with its group.
    Interrupted,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::NotFound(p) => write!(f, "hermit executable not found at `{p}`. Hermit runs only on x86-64 Linux: install it there, or set HAIRSPRING_HERMIT to its path"),
            SpawnError::Other(m) => write!(f, "cannot start hermit: {m}"),
            SpawnError::Interrupted => write!(f, "interrupted"),
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

/// Arguments for `hermit`: `run <flags> -- <command...>`. `extra` are flags a caller adds for one kind of run (the
/// strict verify's), placed before the target's own `hermit_args`.
pub fn hermit_args(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    summary: Option<&std::path::Path>,
    events: Option<&ScratchDir>,
    extra: &[String],
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
    args.extend(extra.iter().cloned());
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
pub(crate) fn resolve_bin(bin: &std::path::Path) -> std::path::PathBuf {
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
    base: Option<&PrivateDir>,
) -> (Option<ScratchDir>, Option<EventsFault>) {
    if let Some(problem) =
        events::config_problem(settings.events_file, spec.hermit_args, base.is_some())
    {
        return (None, Some(EventsFault::Config(problem)));
    }
    let Some(base) = base.filter(|_| settings.events_file) else {
        return (None, None);
    };
    match base.scratch_for() {
        Ok(dir) => (Some(dir), None),
        Err(e) => (
            None,
            Some(EventsFault::Infra(format!(
                "cannot create the events directory: {e}"
            ))),
        ),
    }
}

pub fn run_trial(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    timeout: Duration,
    dirs: Dirs,
) -> Result<Trial, SpawnError> {
    run_trial_with(settings, spec, seed, timeout, dirs, &[])
}

/// [`run_trial`] with `extra` Hermit flags added to the command line.
pub fn run_trial_with(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    timeout: Duration,
    dirs: Dirs,
    extra: &[String],
) -> Result<Trial, SpawnError> {
    let summary_path = dirs.summaries.map(|dir| dir.file_for(seed));
    let (events_dir, scratch_fault) = events_scratch(settings, spec, dirs.events);
    let temp_dir = match trial_dir(dirs.trials) {
        Ok(dir) => dir,
        Err(e) => {
            // Nothing has started; the summary path has not been written.
            let _ = events_dir.map(|d| d.remove());
            return Err(e);
        }
    };
    let trial = run_trial_with_summary(
        settings,
        spec,
        seed,
        timeout,
        summary_path.as_deref(),
        TrialDirs {
            events: events_dir.as_ref(),
            temp: temp_dir.as_ref(),
        },
        extra,
    );
    // Always remove the file, including when the trial failed to start or timed out.
    let schedule = summary_path.as_deref().and_then(|p| {
        let read = summary::read(p);
        let _ = std::fs::remove_file(p);
        read
    });
    // The events were read inside the trial; only now can the directory go, and a failure to remove it is reported.
    let cleanup_fault = events_dir
        .and_then(|dir| dir.remove().err())
        .map(|e| EventsFault::Infra(format!("cannot remove the events directory: {e}")));
    // The trial is over (or never started); Hermit's temporary files go whether it finished, failed or timed out. A
    // directory that cannot be removed leaves files on the host but says nothing about the guest, so it is warned
    // about on standard error and never changes the trial's result.
    if let Some(e) = temp_dir.and_then(|dir| dir.remove().err()) {
        eprintln!("hairspring: warning: cannot remove the trial's private directory: {e}");
    }
    trial.map(|t| Trial {
        schedule,
        events_error: scratch_fault.or(cleanup_fault).or(t.events_error),
        ..t
    })
}

/// This trial's directory for Hermit's own temporary files, made before the trial and removed after it.
fn trial_dir(
    trials: Option<&Result<PrivateDir, String>>,
) -> Result<Option<ScratchDir>, SpawnError> {
    match trials {
        None => Ok(None),
        Some(Err(why)) => Err(SpawnError::Other(why.clone())),
        Some(Ok(base)) => base.scratch_for().map(Some).map_err(|e| {
            SpawnError::Other(format!("cannot create the trial's private directory: {e}"))
        }),
    }
}

/// Reads the events file (when there is one) and folds it with the lines captured from the output.
fn collect_events(
    dir: Option<&ScratchDir>,
    budget: usize,
    out: &Captured,
    err: &Captured,
) -> (Option<TrialProps>, Option<u64>, Option<EventsFault>) {
    let (file, error) = match dir.map(|d| events::read_events_file(d.path(), budget)) {
        Some(Ok(Some(file))) if file.truncated => {
            let fault = EventsFault::Guest(
                "the events file is larger than the per-trial budget; the rest was not read"
                    .to_string(),
            );
            (Some(file), Some(fault))
        }
        Some(Ok(file)) => (file, None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let folded = events::fold_sources(file.as_ref(), &out.events, &err.events, budget);
    let digest = events::digest(file.as_ref(), &out.events, &err.events);
    (folded, digest, error)
}

/// The engine-owned directories of one trial.
struct TrialDirs<'a> {
    /// Mounted as the guest's `/tmp` when the events file is on.
    events: Option<&'a ScratchDir>,
    /// Hermit's own temporary files.
    temp: Option<&'a ScratchDir>,
}

fn run_trial_with_summary(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    timeout: Duration,
    summary: Option<&std::path::Path>,
    dirs: TrialDirs,
    extra: &[String],
) -> Result<Trial, SpawnError> {
    let (events_dir, temp_dir) = (dirs.events, dirs.temp);
    let guest = guest_env(spec.command, spec.env);
    // Hermit's own temporary files; see `trial_tmp` for which variables are safe in which base environment.
    let temp_env = temp_dir.map_or_else(Vec::new, |dir| {
        hermit_temp_env(spec.hermit_args, &guest, &settings.extra_env, dir)
    });
    let mut cmd = Command::new(resolve_bin(&settings.bin));
    cmd.args(hermit_args(
        settings, spec, seed, summary, events_dir, extra,
    ))
    .env_clear()
    .envs(guest)
    // The trial seed is an input like any other; tools such as hairspring-dapr-sim derive their faults from it.
    .env("HAIRSPRING_SEED", seed.to_string())
    .envs(settings.extra_env.iter().map(|(k, v)| (k, v)))
    .envs(temp_env)
    .envs(events_dir.map(|_| (events::EVENTS_FILE_ENV, events::GUEST_EVENTS_FILE)))
    // Last, so nothing configured or passed through can change it: the root process tag is the same bytes for
    // every seed and trial, and a stray pid variable (which would make the root `p1.s<pid>`) never reaches the guest.
    .env(ROOT_PROC_ENV, ROOT_PROC_TAG)
    .env_remove(PROC_PID_ENV)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .process_group(0);
    if let Some(dir) = spec.cwd {
        cmd.current_dir(dir);
    }
    if interrupt::requested() {
        return Err(SpawnError::Interrupted);
    }
    let started = Instant::now();
    let mut child = spawn_with_retry(&mut cmd).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => SpawnError::NotFound(settings.bin.display().to_string()),
        _ => SpawnError::Other(e.to_string()),
    })?;
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    if let Some(s) = child.stdout.take() {
        drain(
            s,
            settings.max_capture_bytes,
            settings.max_event_bytes,
            out_tx,
        );
    }
    if let Some(s) = child.stderr.take() {
        drain(
            s,
            settings.max_capture_bytes,
            settings.max_event_bytes,
            err_tx,
        );
    }
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // The guest is gone, but anything it left running (daemons, background jobs) is still in its group.
                kill_group(child.id());
                break (Some(status), false);
            }
            Ok(None) if interrupt::requested() => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(SpawnError::Interrupted);
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
    let out = out_rx.recv_timeout(READER_GRACE).ok();
    let err = err_rx.recv_timeout(READER_GRACE).ok();
    let incomplete = (out.is_none() || err.is_none()).then(|| {
        EventsFault::Guest(format!(
            "incomplete capture: an output stream was still open {}s after the guest ended (something it started kept it), so output and events may be missing",
            READER_GRACE.as_secs()
        ))
    });
    let (out, err) = (out.unwrap_or_default(), err.unwrap_or_default());
    let (events, events_digest, file_fault) =
        collect_events(events_dir, settings.max_event_bytes, &out, &err);
    let events_error = file_fault.or(incomplete);
    Ok(Trial {
        exit_code: status.and_then(|s| s.code()),
        signal: status.and_then(|s| s.signal()),
        stdout: out.text,
        stderr: err.text,
        stdout_omitted: out.omitted,
        stderr_omitted: err.omitted,
        stdout_scan: Some(out.findings),
        stderr_scan: Some(err.findings),
        timed_out,
        schedule: None,
        events,
        events_digest,
        events_error,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
