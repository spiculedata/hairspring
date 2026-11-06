//! Run inputs: recording the hashes of what a run executed with, and telling a reader which inputs were recorded.
//!
//! A worker (or the process itself) reports the inputs once per job, as one SHA-256 digest per input. The service checks
//! the report before the store sees it: the scheme must be the run's backend, and every name must be one the scheme has.
//! The store checks that every value is a digest. A refusal uses fixed words and never repeats what the caller sent.
//!
//! Digests are kept, never served: a short value can be guessed from its digest. A reader is told which inputs were
//! recorded and which were not, by name, and whether the reporter was trusted.
use crate::{Result, Service, ServiceError};
use hairspring_core::point::address::BackendKind;
use hairspring_core::point::encoding::{escape_value, sha256_hex};
use hairspring_core::point::input_hashes::{known_fields, InputHashes, MAX_INPUT_FIELDS};
use hairspring_engine::engine::Engine;
use hairspring_store::{now_ms, NewRunInputs, RunInputsOutcome, RunInputsRejection, RunRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn invalid(why: &str) -> ServiceError {
    ServiceError::InvalidRequest(why.to_string())
}

/// What a worker sends: the scheme, the digest of each input it recorded by name, and the names it could not record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInputsSubmission {
    pub scheme: String,
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub missing: Vec<String>,
}

impl RunInputsSubmission {
    pub fn from_hashes(hashes: &InputHashes) -> RunInputsSubmission {
        RunInputsSubmission {
            scheme: hashes.scheme.clone(),
            fields: hashes.fields.clone(),
            missing: hashes.missing.clone(),
        }
    }
}

/// The answer to a recording: `stored`, `replaced` (the job's earlier attempt had recorded others), or `duplicate` when
/// the same inputs were already there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunInputsReceipt {
    pub outcome: &'static str,
}

/// What a reader is told about a run's inputs. Names only: no digest, so nothing here can be tried against a guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunInputsView {
    /// False for a run stored before inputs were recorded, and for one whose worker recorded none.
    pub recorded: bool,
    pub scheme: Option<String>,
    /// The inputs that were recorded, by name.
    pub fields: Vec<String>,
    /// The inputs the worker could not record, by name.
    pub missing: Vec<String>,
    /// Whether the reporter was trusted when it reported (a snapshot, as for trial rows).
    pub reporter_trusted: Option<bool>,
}

/// Checks a submission against the scheme it names and the run it is for, in fixed words: the scheme is the run's
/// backend, every name is one the scheme has, every name the scheme has is either recorded or said to be missing, and
/// the target and backend digests are those of this run's own target and backend.
fn check(sub: &RunInputsSubmission, run: &RunRecord) -> Result<()> {
    let Some(known) = known_fields(&sub.scheme) else {
        return Err(invalid("unknown input scheme"));
    };
    let belongs =
        BackendKind::from_id(&run.backend).is_some_and(|kind| kind.scheme() == sub.scheme);
    if !belongs {
        return Err(invalid("the inputs do not belong to this run's backend"));
    }
    if sub.fields.len() + sub.missing.len() > MAX_INPUT_FIELDS {
        return Err(invalid("too many inputs"));
    }
    let names = sub.fields.keys().chain(sub.missing.iter());
    if names
        .into_iter()
        .any(|name| !known.contains(&name.as_str()))
    {
        return Err(invalid("an input name is not known for this scheme"));
    }
    let complete = known
        .iter()
        .all(|name| sub.fields.contains_key(*name) || sub.missing.iter().any(|m| m == name));
    if !complete {
        return Err(invalid("the inputs leave out an input this scheme has"));
    }
    let digest_of = |text: &str| sha256_hex(text.as_bytes());
    let belongs =
        |field: &str, expected: String| sub.fields.get(field).is_none_or(|d| *d == expected);
    if !(belongs("target", digest_of(&escape_value(&run.target)))
        && belongs("backend", digest_of(&run.backend)))
    {
        return Err(invalid("the inputs do not belong to this run"));
    }
    Ok(())
}

