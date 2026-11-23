//! The host-wide lock. A VM name is one stamp and two run digits, so two processes that picked the same stamp would
//! destroy or attach to each other's VM. At most one VM started by this backend exists on a host: an exclusive lock on one
//! file in the state directory is held for the whole of a prepare or a trial, and stamps are allocated under it from a
//! counter file, so no stamp is used twice.
use crate::vmname::Stamp;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const LOCK_FILE: &str = "host.lock";
const COUNTER_FILE: &str = "stamp-counter";
const COUNTER_TEMP: &str = "stamp-counter.new";
/// A counter file longer than this is not one this backend wrote.
const MAX_COUNTER_BYTES: u64 = 32;

/// Why the lock or a stamp could not be had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// Another process held the lock for the whole wait.
    Busy,
    Io(String),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy => write!(f, "another run holds the host"),
            LockError::Io(m) => write!(f, "the host lock could not be used: {m}"),
        }
    }
}

impl std::error::Error for LockError {}

fn io(what: &str, path: &Path, e: &std::io::Error) -> LockError {
    LockError::Io(format!("{what} {}: {}", path.display(), e.kind()))
}

/// The lock, held until it is dropped.
#[derive(Debug)]
pub struct HostLock {
    _file: File,
    dir: PathBuf,
}

impl HostLock {
    /// Takes the lock, waiting up to `wait` and looking every `poll`.
    pub fn acquire(dir: &Path, wait: Duration, poll: Duration) -> Result<HostLock, LockError> {
        std::fs::create_dir_all(dir).map_err(|e| io("create", dir, &e))?;
        let path = dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| io("open", &path, &e))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(HostLock {
                        _file: file,
                        dir: dir.to_path_buf(),
                    })
                }
                Err(TryLockError::WouldBlock) if started.elapsed() < wait => thread::sleep(poll),
                Err(TryLockError::WouldBlock) => return Err(LockError::Busy),
                Err(TryLockError::Error(e)) => return Err(io("lock", &path, &e)),
            }
        }
    }

    /// The next stamp, from the counter file. Never the same twice, across processes and restarts.
    pub fn allocate_stamp(&self) -> Result<Stamp, LockError> {
        let path = self.dir.join(COUNTER_FILE);
        let last = match std::fs::metadata(&path) {
            Ok(m) if m.len() > MAX_COUNTER_BYTES => {
                return Err(LockError::Io(format!(
                    "{} is not a counter file",
                    path.display()
                )))
            }
            Ok(_) => std::fs::read_to_string(&path)
                .map_err(|e| io("read", &path, &e))?
                .trim()
                .parse::<u64>()
                .map_err(|_| LockError::Io(format!("{} is not a counter file", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(io("read", &path, &e)),
        };
        let next = last
            .checked_add(1)
            .ok_or_else(|| LockError::Io("the stamp counter is exhausted".to_string()))?;
        let temp = self.dir.join(COUNTER_TEMP);
        let mut file = File::create(&temp).map_err(|e| io("write", &temp, &e))?;
        file.write_all(next.to_string().as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| io("write", &temp, &e))?;
        std::fs::rename(&temp, &path).map_err(|e| io("write", &path, &e))?;
        Ok(Stamp::from_counter(next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_millis(60);
    const POLL: Duration = Duration::from_millis(5);

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hs-dhyve-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_second_holder_waits_then_is_told_another_run_holds_the_host() {
        let dir = scratch("busy");
        let first = HostLock::acquire(&dir, SHORT, POLL).unwrap();
        let started = Instant::now();
        let second = HostLock::acquire(&dir, SHORT, POLL).unwrap_err();
        assert_eq!(second, LockError::Busy);
        assert!(started.elapsed() >= SHORT);
        assert_eq!(second.to_string(), "another run holds the host");
        drop(first);
    }

    #[test]
    fn the_lock_is_free_again_once_the_holder_is_dropped() {
        let dir = scratch("release");
        drop(HostLock::acquire(&dir, SHORT, POLL).unwrap());
        assert!(HostLock::acquire(&dir, SHORT, POLL).is_ok());
    }

    #[test]
    fn a_waiter_gets_the_lock_when_the_holder_lets_go_inside_the_wait() {
        let dir = scratch("handover");
        let first = HostLock::acquire(&dir, SHORT, POLL).unwrap();
        let other = {
            let dir = dir.clone();
            thread::spawn(move || HostLock::acquire(&dir, Duration::from_secs(5), POLL).is_ok())
        };
        thread::sleep(Duration::from_millis(40));
        drop(first);
        assert!(other.join().unwrap());
    }

    #[test]
    fn stamps_are_never_repeated_across_holders() {
        let dir = scratch("stamps");
        let mut seen = Vec::new();
        for _ in 0..3 {
            let lock = HostLock::acquire(&dir, SHORT, POLL).unwrap();
            seen.push(lock.allocate_stamp().unwrap());
            seen.push(lock.allocate_stamp().unwrap());
        }
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), seen.len());
    }

    #[test]
    fn a_damaged_counter_file_is_an_error_not_a_reset() {
        let dir = scratch("damaged");
        let lock = HostLock::acquire(&dir, SHORT, POLL).unwrap();
        std::fs::write(dir.join(COUNTER_FILE), "not a number").unwrap();
        assert!(matches!(lock.allocate_stamp(), Err(LockError::Io(_))));
        std::fs::write(dir.join(COUNTER_FILE), "9".repeat(64)).unwrap();
        assert!(matches!(lock.allocate_stamp(), Err(LockError::Io(_))));
    }

    #[test]
    fn an_unusable_state_directory_is_an_error() {
        let dir = scratch("unusable");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file");
        std::fs::write(&file, b"x").unwrap();
        let err = HostLock::acquire(&file.join("inside"), SHORT, POLL).unwrap_err();
        assert!(matches!(err, LockError::Io(_)));
        assert!(err.to_string().contains("could not be used"));
    }
}
