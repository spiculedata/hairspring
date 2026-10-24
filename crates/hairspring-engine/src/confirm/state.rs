//! Where a finding stands, derived from its recorded checks and nothing else.
//!
//! The ladder is `candidate -> reproduced -> confirmed -> verified`, with two ends that are not rungs:
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
    /// Same-seed reruns all repeated it.
    Reproduced,
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
            FindingStatus::Reproduced => "reproduced",
            FindingStatus::Confirmed => "confirmed",
            FindingStatus::Verified => "verified",
            FindingStatus::NotReproducible => "not_reproducible",
            FindingStatus::ControlFailed => "control_failed",
        }
    }

    /// The rung the claim wording sees: only a confirmed or verified finding is one it may call reproduced.
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
        (false, _) => FindingStatus::Reproduced,
    }
}

/// Whether the checks ran in an environment the bundle's recorded identity pins: the checks recorded one, and the replay
/// that compared it saw no difference. A finding checked without a recorded identity, or on a host that differs, is
/// not environment-pinned and its wording says so.
pub fn environment_pinned(checks: &[CheckResult]) -> bool {
    let latest = latest(checks);
    let Some(replay) = latest.get(CheckKind::BundleReplay.as_str()) else {
        return false;
    };
    let unchanged = replay
        .detail
        .get(ENVIRONMENT_CHANGED_KEY)
        .is_some_and(|v| v == "false");
    unchanged && !replay.environment.is_empty() && replay.status == CheckStatus::Pass
}
