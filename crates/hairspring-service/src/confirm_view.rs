//! How a failure's stored checks are served: typed facts at the top level, and the free text a runner reported under
//! `guest`, which is untrusted like every other text that starts outside this process.
use crate::{Result, ServiceError};
use hairspring_engine::confirm::{
    environment_pinned, finding_status, CheckCounts, CheckKind, CheckResult, CheckStatus,
    FindingStatus,
};
use hairspring_store::CheckRecord;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Text a runner reported for a check. Anything that renders it, or hands it to a model, must treat it as untrusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckGuest {
    /// The Hermit command the check ran, which names the program under test.
    pub command: String,
    /// Short labelled facts: why the check ended as it did, caveats.
    pub detail: BTreeMap<String, String>,
}

/// One stored check. The runner's own id is stored but never served: a worker id is a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckView {
    pub id: String,
    pub confirmation_id: String,
    pub ordinal: u32,
    pub kind: String,
    pub status: String,
    pub counts: CheckCounts,
    /// Hashes and fingerprints the check saw, the recorded ones first.
    pub artefacts: Vec<String>,
    /// The environment identity the check ran in.
    pub environment: BTreeMap<String, String>,
    pub started_at: i64,
    pub finished_at: i64,
    pub runner_trusted: bool,
    pub created_at: i64,
    pub guest: CheckGuest,
}

/// Where a failure stands, and the checks that say so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FailureChecks {
    pub failure_id: String,
    /// `candidate`, `reproduced`, `confirmed`, `verified`, `not_reproducible` or `control_failed`, derived from the
    /// checks below and nothing else.
    pub finding: String,
    /// The checks ran in the environment the bundle recorded, and it still matched.
    pub environment_pinned: bool,
    pub checks: Vec<CheckView>,
}

/// A failure's standing in one line, for lists and the run page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingSummary {
    pub status: String,
    /// How many checks are stored for the failure.
    pub checks: u32,
    pub environment_pinned: bool,
}

fn corrupt(what: &str) -> ServiceError {
    ServiceError::Internal(format!("a stored check has a corrupt {what}"))
}

fn map_of(json: &str, what: &str) -> Result<BTreeMap<String, String>> {
    serde_json::from_str(json).map_err(|_| corrupt(what))
}

fn list_of(json: &str, what: &str) -> Result<Vec<String>> {
    serde_json::from_str(json).map_err(|_| corrupt(what))
}

/// The result a stored check records, when its kind and status are ones this build knows. A row from a newer build
/// with a kind or status this one cannot read is still listed, but takes no part in deriving the state.
fn result_of(record: &CheckRecord) -> Result<Option<CheckResult>> {
    let (Some(kind), Some(status)) = (
        CheckKind::parse(&record.kind),
        CheckStatus::parse(&record.status),
    ) else {
        return Ok(None);
    };
    Ok(Some(
        CheckResult::new(kind, status)
            .with_counts(record.passed, record.failed, record.inconclusive)
            .with_command(record.command.clone())
            .with_times(record.started_at, record.finished_at)
            .with_environment_fields(map_of(&record.environment_json, "environment")?)
            .with_detail_fields(map_of(&record.detail_json, "detail")?),
    ))
}

fn results_of(records: &[CheckRecord]) -> Result<Vec<CheckResult>> {
    let mut results = Vec::new();
    for record in records {
        results.extend(result_of(record)?);
    }
    Ok(results)
}

fn view(record: &CheckRecord) -> Result<CheckView> {
    Ok(CheckView {
        id: record.id.clone(),
        confirmation_id: record.confirmation_id.clone(),
        ordinal: record.ordinal,
        kind: record.kind.clone(),
        status: record.status.clone(),
        counts: CheckCounts {
            passed: record.passed,
            failed: record.failed,
            inconclusive: record.inconclusive,
        },
        artefacts: list_of(&record.artefacts_json, "artefact list")?,
        environment: map_of(&record.environment_json, "environment")?,
        started_at: record.started_at,
        finished_at: record.finished_at,
        runner_trusted: record.runner_trusted,
        created_at: record.created_at,
        guest: CheckGuest {
            command: record.command.clone(),
            detail: map_of(&record.detail_json, "detail")?,
        },
    })
}

pub(crate) fn failure_checks(failure_id: &str, records: &[CheckRecord]) -> Result<FailureChecks> {
    let results = results_of(records)?;
    Ok(FailureChecks {
        failure_id: failure_id.to_owned(),
        finding: finding_status(&results).as_str().to_owned(),
        environment_pinned: environment_pinned(&results),
        checks: records.iter().map(view).collect::<Result<_>>()?,
    })
}

pub(crate) fn summary(records: &[CheckRecord]) -> Result<FindingSummary> {
    let results = results_of(records)?;
    Ok(FindingSummary {
        status: finding_status(&results).as_str().to_owned(),
        checks: records.len() as u32,
        environment_pinned: environment_pinned(&results),
    })
}

/// The state and environment pin the claim wording sees for a failure's stored checks.
pub(crate) fn standing(records: &[CheckRecord]) -> Result<(FindingStatus, bool)> {
    let results = results_of(records)?;
    Ok((finding_status(&results), environment_pinned(&results)))
}
