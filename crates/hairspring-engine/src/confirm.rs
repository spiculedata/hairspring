//! The confirmation pipeline: explicit, recorded checks on a failing seed.
//!
//! A single rerun that agrees with the original is weaker than what is needed to trust a finding. The pipeline runs
//! separate checks (same-seed reruns, a bundle replay, an optional fixed-version control, an optional strict
//! determinism verify), records each one with its command, counts and the environment it ran in, and derives where the
//! finding stands from the recorded results alone. Nothing here reads text from the program under test: every value a
//! check carries is a count, an enum, a hash or a bounded label.
mod request;
mod result;
mod state;

pub use request::{
    forbidden_logging, ConfirmRequest, DEFAULT_RERUNS, DEFAULT_WALL_SECS, MAX_CONTROL_NAME_CHARS,
    MAX_RERUNS, MAX_WALL_SECS,
};
pub use result::{
    CheckCounts, CheckKind, CheckResult, CheckStatus, MAX_ARTEFACTS, MAX_ARTEFACT_CHARS,
    MAX_COLUMN_BYTES, MAX_COMMAND_CHARS, MAX_DETAIL_ENTRIES, MAX_DETAIL_KEY_CHARS,
    MAX_DETAIL_VALUE_CHARS, MAX_ENVIRONMENT_ENTRIES, MAX_ENVIRONMENT_VALUE_CHARS,
};
pub use state::{
    environment_pinned, finding_status, standing, FindingStatus, Standing, ENVIRONMENT_CHANGED_KEY,
};
