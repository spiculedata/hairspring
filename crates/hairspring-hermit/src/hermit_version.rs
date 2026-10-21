//! Asks the Hermit executable for its version, without ever waiting on it for long.
//!
//! The child writes to a private temporary file instead of a pipe, so no thread ever blocks reading from it: when the
//! deadline passes the child is killed, and anything it left holding the file open cannot hold up the run.
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A real `hermit --version` answers at once; anything slower is not worth holding a run for.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(10);
const MAX_VERSION_BYTES: u64 = 512;
const OWNER_ONLY: u32 = 0o600;
/// Enough for a wrapper script to find its tools; the probe does not inherit the caller's environment.
const PROBE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

static PROBES: AtomicU64 = AtomicU64::new(0);

/// A new file in the temporary directory that only this user can read; removed when dropped.
struct Output {
    path: PathBuf,
}

impl Output {
    fn create() -> std::io::Result<(Output, std::fs::File)> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let n = PROBES.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "hairspring-version-{}-{n}-{nanos}",
            std::process::id()
        ));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(OWNER_ONLY)
            .open(&path)?;
        Ok((Output { path }, file))
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The first line `hermit --version` prints, or `None` when it fails, prints nothing, or does not finish in time.
pub fn probe(bin: &Path) -> Option<String> {
    let (output, file) = Output::create().ok()?;
    let mut child = Command::new(bin)
        .arg("--version")
        .env_clear()
        .env("PATH", PROBE_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
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
    let mut text = String::new();
    std::fs::File::open(&output.path)
        .ok()?
        .take(MAX_VERSION_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    let line = text.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}
