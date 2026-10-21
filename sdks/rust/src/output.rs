//! Where the lines go: a private copy of stderr (or the events file) at a high descriptor number, so a program that
//! closes or reuses descriptor 2 cannot redirect the report. One `write(2)` per line in the normal case.

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::mem::ManuallyDrop;
use std::os::fd::{FromRawFd, IntoRawFd, RawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64};

use crate::sys;

/// The private events descriptor lives well above the ones a program numbers itself.
const HIGH_FD_FIRST: i32 = 200;
/// Used when `RLIMIT_NOFILE` is too low for the first.
const HIGH_FD_SECOND: i32 = 20;
const STDERR_FD: RawFd = 2;
/// `EINTR` and `EAGAIN` retries per line.
const RETRY_LIMIT: u32 = 1000;
const FILE_MODE: u32 = 0o644;

static FD: AtomicI32 = AtomicI32::new(-1);
// Which file the descriptor referred to at startup; set before the descriptor is published.
static DEV: AtomicU64 = AtomicU64::new(0);
static INO: AtomicU64 = AtomicU64::new(0);

/// A line could not be written (a closed reader, a full non-blocking pipe).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteFailed;

fn borrow(fd: RawFd) -> ManuallyDrop<File> {
    // SAFETY: the descriptor is only read through this handle, which is never dropped, so it is never closed here.
    ManuallyDrop::new(unsafe { File::from_raw_fd(fd) })
}

/// A copy of `fd` at a high number, close-on-exec. `Ok(None)` if `fd` is not open; the flag says no high number was
/// available, so the original stays exposed to the program (the SDK counts that as dropped).
fn private_copy(fd: RawFd) -> (Option<RawFd>, bool) {
    // SAFETY: fcntl with an integer argument on a descriptor number has no memory effects.
    let copy = unsafe {
        let first = sys::fcntl(fd, sys::F_DUPFD_CLOEXEC, HIGH_FD_FIRST);
        if first >= 0 {
            first
        } else {
            sys::fcntl(fd, sys::F_DUPFD_CLOEXEC, HIGH_FD_SECOND)
        }
    };
    if copy >= 0 {
        return (Some(copy), false);
    }
    // SAFETY: as above.
    if unsafe { sys::fcntl(fd, sys::F_GETFD) } < 0 {
        return (None, false);
    }
    (Some(fd), true)
}

/// Record what `fd` refers to, then publish it (release), so a thread that sees the descriptor also sees its identity.
fn publish(fd: Option<RawFd>) {
    let Some(fd) = fd else { return };
    let Ok(meta) = borrow(fd).metadata() else {
        return;
    };
    DEV.store(meta.dev(), Relaxed);
    INO.store(meta.ino(), Relaxed);
    FD.store(fd, Release);
}

fn events_file_setting() -> Option<std::ffi::OsString> {
    if sys::is_setid() {
        return None;
    }
    env::var_os("HAIRSPRING_EVENTS_FILE").filter(|p| !p.is_empty())
}

/// Open the events file for appending. It must be a regular file: it is opened non-blocking, so a FIFO with no reader
/// fails at once instead of hanging the program before `main`, and a FIFO (or device) that does open is refused.
fn open_events_file(path: &std::ffi::OsStr) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(FILE_MODE)
        .custom_flags(sys::O_NOFOLLOW | sys::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "the events file is not a regular file",
        ));
    }
    Ok(file)
}

/// Open the events channel from the environment. Returns how many losses to count as dropped (an events file that
/// could not be opened, no room for a private copy).
pub fn open_from_env() -> u64 {
    if env::var("HAIRSPRING_EVENTS").as_deref() == Ok("off") {
        return 0;
    }
    let mut dropped = 0;
    if let Some(path) = events_file_setting() {
        match open_events_file(&path) {
            Ok(file) => {
                let fd = file.into_raw_fd();
                let (copy, exposed) = private_copy(fd);
                dropped += u64::from(exposed);
                if let Some(high) = copy.filter(|&high| high != fd) {
                    drop(borrow_owned(fd));
                    publish(Some(high));
                } else {
                    publish(Some(fd));
                }
                return dropped;
            }
            // The file could not be opened: events fall back to stderr, and this is counted.
            Err(_) => dropped += 1,
        }
    }
    let (copy, exposed) = private_copy(STDERR_FD);
    publish(copy);
    dropped + u64::from(exposed)
}

