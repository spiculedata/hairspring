//! The stored results of confirming a failure (schema version 5).
//!
//! A confirmation is one run of the confirmation pipeline on a failure: several checks, stored together or not at all.
//! Each check keeps its kind, outcome, counts, the exact command, bounded JSON for its detail, the hashes it saw, the
//! environment it ran in and when. The store does not interpret any of it; the service validates what a runner reports
//! before it gets here, and the store only enforces the sizes so a caller cannot make one failure grow without limit.
use crate::types::{Result, StoreError};
use serde::{Deserialize, Serialize};

/// Most checks one confirmation may hold (the pipeline has four kinds).
pub const MAX_CHECKS_PER_CONFIRMATION: usize = 8;
/// Most checks one failure may hold across all its confirmations.
pub const MAX_CHECKS_PER_FAILURE: u64 = 200;
/// Longest text kept in any one column of a check, in bytes.
pub const MAX_CHECK_TEXT_BYTES: usize = 16 * 1024;
/// Longest kind, status, runner or id, in bytes.
pub const MAX_CHECK_TAG_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCheck {
    pub kind: String,
    pub status: String,
    pub passed: u32,
    pub failed: u32,
    pub inconclusive: u32,
    pub command: String,
    pub detail_json: String,
    pub artefacts_json: String,
    pub environment_json: String,
    pub started_at: i64,
    pub finished_at: i64,
}

/// One confirmation: `id` names it, so storing it again (a retry) changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewConfirmation {
    pub id: String,
    pub failure_id: String,
    /// Who ran the checks: the worker or process, as an opaque label.
    pub runner: String,
    /// Whether the runner was trusted when it reported, a snapshot as for trial rows.
    pub runner_trusted: bool,
    pub checks: Vec<NewCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRecord {
    pub id: String,
    pub failure_id: String,
    pub confirmation_id: String,
    /// The check's place in its confirmation, from zero.
    pub ordinal: u32,
    pub kind: String,
    pub status: String,
    pub passed: u32,
    pub failed: u32,
    pub inconclusive: u32,
    pub command: String,
    pub detail_json: String,
    pub artefacts_json: String,
    pub environment_json: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub runner: String,
    pub runner_trusted: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationOutcome {
    Stored,
    /// The same confirmation was already stored; nothing changed.
    Duplicate,
}

impl CheckRecord {
    /// Whether this stored check is what `new` asks to store, apart from the time it was stored.
    pub fn same_as(&self, new: &NewCheck, ordinal: u32) -> bool {
        self.ordinal == ordinal
            && self.kind == new.kind
            && self.status == new.status
            && (self.passed, self.failed, self.inconclusive)
                == (new.passed, new.failed, new.inconclusive)
            && self.command == new.command
            && self.detail_json == new.detail_json
            && self.artefacts_json == new.artefacts_json
            && self.environment_json == new.environment_json
            && (self.started_at, self.finished_at) == (new.started_at, new.finished_at)
    }
}

fn invalid(what: &str) -> StoreError {
    StoreError::Backend(format!("invalid confirmation: {what}"))
}

fn tag(value: &str, what: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_CHECK_TAG_BYTES {
        return Err(invalid(&format!(
            "{what} must be 1-{MAX_CHECK_TAG_BYTES} bytes"
        )));
    }
    Ok(())
}

fn text(value: &str, what: &'static str) -> Result<()> {
    if value.len() > MAX_CHECK_TEXT_BYTES {
        return Err(StoreError::LimitExceeded {
            what,
            limit: MAX_CHECK_TEXT_BYTES as u64,
        });
    }
    Ok(())
}

/// The checks of a confirmation against the sizes the store keeps. Nothing is written for a confirmation that fails.
pub(crate) fn validate(new: &NewConfirmation) -> Result<()> {
    tag(&new.id, "the confirmation id")?;
    tag(&new.failure_id, "the failure id")?;
    tag(&new.runner, "the runner")?;
    if new.checks.is_empty() {
        return Err(invalid("it holds no checks"));
    }
    if new.checks.len() > MAX_CHECKS_PER_CONFIRMATION {
        return Err(StoreError::LimitExceeded {
            what: "checks in one confirmation",
            limit: MAX_CHECKS_PER_CONFIRMATION as u64,
        });
    }
    for check in &new.checks {
        tag(&check.kind, "a check's kind")?;
        tag(&check.status, "a check's status")?;
        text(&check.command, "a check's command")?;
        text(&check.detail_json, "a check's detail")?;
        text(&check.artefacts_json, "a check's artefacts")?;
        text(&check.environment_json, "a check's environment")?;
    }
    Ok(())
}

/// Whether adding `adding` checks to a failure that holds `held` would pass the per-failure cap.
pub(crate) fn over_cap(held: u64, adding: usize) -> bool {
    held.saturating_add(adding as u64) > MAX_CHECKS_PER_FAILURE
}
