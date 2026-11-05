//! Reports the inputs of a job to the server before the job runs: one digest per input the engine's backend can
//! describe. Best effort. A server that predates the endpoint, or a refusal, leaves the run without recorded inputs,
//! which readers see as "not recorded"; it never stops the job.
use crate::{Api, CallError};
use hairspring_engine::engine::Engine;
use hairspring_service::{LeasedJob, RunInputsSubmission};
use serde_json::json;

pub(crate) fn report(api: &Api, engine: &Engine, worker_id: &str, secret: &str, job: &LeasedJob) {
    let Some(draft) = engine.run_inputs(&job.target) else {
        return;
    };
    let inputs = RunInputsSubmission::from_hashes(&draft.input_hashes());
    let body = json!({ "worker_id": worker_id, "worker_secret": secret, "inputs": inputs });
    let path = format!("/api/worker/jobs/{}/inputs", job.job_id);
    // Only fixed words and the status are printed: nothing the server sent back is repeated.
    match api.post(&path, &body) {
        Ok(_) => {}
        Err(CallError::Status(code, _)) => eprintln!(
            "hairspring-worker: the server answered {code} to the inputs of job {}; the run has none recorded",
            job.job_id
        ),
        Err(CallError::Transport(_)) => eprintln!(
            "hairspring-worker: could not reach the server with the inputs of job {}; the run has none recorded",
            job.job_id
        ),
    }
}
