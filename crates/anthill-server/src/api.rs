//! REST handlers for humans, the web UI and CI. Every handler authenticates through
//! [`Caller`], checks its role first, validates identifiers, and runs storage and
//! engine work on the blocking pool.
use crate::auth::Caller;
use crate::error::ApiError;
use crate::AppState;
use anthill_engine::config::Budget;
use anthill_mcp::validate;
use anthill_service::{Role, ServiceError, StartRun};
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

/// Runs blocking service work off the async executor.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static,
) -> Result<T, ApiError> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result.map_err(ApiError::from),
        Err(e) => Err(ApiError::internal(format!("task failed: {e}"))),
    }
}

/// JSON body extractor that reports every problem as 400 (or 413 when too large).
pub struct ApiJson<T>(pub T);

impl<T: DeserializeOwned> FromRequest<AppState> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, ApiError> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(ApiJson(v)),
            Err(rej) => Err(reject(rej)),
        }
    }
}

fn reject(rej: JsonRejection) -> ApiError {
    let status = match rej.status() {
        StatusCode::UNPROCESSABLE_ENTITY => StatusCode::BAD_REQUEST,
        other => other,
    };
    ApiError::new(
        status,
        format!(
            "invalid request body: {}",
            rej.body_text().lines().next().unwrap_or("malformed JSON")
        ),
    )
}

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

pub async fn targets(State(s): State<AppState>, caller: Caller) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    Ok(Json(json!({ "targets": s.svc.list_targets(&s.engine) })))
}

#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<u32>,
}

pub async fn list_runs(
    State(s): State<AppState>,
    caller: Caller,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    let limit = q.limit.unwrap_or(50);
    let runs = blocking(move || s.svc.list_runs(limit)).await?;
    Ok(Json(json!({ "runs": runs })))
}

#[derive(Deserialize)]
pub struct CreateRun {
    target: String,
    seed: Option<u64>,
    budget: Option<Budget>,
}

pub async fn create_run(
    State(s): State<AppState>,
    caller: Caller,
    ApiJson(body): ApiJson<CreateRun>,
) -> Result<Response, ApiError> {
    caller.need(Role::Run)?;
    let run = blocking(move || {
        s.svc.start_run(
            &s.engine,
            &StartRun {
                target: body.target,
                seed: body.seed,
                budget: body.budget,
            },
        )
    })
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "run": run }))).into_response())
}

pub async fn get_run(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    if !validate::run_id(&id) {
        return Err(ApiError::not_found());
    }
    let detail = blocking(move || s.svc.get_run(&id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(serde_json::to_value(detail).unwrap_or_default()))
}

pub async fn cancel_run(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Run)?;
    if !validate::run_id(&id) {
        return Err(ApiError::not_found());
    }
    let cancelled = blocking(move || s.svc.cancel_run(&id)).await?;
    Ok(Json(json!({ "cancelled": cancelled })))
}

pub async fn list_failures(
    State(s): State<AppState>,
    caller: Caller,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    let limit = q.limit.unwrap_or(50);
    let failures = blocking(move || s.svc.recent_failures(limit)).await?;
    Ok(Json(json!({ "failures": failures })))
}

pub async fn get_failure(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    if !validate::failure_id(&id) {
        return Err(ApiError::not_found());
    }
    let f = blocking(move || s.svc.get_failure(&id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(serde_json::to_value(f).unwrap_or_default()))
}

pub async fn get_bundle(
    State(s): State<AppState>,
    caller: Caller,
    Path(hash): Path<String>,
) -> Result<Response, ApiError> {
    caller.need(Role::Read)?;
    if !validate::bundle_hash(&hash) {
        return Err(ApiError::not_found());
    }
    let text = blocking(move || s.svc.get_bundle(&hash))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response())
}

#[derive(Deserialize)]
pub struct ReplayBody {
    bundle: String,
}

pub async fn replay(
    State(s): State<AppState>,
    caller: Caller,
    ApiJson(body): ApiJson<ReplayBody>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    let out = blocking(move || s.svc.replay_bundle(&s.engine, &body.bundle)).await?;
    Ok(Json(serde_json::to_value(out).unwrap_or_default()))
}

pub async fn workers(State(s): State<AppState>, caller: Caller) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    let workers = blocking(move || s.svc.list_workers()).await?;
    Ok(Json(json!({ "workers": workers })))
}

/// MCP over HTTP: one JSON-RPC message (or batch) per POST.
pub async fn mcp(
    State(s): State<AppState>,
    caller: Caller,
    body: String,
) -> Result<Response, ApiError> {
    let role = caller.role;
    let mcp = s.mcp.clone();
    let reply = tokio::task::spawn_blocking(move || mcp.handle_message(&body, role))
        .await
        .map_err(|e| ApiError::internal(format!("task failed: {e}")))?;
    Ok(match reply {
        Some(text) => ([(header::CONTENT_TYPE, "application/json")], text).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    })
}
