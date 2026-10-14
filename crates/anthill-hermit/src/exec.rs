//! Running one trial: spawn hermit, capture bounded output, enforce a timeout.
use crate::{env::guest_env, HermitSettings};
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
}

#[derive(Debug, Clone)]
pub struct Trial {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[derive(Debug)]
pub enum SpawnError {
    NotFound(String),
    Other(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::NotFound(p) => write!(f, "hermit executable not found at `{p}`. Hermit runs only on x86-64 Linux: install it there, or set ANTHILL_HERMIT to its path"),
            SpawnError::Other(m) => write!(f, "cannot start hermit: {m}"),
        }
    }
}

/// Arguments for `hermit`: `run <flags> -- <command...>`.
pub fn hermit_args(settings: &HermitSettings, spec: &ProcessSpec, seed: u64) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        format!("--epoch={}", settings.epoch),
        "--sched-heuristic=random".to_string(),
        format!("--sched-seed={seed}"),
    ];
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

fn kill_group(pid: u32) {
    // Negative pid addresses the whole process group, so grandchildren die with the guest.
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
}

pub fn run_trial(
    settings: &HermitSettings,
    spec: &ProcessSpec,
    seed: u64,
    timeout: Duration,
) -> Result<Trial, SpawnError> {
    let mut cmd = Command::new(&settings.bin);
    cmd.args(hermit_args(settings, spec, seed))
        .env_clear()
        .envs(guest_env(spec.command, spec.env))
        .envs(settings.extra_env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(dir) = spec.cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
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
            Ok(Some(status)) => break (Some(status), false),
            Ok(None) if Instant::now() >= deadline => {
                kill_group(child.id());
                let _ = child.kill();
                break (child.wait().ok(), true);
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(e) => return Err(SpawnError::Other(e.to_string())),
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
    })
}
