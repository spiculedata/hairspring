//! Confirming a failure: the pipeline run on the engine, every check stored, and the failure's standing read back from
//! the stored checks alone.
//!
//! The pipeline runs where the engine is (the CLI's own process, or the server's embedded engine), so its runner is this
//! process and is trusted as the process's own workers are. A remote worker's report would come through
//! [`Service::record_confirmation`] with its own trust flag, after the same validation.
use crate::confirm_sanitize;
use crate::confirm_view::{failure_checks, summary, FailureChecks, FindingSummary};
use crate::{Result, Service, ServiceError, MAX_BUNDLE_BYTES, MAX_REPLAY_STEPS};
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::Progress;
use hairspring_engine::confirm::{CheckResult, ConfirmRequest};
use hairspring_engine::engine::Engine;
use hairspring_store::{
    new_id, now_ms, CheckRecord, ConfirmationOutcome, NewCheck, NewConfirmation, StoreError,
};

const FAILURE_ID_PREFIX: &str = "fail_";
const FAILURE_ID_HEX: usize = 16;

/// What storing a confirmation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationReceipt {
    pub confirmation_id: String,
    pub outcome: ConfirmationOutcome,
}

fn well_formed_failure_id(id: &str) -> bool {
    id.strip_prefix(FAILURE_ID_PREFIX).is_some_and(|hex| {
        hex.len() == FAILURE_ID_HEX && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

fn checked_failure_id(id: &str) -> Result<()> {
    if well_formed_failure_id(id) {
        Ok(())
    } else {
        Err(ServiceError::InvalidRequest(
            "the failure id is not valid".to_string(),
        ))
    }
}

fn json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| ServiceError::Internal(e.to_string()))
}

fn new_check(check: &CheckResult) -> Result<NewCheck> {
    Ok(NewCheck {
        kind: check.kind.as_str().to_owned(),
        status: check.status.as_str().to_owned(),
        passed: check.counts.passed,
        failed: check.counts.failed,
        inconclusive: check.counts.inconclusive,
        command: check.command.clone(),
        detail_json: json(&check.detail)?,
        artefacts_json: json(&check.artefacts)?,
        environment_json: json(&check.environment)?,
        started_at: check.started_at_ms,
        finished_at: check.finished_at_ms,
    })
}

fn store_error(e: StoreError, failure_id: &str) -> ServiceError {
    match e {
        StoreError::NotFound(_) => ServiceError::NotFound(format!("failure {failure_id}")),
        StoreError::LimitExceeded { what, .. } => {
            ServiceError::InvalidRequest(format!("too many checks: {what}"))
        }
        other => ServiceError::Store(other),
    }
}

impl Service {
    /// Replays and reruns the failure `failure_id` with `req`, stores every check, and returns where the failure now
    /// stands. `progress` is told after each run; returning `false` stops the confirmation, and what did not run is
    /// stored as skipped. Nothing is stored for a request the engine refuses.
    pub fn confirm_failure(
        &self,
        engine: &Engine,
        failure_id: &str,
        req: &ConfirmRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<FailureChecks> {
        checked_failure_id(failure_id)?;
        let detail = self
            .get_failure(failure_id)?
            .ok_or_else(|| ServiceError::NotFound(format!("failure {failure_id}")))?;
        let checks = self.confirm_bundle(engine, &detail.bundle, req, progress)?;
        self.record_confirmation(failure_id, &Service::local_worker_id(), true, &checks)?;
        self.failure_checks(failure_id)?
            .ok_or_else(|| ServiceError::NotFound(format!("failure {failure_id}")))
    }

    /// Runs the pipeline on bundle text without storing anything: for a bundle that is not a stored failure.
    pub fn confirm_bundle(
        &self,
        engine: &Engine,
        text: &str,
        req: &ConfirmRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<Vec<CheckResult>> {
        if text.len() > MAX_BUNDLE_BYTES {
            return Err(ServiceError::InvalidRequest(format!(
                "bundle exceeds {MAX_BUNDLE_BYTES} bytes"
            )));
        }
        let bundle =
            Bundle::decode(text).map_err(|e| ServiceError::InvalidRequest(e.to_string()))?;
        if bundle.path.len() > MAX_REPLAY_STEPS {
            return Err(ServiceError::InvalidRequest(format!(
                "bundle has {} steps; the limit is {MAX_REPLAY_STEPS} steps",
                bundle.path.len()
            )));
        }
        Ok(engine.confirm(&bundle, req, progress)?)
    }

    /// Validates the checks a runner reports and stores them as one confirmation of `failure_id`. `runner` is an
    /// opaque label, stored and never served; `runner_trusted` is a snapshot of whether it was trusted when it reported.
    pub fn record_confirmation(
        &self,
        failure_id: &str,
        runner: &str,
        runner_trusted: bool,
        reported: &[CheckResult],
    ) -> Result<ConfirmationReceipt> {
        checked_failure_id(failure_id)?;
        let clean = confirm_sanitize::checks(reported)?;
        let confirmation_id = new_id("conf");
        let new = NewConfirmation {
            id: confirmation_id.clone(),
            failure_id: failure_id.to_owned(),
            runner: runner
                .chars()
                .take(hairspring_store::MAX_CHECK_TAG_BYTES)
                .collect(),
            runner_trusted,
            checks: clean.iter().map(new_check).collect::<Result<_>>()?,
        };
        let outcome = self
            .store
            .add_confirmation(&new, now_ms())
            .map_err(|e| store_error(e, failure_id))?;
        Ok(ConfirmationReceipt {
            confirmation_id,
            outcome,
        })
    }

    /// The stored checks of a failure and where they leave it, or `None` for a failure that does not exist.
    pub fn failure_checks(&self, failure_id: &str) -> Result<Option<FailureChecks>> {
        checked_failure_id(failure_id)?;
        if self.store.get_failure(failure_id)?.is_none() {
            return Ok(None);
        }
        let records = self.store.list_failure_checks(failure_id)?;
        failure_checks(failure_id, &records).map(Some)
    }

    /// A failure's standing in one line.
    pub(crate) fn finding_summary(&self, failure_id: &str) -> Result<FindingSummary> {
        let records: Vec<CheckRecord> = self.store.list_failure_checks(failure_id)?;
        summary(&records)
    }
}
