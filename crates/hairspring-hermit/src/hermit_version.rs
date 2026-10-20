//! Asks the Hermit executable for its version, without ever waiting on it for long.
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A real `hermit --version` answers at once; anything slower is not worth holding a run for.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(10);
const MAX_VERSION_BYTES: u64 = 512;

/// The first line `hermit --version` prints, or `None` when it fails, prints nothing, or does not answer in time.
pub fn probe(bin: &Path) -> Option<String> {
    let mut child = Command::new(bin)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut text = String::new();
    child
        .stdout
        .take()?
        .take(MAX_VERSION_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}
