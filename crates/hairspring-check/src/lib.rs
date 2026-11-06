//! Pure history checkers: a checker is a function from a recorded operation history (and a final state) to a
//! [`Verdict`]: `Ok`, `Violation` with a witness, or `Unknown`. `Unknown` (budget exhausted, history incomplete, shape
//! not modelled) is inconclusive and is never a pass.
//!
//! | Checker | Final state | Cost |
//! |---|---|---|
//! | [`check_set`] | the set's contents | linear in the operations on the key |
//! | [`check_queue`] | what is left in the queue | linear (a sorted count of values) |
//! | [`check_wal_prefix`] | the recovered log | linear |
//! | [`check_register_linearizable`] | none | exponential in the worst case: bounded by [`Limits`] |
//!
//! Single-register linearizability is a search per key (operations on different keys cannot constrain each other),
//! with a memo of failed states, a step budget and a memo budget; a violation is shrunk to a 1-minimal witness under a
//! separate budget. Everything is deterministic: the same history and limits give the same verdict and witness.
//!
//! The history type ([`History`]) is built from [`OpRecord`]s, one per `op` line; the wire parser is not part of this
//! crate. Internal building block: nothing here is a public claim about distributed testing.

mod common;
pub mod history;
pub mod limits;
mod queue;
mod register;
mod set;
pub mod verdict;
mod wal;

pub use history::{Completeness, History, HistoryError, OpRecord, OpType, Operation, Outcome};
pub use limits::Limits;
pub use queue::check_queue;
pub use register::{check_register_key, check_register_linearizable};
pub use set::check_set;
pub use verdict::{CheckerId, UnknownReason, Verdict, Violation, ViolationKind, CHECKER_VERSION};
pub use wal::check_wal_prefix;
