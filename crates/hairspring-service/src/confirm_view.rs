//! How a failure's stored checks are served: typed facts at the top level, and the free text a runner reported under
//! `guest`, which is untrusted like every other text that starts outside this process.
use crate::{Result, ServiceError};
use hairspring_engine::confirm::{
    standing, CheckCounts, CheckKind, CheckResult, CheckStatus, FindingStatus,
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
    /// `candidate`, `rerun_repeated`, `confirmed`, `verified`, `not_reproducible` or `control_failed`, derived from
    /// one confirmation among the checks below and nothing else.
    pub finding: String,
    /// The confirmation `finding` is derived from: the newest that finished (nothing skipped), or the newest when none
    /// did. `None` for a failure with no checks.
    pub confirmation_id: Option<String>,
    /// The checks of that confirmation ran in the environment the bundle recorded, and it still matched.
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

/// The stored checks of one confirmation, with who ran them.
struct Group {
    id: String,
    runner: String,
    trusted: bool,
    results: Vec<CheckResult>,
}

/// The confirmations in `records`, oldest first. Rows are kept together by confirmation, in the order the store lists
/// them (creation time, then the order they were written), and a confirmation is placed where its first row is.
fn groups_of(records: &[CheckRecord]) -> Result<Vec<Group>> {
    let mut groups: Vec<Group> = Vec::new();
    for record in records {
        let position = match groups.iter().position(|g| g.id == record.confirmation_id) {
            Some(p) => p,
            None => {
                groups.push(Group {
                    id: record.confirmation_id.clone(),
                    runner: record.runner.clone(),
                    trusted: record.runner_trusted,
                    results: Vec::new(),
                });
                groups.len() - 1
            }
        };
        groups[position].results.extend(result_of(record)?);
    }
    groups.retain(|g| !g.results.is_empty());
    Ok(groups)
}

/// What the checks leave a failure as, and the confirmation that says so.
pub(crate) struct Derived {
    pub status: FindingStatus,
    pub pinned: bool,
    pub confirmation_id: Option<String>,
    pub runner: String,
    pub runner_trusted: bool,
}

fn derive_from(groups: &[&Group]) -> Derived {
    let sets: Vec<Vec<CheckResult>> = groups.iter().map(|g| g.results.clone()).collect();
    let found = standing(&sets);
    let from = found.confirmation.map(|i| groups[i]);
    Derived {
        status: found.status,
        pinned: found.pinned,
        confirmation_id: from.map(|g| g.id.clone()),
        runner: from.map(|g| g.runner.clone()).unwrap_or_default(),
        runner_trusted: from.is_some_and(|g| g.trusted),
    }
}

/// The state as the record shows it, from every confirmation whoever ran it.
pub(crate) fn derive(records: &[CheckRecord]) -> Result<Derived> {
    let groups = groups_of(records)?;
    Ok(derive_from(&groups.iter().collect::<Vec<_>>()))
}

/// The state a claim may rest on: from the confirmations of trusted runners only, so that nothing an untrusted runner
/// stored can lift, or complete, what a trusted one found. A failure that only untrusted runners checked is read from
/// theirs, and the claim gate then refuses it for that reason.
pub(crate) fn derive_for_claim(records: &[CheckRecord]) -> Result<Derived> {
    let groups = groups_of(records)?;
    let trusted: Vec<&Group> = groups.iter().filter(|g| g.trusted).collect();
    Ok(if trusted.is_empty() {
        derive_from(&groups.iter().collect::<Vec<_>>())
    } else {
        derive_from(&trusted)
    })
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
    let derived = derive(records)?;
    Ok(FailureChecks {
        failure_id: failure_id.to_owned(),
        finding: derived.status.as_str().to_owned(),
        confirmation_id: derived.confirmation_id,
        environment_pinned: derived.pinned,
        checks: records.iter().map(view).collect::<Result<_>>()?,
    })
}

pub(crate) fn summary(records: &[CheckRecord]) -> Result<FindingSummary> {
    let derived = derive(records)?;
    Ok(FindingSummary {
        status: derived.status.as_str().to_owned(),
        checks: records.len() as u32,
        environment_pinned: derived.pinned,
    })
}
