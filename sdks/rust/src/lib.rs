//! Hairspring guest SDK for Rust: in-source property assertions that speak guest protocol v1.
//!
//! ```no_run
//! use hairspring_sdk::{always, reachable, sometimes, thread, work};
//!
//! let (nbackfill, mxframe, waited) = (3, 5, true);
//! always!(nbackfill <= mxframe, "wal.ckpt.nbackfill_le_mxframe", "nBackfill <= live mxFrame", nbackfill, mxframe);
//! sometimes!(waited, "wal.writer.waited", "a writer waited for the lock");
//! reachable!("wal.reset.reached", "the WAL was reset");
//! work(1); // operations done, reported as work.ops
//! let _ = thread("writer", 0); // stable thread key "writer-0"
//! ```
//!
//! A call site only updates counters in memory; the program reports once, at exit. See docs/rust-sdk.md for the
//! semantics, the catalog sections and their limits, and the environment variables.

mod catalog;
mod eval;
mod fmt;
mod identity;
mod lines;
mod macros;
mod output;
mod report;
mod runtime;
mod section;
mod site;
mod slots;
mod sys;

pub use slots::KeyError;

/// Not part of the API: what the macros expand to.
#[doc(hidden)]
pub mod __private {
    pub use crate::eval::{eval, hit};
    pub use crate::site::{check_id, check_msg, twin, twin_len, Kind, Site, TwinSource};
}

/// Start the SDK now (it otherwise starts before `main` or at the first call site) and make sure it is linked into
/// a program that makes no assertion of its own. Idempotent.
pub fn init() {
    runtime::ensure_started();
}

/// Give the calling thread the stable key `label-index`. Bytes of the label outside `[A-Za-z0-9._:/-]` become `_`.
/// An error leaves the thread with its claim-order key `t#<n>`, flagged `key_stable:false` in the report. Call it
/// before the thread does anything else.
pub fn thread(label: &str, index: u32) -> Result<(), KeyError> {
    slots::table().set_key(slots::current(), label, index)
}

/// Add `n` to the calling thread's operation count, reported as `work.ops` summed over threads.
pub fn work(n: u64) {
    eval::work(n);
}

/// Write the report now (reason `explicit`). Idempotent: the first report wins and the exit handler then does
/// nothing.
pub fn report() {
    runtime::ensure_started();
    report::report_now(lines::EndReason::Explicit, i64::from(runtime::status()));
}

/// Record `status`, report and exit. Use instead of `std::process::exit` when the report should carry the status.
/// `end.status` is advisory: the engine always uses the operating system's wait status.
pub fn exit(status: i32) -> ! {
    runtime::exit(status)
}
