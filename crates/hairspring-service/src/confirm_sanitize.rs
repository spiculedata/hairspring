//! Validation of the check results a runner reports. A runner is a separate (possibly remote) process, and what it
//! reports is later shown to people and to AI agents, so a result that cannot be true is refused whole, and every piece
//! of text is cut to size and stripped of control characters rather than trusted. Facts that are enums, counts, times,
//! hashes or environment fields are checked strictly: nothing is guessed or repaired.
use crate::{Result, ServiceError};
use hairspring_core::environment::EnvironmentIdentity;
use hairspring_engine::confirm::{
    CheckResult, CheckStatus, MAX_ARTEFACTS, MAX_ARTEFACT_CHARS, MAX_ENVIRONMENT_ENTRIES,
    MAX_RERUNS,
};
use hairspring_store::MAX_CHECKS_PER_CONFIRMATION;
use std::collections::BTreeSet;

/// No check takes longer than a day: the longest confirmation is ten minutes.
const MAX_CHECK_DURATION_MS: i64 = 24 * 60 * 60 * 1000;

fn invalid(why: &str) -> ServiceError {
    ServiceError::InvalidRequest(format!("a reported check is not valid: {why}"))
}

fn counts(check: &CheckResult) -> Result<()> {
    let c = check.counts;
    let total = u64::from(c.passed) + u64::from(c.failed) + u64::from(c.inconclusive);
    if total > u64::from(MAX_RERUNS) {
        return Err(invalid("it counts more runs than a check may make"));
    }
    let consistent = match check.status {
        CheckStatus::Pass => c.failed == 0 && c.inconclusive == 0,
        CheckStatus::Fail => c.failed > 0,
        CheckStatus::Skipped => total == 0,
        CheckStatus::Inconclusive => true,
    };
    if !consistent {
        return Err(invalid("its counts do not agree with its status"));
    }
    Ok(())
}

fn times(check: &CheckResult) -> Result<()> {
    let (start, end) = (check.started_at_ms, check.finished_at_ms);
    if start < 0 || end < start || end - start > MAX_CHECK_DURATION_MS {
        return Err(invalid("its times are impossible"));
    }
    Ok(())
}

fn environment(check: &CheckResult) -> Result<()> {
    if check.environment.len() > MAX_ENVIRONMENT_ENTRIES {
        return Err(invalid("its environment has too many fields"));
    }
    check
        .environment
        .iter()
        .try_fold(EnvironmentIdentity::default(), |id, (name, value)| {
            id.with_field(name.as_str(), value.as_str())
        })
        .map(|_| ())
        .map_err(|_| invalid("its environment names a field that is not part of the identity, or gives it a bad value"))
}

fn artefacts(check: &CheckResult) -> Result<()> {
    let is_hash = |a: &String| {
        !a.is_empty()
            && a.len() <= MAX_ARTEFACT_CHARS
            && a.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    if check.artefacts.len() > MAX_ARTEFACTS || !check.artefacts.iter().all(is_hash) {
        return Err(invalid("an artefact is not a hash"));
    }
    Ok(())
}

/// The reported checks of one confirmation, validated and cut to size, or the reason they are refused.
pub(crate) fn checks(reported: &[CheckResult]) -> Result<Vec<CheckResult>> {
    if reported.is_empty() || reported.len() > MAX_CHECKS_PER_CONFIRMATION {
        return Err(invalid("a confirmation holds between one check and eight"));
    }
    let kinds: BTreeSet<&str> = reported.iter().map(|c| c.kind.as_str()).collect();
    if kinds.len() != reported.len() {
        return Err(invalid("a kind of check appears twice in one confirmation"));
    }
    for check in reported {
        counts(check)?;
        times(check)?;
        environment(check)?;
        artefacts(check)?;
    }
    Ok(reported.iter().cloned().map(CheckResult::bounded).collect())
}
