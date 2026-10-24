//! Where a finding stands, derived from its recorded checks and nothing else.
//!
//! The ladder is `candidate -> rerun_repeated -> confirmed -> verified`, with two ends that are not rungs:
//! `not_reproducible` (the checks that should repeat it did not) and `control_failed` (the fixed build still fails at
//! the seed, so the failure is not what was blamed). Only `confirmed` and `verified` findings reach the claim wording.
use super::{CheckKind, CheckResult, CheckStatus};
use hairspring_props::claim_text::FindingState;
use serde::Serialize;
use std::collections::BTreeMap;

/// The detail key of a bundle replay that says whether the host differed from the recording.
pub const ENVIRONMENT_CHANGED_KEY: &str = "environment_changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    /// No check has repeated the failure.
    Candidate,
    /// Same-seed reruns all repeated it, and nothing more has: not yet a replay of the bundle.
    RerunRepeated,
    /// Reruns repeated it and a replay of the bundle did too.
    Confirmed,
    /// Confirmed, and Hermit's strict determinism verify agreed.
    Verified,
    NotReproducible,
    ControlFailed,
}

impl FindingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FindingStatus::Candidate => "candidate",
            FindingStatus::RerunRepeated => "rerun_repeated",
            FindingStatus::Confirmed => "confirmed",
            FindingStatus::Verified => "verified",
            FindingStatus::NotReproducible => "not_reproducible",
            FindingStatus::ControlFailed => "control_failed",
        }
    }

    /// The rung the claim wording sees: only a confirmed or verified finding is one it may call reproduced. `rerun_repeated` is not.
    pub fn claim_state(self) -> FindingState {
        match self {
            FindingStatus::Confirmed => FindingState::Confirmed,
            FindingStatus::Verified => FindingState::Verified,
            _ => FindingState::SingleRun,
        }
    }
}

/// The newest result of each kind. `checks` are in the order they were recorded, so a later one replaces an earlier one.
fn latest(checks: &[CheckResult]) -> BTreeMap<&'static str, &CheckResult> {
    checks.iter().map(|c| (c.kind.as_str(), c)).collect()
}

fn status_of(
    latest: &BTreeMap<&'static str, &CheckResult>,
    kind: CheckKind,
) -> Option<CheckStatus> {
    latest.get(kind.as_str()).map(|c| c.status)
}

/// The finding's state from its checks, oldest first.
pub fn finding_status(checks: &[CheckResult]) -> FindingStatus {
    let latest = latest(checks);
    let rerun = status_of(&latest, CheckKind::SameSeedRerun);
    let replay = status_of(&latest, CheckKind::BundleReplay);
    let control = status_of(&latest, CheckKind::FixedControl);
    let strict = status_of(&latest, CheckKind::StrictVerify);
    let pass = Some(CheckStatus::Pass);
    if rerun == Some(CheckStatus::Fail) || replay == Some(CheckStatus::Fail) {
        return FindingStatus::NotReproducible;
    }
    if rerun != pass {
        return FindingStatus::Candidate;
    }
    if control == Some(CheckStatus::Fail) {
        return FindingStatus::ControlFailed;
    }
    match (replay == pass, strict == pass) {
        (true, true) => FindingStatus::Verified,
        (true, false) => FindingStatus::Confirmed,
        (false, _) => FindingStatus::RerunRepeated,
    }
}

/// Whether the checks ran in an environment the bundle's recorded identity pins: the checks recorded one, the replay
/// that compared it saw no difference, and the reruns passed in that same environment (they are checks of the same
/// confirmation, so they share the identity the replay compared). A finding checked without a recorded identity, or on
/// a host that differs, is not environment-pinned and its wording says so.
pub fn environment_pinned(checks: &[CheckResult]) -> bool {
    let latest = latest(checks);
    let (Some(replay), Some(rerun)) = (
        latest.get(CheckKind::BundleReplay.as_str()),
        latest.get(CheckKind::SameSeedRerun.as_str()),
    ) else {
        return false;
    };
    let unchanged = |c: &CheckResult, key: &str| c.detail.get(key).is_none_or(|v| v != "true");
    replay.status == CheckStatus::Pass
        && replay
            .detail
            .get(ENVIRONMENT_CHANGED_KEY)
            .is_some_and(|v| v == "false")
        && !replay.environment.is_empty()
        && rerun.status == CheckStatus::Pass
        && unchanged(rerun, ENVIRONMENT_CHANGED_KEY)
        && rerun.environment == replay.environment
}

/// Where a finding stands after all its confirmations, and which one says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub status: FindingStatus,
    pub pinned: bool,
    /// The confirmation the state was derived from, as an index into the slice given; `None` when there is none.
    pub confirmation: Option<usize>,
}

/// A confirmation that ran every check it set out to: nothing was skipped (cancelled, or out of time).
fn finished(checks: &[CheckResult]) -> bool {
    checks.iter().all(|c| c.status != CheckStatus::Skipped)
}

/// The finding's state from its confirmations, oldest first. The state is derived from ONE confirmation, never from a
/// mix of checks taken at different times: the newest that finished, or, when none did, the newest, with its skipped
/// checks counting for nothing. A confirmation that was cancelled therefore cannot revert an earlier finished one.
pub fn standing(confirmations: &[Vec<CheckResult>]) -> Standing {
    let chosen = confirmations
        .iter()
        .rposition(|c| finished(c))
        .or(confirmations.len().checked_sub(1));
    match chosen {
        Some(i) => Standing {
            status: finding_status(&confirmations[i]),
            pinned: environment_pinned(&confirmations[i]),
            confirmation: Some(i),
        },
        None => Standing {
            status: FindingStatus::Candidate,
            pinned: false,
            confirmation: None,
        },
    }
}
