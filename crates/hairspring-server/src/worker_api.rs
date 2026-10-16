//! Endpoints used by remote workers (`hairspring worker`). All require the `run` role, and
//! every job-scoped call is checked against the lease owner by the service layer.
use crate::api::{blocking, ApiJson};
use crate::auth::Caller;
use crate::error::ApiError;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use hairspring_engine::backend::Progress;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::TrialReport;
use hairspring_service::{Role, ServiceError};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_CAPS_BYTES: usize = 4096;

fn hex_id(prefix: &str, s: &str) -> bool {
    s.strip_prefix(prefix)
        .is_some_and(|t| t.len() == 16 && t.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub(crate) fn check_worker(id: &str) -> Result<(), ApiError> {
    if hex_id("wrk_", id) {
        Ok(())
    } else {
        Err(ApiError::bad_request("worker_id has an invalid format"))
    }
}

fn check_job(id: &str) -> Result<(), ApiError> {
    if hex_id("job_", id) {
        Ok(())
    } else {
        Err(ApiError::not_found())
    }
}

#[derive(Deserialize)]
pub struct Register {
    name: String,
    caps: Option<Value>,
}

pub async fn register(
    State(s): State<AppState>,
    caller: Caller,
    ApiJson(b): ApiJson<Register>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Run)?;
    let caps = b
        .caps
        .map(|c| c.to_string())
        .unwrap_or_else(|| "{}".to_string());
    if caps.len() > MAX_CAPS_BYTES {
        return Err(ApiError::bad_request("caps is too large"));
    }
    let (w, secret) = blocking(move || s.svc.register_remote_worker(&b.name, &caps)).await?;
    // The secret is shown once; only its hash is stored. The worker presents it to send trial records.
    Ok(Json(json!({ "worker_id": w.id, "worker_secret": secret })))
}

#[derive(Deserialize)]
pub struct WorkerRef {
    worker_id: String,
    /// The secret issued at registration. Every worker endpoint requires it: a worker id alone is not a credential.
    worker_secret: Option<String>,
}

pub async fn lease(
    State(s): State<AppState>,
    caller: Caller,
    ApiJson(b): ApiJson<WorkerRef>,
) -> Result<Response, ApiError> {
    caller.need(Role::Run)?;
    check_worker(&b.worker_id)?;
    let job = blocking(move || {
        s.svc
            .authenticate_worker(&b.worker_id, b.worker_secret.as_deref())?;
        s.svc
            .touch_worker(&b.worker_id)
            .map_err(|_| ServiceError::InvalidRequest("unknown worker".to_string()))?;
        s.svc.lease(&b.worker_id)
    })
    .await?;
    Ok(match job {
        Some(j) => Json(serde_json::to_value(j).unwrap_or_default()).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

#[derive(Deserialize)]
pub struct ProgressBody {
    worker_id: String,
    worker_secret: Option<String>,
    steps: u64,
    trials: u64,
    message: Option<String>,
}

pub async fn progress(
    State(s): State<AppState>,
    caller: Caller,
    Path(job): Path<String>,
    ApiJson(b): ApiJson<ProgressBody>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Run)?;
    check_job(&job)?;
    check_worker(&b.worker_id)?;
    let message: String = b.message.unwrap_or_default().chars().take(200).collect();
    let keep_going = blocking(move || {
        s.svc
            .authenticate_worker(&b.worker_id, b.worker_secret.as_deref())?;
        s.svc.report_job_progress(
            &job,
            &b.worker_id,
            &Progress {
                steps: b.steps,
                trials: b.trials,
                message,
            },
        )
    })
    .await?;
    Ok(Json(json!({ "ok": true, "continue": keep_going })))
}

#[derive(Deserialize)]
pub struct CompleteBody {
    worker_id: String,
    worker_secret: Option<String>,
    outcome: RunOutcome,
}

pub async fn complete(
    State(s): State<AppState>,
    caller: Caller,
    Path(job): Path<String>,
    ApiJson(b): ApiJson<CompleteBody>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Run)?;
    check_job(&job)?;
    check_worker(&b.worker_id)?;
    blocking(move || {
        s.svc
            .authenticate_worker(&b.worker_id, b.worker_secret.as_deref())?;
        s.svc.complete(&job, &b.worker_id, &b.outcome)
    })
    .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct FailBody {
    worker_id: String,
    worker_secret: Option<String>,
    error: String,
}

pub async fn fail(
    State(s): State<AppState>,
    caller: Caller,
    Path(job): Path<String>,
    ApiJson(b): ApiJson<FailBody>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Run)?;
    check_job(&job)?;
    check_worker(&b.worker_id)?;
    let error: String = b.error.chars().take(500).collect();
    blocking(move || {
        s.svc
            .authenticate_worker(&b.worker_id, b.worker_secret.as_deref())?;
        s.svc.fail(&job, &b.worker_id, &error)
    })
    .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn heartbeat(
    State(s): State<AppState>,
    caller: Caller,
    Path(job): Path<String>,
    ApiJson(b): ApiJson<WorkerRef>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Run)?;
    check_job(&job)?;
    check_worker(&b.worker_id)?;
    let alive = blocking(move || {
        s.svc
            .authenticate_worker(&b.worker_id, b.worker_secret.as_deref())?;
        s.svc.heartbeat(&job, &b.worker_id)
    })
    .await?;
    Ok(Json(json!({ "ok": alive })))
}

#[derive(Deserialize)]
pub struct TrialsBody {
    worker_id: String,
    /// The secret issued at registration. Required: a worker id alone is not a credential.
    worker_secret: Option<String>,
    trials: Vec<TrialReport>,
}

/// Per-trial records for the job's run. Everything in them is checked by the service before anything is stored.
pub async fn trials(
    State(s): State<AppState>,
    caller: Caller,
    Path(job): Path<String>,
    ApiJson(b): ApiJson<TrialsBody>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Run)?;
    check_job(&job)?;
    check_worker(&b.worker_id)?;
    let receipt = blocking(move || {
        s.svc
            .ingest_trials_with_secret(&job, &b.worker_id, b.worker_secret.as_deref(), &b.trials)
    })
    .await?;
    Ok(Json(json!({
        "ok": true,
        "inserted": receipt.inserted,
        "duplicates": receipt.duplicates,
    })))
}
