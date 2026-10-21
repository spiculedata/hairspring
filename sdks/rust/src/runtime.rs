//! Process-wide state and startup: the identity, the events channel, the catalog index, the exit handler and the
//! `hello` and `catalog` lines. Startup runs once, from a constructor before `main` or, failing that, from the first
//! call site that runs (which is then not syscall-free).

use std::env;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicI32, AtomicU64};
use std::sync::OnceLock;

use crate::catalog::{self, Index};
use crate::identity::{self, Identity};
use crate::lines::{EndReason, ProcInfo};
use crate::site::Site;
use crate::{output, report, section, sys};

/// What the process is, fixed at startup.
pub struct Runtime {
    pub sites: &'static [Site],
    pub index: Index,
    pub identity: Identity,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
/// Losses inside the SDK: threads without a slot of their own, refused settings, lines that did not fit or could not
/// be written. Reported in `end.dropped`.
static DROPPED: AtomicU64 = AtomicU64::new(0);
/// Recorded by `exit`; advisory, the engine uses the operating system's wait status.
static STATUS: AtomicI32 = AtomicI32::new(0);

pub fn note_dropped(n: u64) {
    DROPPED.fetch_add(n, Relaxed);
}

pub fn dropped() -> u64 {
    DROPPED.load(Relaxed)
}

pub fn status() -> i32 {
    STATUS.load(Relaxed)
}

/// Start the SDK if nothing has yet. Idempotent and safe to call from any thread.
pub fn ensure_started() -> &'static Runtime {
    RUNTIME.get_or_init(start)
}

/// The values the lines say about the process, with the drop count as it stands now.
pub fn proc_info(rt: &Runtime) -> ProcInfo {
    ProcInfo {
        tag: rt.identity.tag.clone(),
        tag_stable: rt.identity.tag_stable,
        // Signal-report handlers are not implemented in the Rust SDK; see docs/rust-sdk.md.
        signal_handlers: false,
        catalog_hash: rt.index.hash,
        n_ids: rt.index.n_ids,
        dropped: dropped(),
    }
}

fn export_identity(identity: &Identity, announced: bool) {
    env::set_var(identity::ENV_TAG, &identity.tag);
    env::set_var(
        identity::ENV_PID,
        identity::owner_field(std::process::id(), announced, identity.tag_stable),
    );
}

fn start() -> Runtime {
    let inherited = env::var(identity::ENV_TAG).ok();
    let owner = env::var(identity::ENV_PID).ok();
    let identity = identity::resolve(inherited.as_deref(), owner.as_deref(), std::process::id());
    note_dropped(u64::from(identity.refused));
    note_dropped(output::open_from_env());
    let sites = section::sites();
    let index = catalog::build(sites);
    note_dropped(index.excess);
    export_identity(&identity, false);
    let rt = Runtime {
        sites,
        index,
        identity,
    };
    register_exit_handler();
    report::emit_startup(&rt);
    export_identity(&rt.identity, true);
    rt
}

/// glibc hands exit handlers the real status, so a plain `return` from `main` or `process::exit(n)` is reported as n.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn register_exit_handler() {
    extern "C" fn at_exit(status: std::os::raw::c_int, _arg: *mut std::os::raw::c_void) {
        report::report_now(EndReason::Exit, i64::from(status));
    }
    // SAFETY: registers a plain function; a failed registration only means no report at exit.
    unsafe {
        sys::on_exit(at_exit, std::ptr::null_mut());
    }
}

/// Elsewhere an exit handler cannot see the status: `end.status` is 0 unless `exit` recorded one. It is advisory
/// either way.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn register_exit_handler() {
    extern "C" fn at_exit() {
        report::report_now(EndReason::Exit, i64::from(status()));
    }
    // SAFETY: as above.
    unsafe {
        sys::atexit(at_exit);
    }
}

/// Record `status`, then exit through the normal path so the exit handler writes the report.
pub fn exit(status: i32) -> ! {
    STATUS.store(status, Relaxed);
    std::process::exit(status)
}

/// A constructor: the catalog goes out before `main` runs, so a program that crashes or hangs still has one. Where a
/// constructor cannot be registered the first call site starts the SDK instead.
#[cfg(not(test))]
#[used]
#[cfg_attr(
    any(target_os = "linux", target_os = "android"),
    link_section = ".init_array"
)]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
static CONSTRUCTOR: extern "C" fn() = {
    extern "C" fn construct() {
        ensure_started();
    }
    construct
};