fn borrow_owned(fd: RawFd) -> File {
    // SAFETY: the caller owns `fd` (it came from `into_raw_fd`) and gives that ownership up here.
    unsafe { File::from_raw_fd(fd) }
}

/// Has the descriptor been lost to the program (a `dup2` onto it)? Once it has, it is given up for good, so no later
/// line can follow it either. Returns true if it was lost by this call.
pub fn verify() -> bool {
    let fd = FD.load(Acquire);
    if fd < 0 {
        return false;
    }
    let same = borrow(fd)
        .metadata()
        .is_ok_and(|m| m.dev() == DEV.load(Relaxed) && m.ino() == INO.load(Relaxed));
    if same {
        return false;
    }
    FD.store(-1, Release);
    true
}

/// Write all of `bytes` (a line, or a plain diagnostic). A short write is completed; a line that still fails after
/// part of it went out is closed with a newline so the next line does not glue onto the fragment.
pub fn write_line(bytes: &[u8]) -> Result<(), WriteFailed> {
    let fd = FD.load(Acquire);
    if fd < 0 {
        return Ok(());
    }
    let file = borrow(fd);
    let mut out: &File = &file;
    let (mut done, mut retries) = (0, 0);
    while done < bytes.len() {
        match out.write(&bytes[done..]) {
            Ok(n) if n > 0 => done += n,
            Err(e) if retries < RETRY_LIMIT && e.kind() == ErrorKind::Interrupted => retries += 1,
            Err(e) if retries < RETRY_LIMIT && e.kind() == ErrorKind::WouldBlock => {
                retries += 1;
                std::thread::yield_now();
            }
            Err(e) => {
                SAW_EPIPE.fetch_or(e.raw_os_error() == Some(sys::EPIPE), Relaxed);
                if done > 0 {
                    let _ = out.write(b"\n");
                }
                return Err(WriteFailed);
            }
            Ok(_) => return Err(WriteFailed),
        }
    }
    Ok(())
}

/// Set when a write failed with `EPIPE` (the reader is gone).
static SAW_EPIPE: AtomicBool = AtomicBool::new(false);

/// Run `f` (a sequence of writes) with `SIGPIPE` blocked, so a reader that has gone away costs the report, not the
/// process. If a write raised a `SIGPIPE` and the caller did not already have it blocked, it is consumed before the
/// mask is restored, so it is not delivered on unblocking. This matters before `main`: Rust ignores `SIGPIPE` only
/// once its runtime has started, and the startup lines are written from a constructor, before that.
pub fn with_sigpipe_blocked<R>(f: impl FnOnce() -> R) -> R {
    let (mut pipe_only, mut old) = (sys::SigSet::zeroed(), sys::SigSet::zeroed());
    // SAFETY: the sets are valid, aligned and large enough for any platform's `sigset_t`.
    unsafe {
        sys::sigemptyset(&mut pipe_only);
        sys::sigaddset(&mut pipe_only, sys::SIGPIPE);
        sys::pthread_sigmask(sys::SIG_BLOCK, &pipe_only, &mut old);
    }
    SAW_EPIPE.store(false, Relaxed);
    let result = f();
    // SAFETY: as above; `sigwait` only runs when a `SIGPIPE` is pending for this thread, so it returns at once.
    unsafe {
        if SAW_EPIPE.load(Relaxed) && sys::sigismember(&old, sys::SIGPIPE) == 0 {
            let mut pending = sys::SigSet::zeroed();
            sys::sigpending(&mut pending);
            if sys::sigismember(&pending, sys::SIGPIPE) == 1 {
                let mut signal = 0;
                sys::sigwait(&pipe_only, &mut signal);
            }
        }
        sys::pthread_sigmask(sys::SIG_SETMASK, &old, std::ptr::null_mut());
    }
    result
}