impl Service {
    /// Records the inputs of the run `job_id` belongs to, for code running inside the process, which holds the lease
    /// directly. It skips the credential check, so no network handler may call it: a remote worker uses
    /// [`Service::record_run_inputs_with_secret`].
    pub fn record_run_inputs(
        &self,
        job_id: &str,
        worker_id: &str,
        sub: &RunInputsSubmission,
    ) -> Result<RunInputsReceipt> {
        self.record_run_inputs_authenticated(job_id, worker_id, sub)
    }

    /// Records the inputs reported by a remote worker, after checking its secret.
    pub fn record_run_inputs_with_secret(
        &self,
        job_id: &str,
        worker_id: &str,
        secret: Option<&str>,
        sub: &RunInputsSubmission,
    ) -> Result<RunInputsReceipt> {
        self.authenticate_worker(worker_id, secret)?;
        self.record_run_inputs_authenticated(job_id, worker_id, sub)
    }

    fn record_run_inputs_authenticated(
        &self,
        job_id: &str,
        worker_id: &str,
        sub: &RunInputsSubmission,
    ) -> Result<RunInputsReceipt> {
        let job = self.owned_running_job(job_id, worker_id)?;
        let run = self
            .store
            .get_run(&job.run_id)?
            .ok_or_else(|| ServiceError::NotFound(format!("run {}", job.run_id)))?;
        check(sub, &run)?;
        let new = NewRunInputs {
            run_id: run.id.clone(),
            scheme: sub.scheme.clone(),
            fields: sub.fields.clone(),
            missing: sub.missing.clone(),
            reporter_id: worker_id.to_string(),
            reporter_trusted: self.worker_is_trusted(worker_id)?,
            attempt: job.attempts,
        };
        match self.store.put_run_inputs(&new, now_ms())? {
            RunInputsOutcome::Stored => Ok(RunInputsReceipt { outcome: "stored" }),
            RunInputsOutcome::Replaced => Ok(RunInputsReceipt {
                outcome: "replaced",
            }),
            RunInputsOutcome::Duplicate => Ok(RunInputsReceipt {
                outcome: "duplicate",
            }),
            RunInputsOutcome::Rejected(RunInputsRejection::RunNotFound) => {
                Err(ServiceError::NotFound(format!("run {}", run.id)))
            }
            RunInputsOutcome::Rejected(RunInputsRejection::Stale) => Err(invalid(
                "these inputs are from an earlier attempt of this job",
            )),
            RunInputsOutcome::Rejected(RunInputsRejection::Conflict) => {
                Err(invalid("this run already has different inputs recorded"))
            }
            RunInputsOutcome::Rejected(RunInputsRejection::Invalid(why)) => Err(invalid(why)),
        }
    }

    /// Records what `engine` knows about the inputs of the job just leased, when it knows any. Best effort: a run is
    /// never held up or failed because its inputs could not be recorded, it just has none, which readers see as "not
    /// recorded". Only fixed words are logged.
    pub(crate) fn record_engine_inputs(
        &self,
        engine: &Engine,
        job_id: &str,
        worker_id: &str,
        run_id: &str,
        target: &str,
    ) {
        let Some(draft) = engine.run_inputs(target) else {
            return;
        };
        let sub = RunInputsSubmission::from_hashes(&draft.input_hashes());
        if self.record_run_inputs(job_id, worker_id, &sub).is_err() {
            eprintln!("hairspring: the inputs of {run_id} could not be recorded");
        }
    }

    /// Which inputs were recorded for a run, by name. `NotFound` for an unknown run.
    pub fn run_inputs_view(&self, run_id: &str) -> Result<RunInputsView> {
        if self.store.get_run(run_id)?.is_none() {
            return Err(ServiceError::NotFound(format!("run {run_id}")));
        }
        Ok(match self.store.get_run_inputs(run_id)? {
            None => RunInputsView {
                recorded: false,
                scheme: None,
                fields: Vec::new(),
                missing: Vec::new(),
                reporter_trusted: None,
            },
            Some(record) => RunInputsView {
                recorded: true,
                scheme: Some(record.scheme),
                fields: record.fields.into_keys().collect(),
                missing: record.missing,
                reporter_trusted: Some(record.reporter_trusted),
            },
        })
    }
}
