//! A process-wide interrupt for the trials this backend runs, set by Ctrl-C in the CLI.
//!
//! Each trial's guest runs in a process group of its own, so the terminal's SIGINT (which goes to the CLI's group only)
//! never reaches it. Once the interrupt is requested, a trial that is running kills its group, as it does when it
//! times out, and returns [`SpawnError::Interrupted`](crate::exec::SpawnError); no further trial starts.
use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

const SIGINT: i32 = 2;
/// `SIG_DFL`: the default action, which for SIGINT ends the process.
const SIG_DFL: usize = 0;

/// Asks every running and future trial to stop. Only stores to an atomic, so a signal handler may call it.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// The flag itself, for callers that take one to watch (the service stops a run when it is set).
pub fn flag() -> &'static AtomicBool {
    &REQUESTED
}

/// Whether an interrupt has been requested and not cleared.
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Forgets a request, so trials run again.
pub fn clear() {
    REQUESTED.store(false, Ordering::SeqCst);
}

extern "C" fn on_sigint(_signum: i32) {
    request();
    // A second Ctrl-C ends the process the usual way, in case the first is taking too long.
    // SAFETY: `signal` is async-signal-safe and has no memory-safety preconditions.
    unsafe {
        signal(SIGINT, SIG_DFL);
    }
}

/// Makes Ctrl-C request the interrupt instead of ending the process, and clears any earlier request. The first Ctrl-C
/// is a request; a second one ends the process.
pub fn install_sigint_handler() {
    clear();
    // SAFETY: `on_sigint` only stores to an atomic and calls `signal`, both async-signal-safe.
    unsafe {
        signal(SIGINT, on_sigint as extern "C" fn(i32) as usize);
    }
}
