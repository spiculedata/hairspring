//! Asks the Hermit executable for its version, without ever waiting on it for long.
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A real `hermit --version` answers at once; anything slower is not worth holding a run for.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(10);
const MAX_VERSION_BYTES: u64 = 512;
/// Enough for a wrapper script to find its tools; the probe does not inherit the caller's environment.
const PROBE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// The first line `hermit --version` prints, or `None` when it fails, prints nothing, or does not finish in time. The
/// whole probe, including reading its output, is bounded by one deadline, and the child is killed when it is missed,
/// so a process that holds the pipe open cannot block a run.
pub fn probe(bin: &Path) -> Option<String> {
    let mut child = Command::new(bin)
        .arg("--version")
        .env_clear()
        .env("PATH", PROBE_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let read = (&mut stdout)
            .take(MAX_VERSION_BYTES)
            .read_to_string(&mut text);
        let _ = tx.send(read.ok().map(|_| text));
    });
    let text = rx.recv_timeout(PROBE_TIMEOUT).ok().flatten();
    let succeeded = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => break false,
        }
    };
    if !succeeded {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let line = text?.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}
