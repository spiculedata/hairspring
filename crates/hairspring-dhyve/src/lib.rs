//! dhyve backend for Hairspring: experimental, off by default, and so far a machine check only.
//!
//! The backend runs nothing yet. `probe()` says whether this machine could run it, by reading facts (`gather`) and
//! judging them (`assess`). It never loads or unloads a kernel module and never starts a virtual machine. Every
//! capability is `false` and the determinism class is [`DETERMINISM_CLASS`].
mod assess;
#[cfg(test)]
mod assess_tests;
mod backend;
#[cfg(any(target_os = "freebsd", test))]
mod bounded;
mod facts;
mod gather;
#[cfg(any(target_os = "freebsd", test))]
mod read;
#[cfg(any(target_os = "freebsd", test))]
mod vmname;

pub use assess::assess;
pub use backend::{DhyveBackend, BACKEND_ID, DETERMINISM_CLASS};
pub use facts::{BinaryFact, CpuFact, DhyveFacts, ModuleFact};
pub use gather::gather;
