//! dhyve backend for Hairspring: experimental, off by default, and not covered by the support matrix.
//!
//! `probe()` says whether this machine could run it, by reading facts (`gather`) and judging them (`assess`). The run
//! path is the smallest test shape: one prepare (a cold boot to a stop, a checkpoint, a shutdown) and then one restore
//! for each trial. It never loads or unloads a kernel module, and it only ever starts the pinned VM programs, through
//! the argument lists in [`commands`]. Every capability is `false` and the determinism class is [`DETERMINISM_CLASS`].
//!
//! The modules below the backend are public so that the session driver and the tests against the fake can use them;
//! they are not a stable interface.
mod assess;
#[cfg(test)]
mod assess_tests;
mod backend;
mod bounded;
pub mod checkpoint;
pub mod ckptmeta;
pub mod commands;
pub mod director;
pub mod driver;
pub mod error;
mod facts;
pub mod fwcfg;
mod gather;
pub mod host;
pub mod hostlock;
pub mod judge;
pub mod layout;
pub mod logs;
pub mod pins;
pub mod prepare;
#[cfg(any(target_os = "freebsd", test))]
mod read;
mod run;
pub mod runlog;
mod safe_open;
pub mod serial;
pub mod session;
pub mod signal;
mod spawn;
mod startcheck;
pub mod trial;
pub mod vmargs;
pub mod vmname;
pub mod wire;

pub use assess::assess;
pub use backend::{DhyveBackend, BACKEND_ID, DETERMINISM_CLASS};
pub use facts::{BinaryFact, CpuFact, DhyveFacts, ModuleFact};
pub use gather::gather;
