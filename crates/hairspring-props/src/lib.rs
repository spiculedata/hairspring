//! Property layer for Hairspring: the guest event protocol v1 parser, verdicts and binomial statistics.
//!
//! Pure: no I/O, no clocks, no randomness. The parser reads untrusted guest output, so every limit is enforced and no
//! input can make it panic.

pub mod catalog;
mod decode;
pub mod digest;
pub mod event;
mod fold;
mod json;
pub mod limits;
pub mod line;
pub mod record;
pub mod run;
pub mod stats;
pub mod trial;
pub mod valid;

pub use catalog::{CatalogError, CatalogRow, CatalogUpdate, Recorded, RunCatalog};
pub use digest::{
    Decl, DigestEntry, DigestError, DigestFlags, TrialDigest, MAX_DIGEST_BYTES, MAX_DIGEST_DECLS,
    MAX_DIGEST_WITNESSES,
};
pub use event::*;
pub use fold::TrialEvents;
pub use line::{parse_line, IgnoreReason, LineOutcome, RejectReason};
pub use run::{
    FirstFailure, KindMismatch, PropertyStat, PropertyStatus, Rate, RateBasis, RunProps,
    RunSummary, UnratedReason,
};
pub use trial::{
    failure_basis, CatalogConflict, DroppedCounts, EndReport, EventHealth, FailureReason,
    ProcReport, PropertyFailure, TrialProperty, TrialProps, Witness,
};
