//! Points: addresses and content hashes for places inside a trial.
//!
//! A point is `(run, trial, cut)`. This module is pure: it parses and formats the address
//! (`p1~<run_id>~<seq>~<cut>`), builds the canonical identity text of the inputs that can change
//! execution, and derives the point hash and the command hash. It does no I/O.
//!
//! - [`address`]: the strict address parser and formatter.
//! - [`encoding`]: the shared text escaping and list encoding.
//! - [`identity`]: the per-backend identity inputs and the canonical identity text.

pub mod address;
pub mod encoding;
pub mod identity;
mod identity_text;

pub use address::{AddressError, BackendKind, Cut, PointAddress};
pub use encoding::Recorded;
pub use identity::{
    CommonInputs, HermitInputs, Identity, IdentityDraft, IdentityError, SimInputs, TmpPolicy,
    WasmInputs,
};
