//! REST handlers for humans, the web UI and CI. Every handler authenticates through
//! [`Caller`], checks its role first, validates identifiers, and runs storage and
//! engine work on the blocking pool.
use crate::auth::Caller;
use crate::error::ApiError;
use crate::ndjson::list_response;
use crate::read_api::{limit, param, Params};
use crate::AppState;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, FromRequest, Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use hairspring_engine::config::Budget;
use hairspring_engine::mode::RunMode;
use hairspring_mcp::validate;
use hairspring_service::{FailureListQuery, Role, RunListQuery, ServiceError, StartRun};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;

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

/// Liveness for probes, open to everyone. The organisation name, when one is configured, is added only for a caller
/// the other endpoints would answer: a valid token (any role), or any caller when the server has no tokens.
///
/// A request without an `Authorization` header is never limited, so probes keep working. One that carries the header
/// goes through the same admission rule as the limited endpoints ([`crate::admit`]): a valid token counts against its
/// own bucket, anything else against the client address's anonymous bucket, and a valid token is not admitted while
/// that address bucket is over the limit. When the request is not admitted the name is left out and the answer is
/// still 200. Health cannot be used to guess tokens faster than the limit allows, and one token polling it cannot
/// spend the address's budget and lock other tokens on that address out.
pub async fn health(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Json<Value> {
    let mut body = json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") });
    if let Some(name) = &s.organisation {
        if may_see_organisation(&s, peer, headers.get(header::AUTHORIZATION)) {
            body["organisation"] = json!(name.as_ref());
        }
    }
    Json(body)
}

fn may_see_organisation(
    s: &AppState,
    peer: SocketAddr,
    authorization: Option<&header::HeaderValue>,
) -> bool {
    if s.auth.is_open() {
        return true;
    }
    let Some(authorization) = authorization else {
        return false;
    };
    let authorization = authorization.to_str().ok();
    crate::admit(s, peer, authorization) && s.auth.authenticate(authorization).is_some()
}

/// Who the caller is and what they may do, so a client can hide or explain actions instead of guessing from a 403.
/// It carries the role name and what that role allows, whether the server is in open mode, and nothing that
/// identifies or could stand in for the token.
pub async fn me(State(s): State<AppState>, caller: Caller) -> Json<Value> {
    let can = |needed: Role| caller.role.allows(needed);
    Json(json!({
        "role": caller.role.as_str(),
        "open_mode": s.auth.is_open(),
        "can": {
            "read": can(Role::Read),
            "run": can(Role::Run),
            "admin": can(Role::Admin),
        },
        "roles": [Role::Read.as_str(), Role::Run.as_str(), Role::Admin.as_str()],
    }))
}

pub async fn targets(
    State(s): State<AppState>,
    caller: Caller,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    caller.need(Role::Read)?;
    let value = json!({ "targets": s.svc.list_targets(&s.engine) });
    list_response(&headers, value, "targets")
}

/// Runs, newest first, in pages. Every parameter is optional and checked by the service, which is the one place that
/// decides what a value may be: `state`, `target`, `mode`, `cursor`, `limit`.
pub async fn list_runs(
    State(s): State<AppState>,
    caller: Caller,
    headers: HeaderMap,
    Query(p): Query<Params>,
) -> Result<Response, ApiError> {
    caller.need(Role::Read)?;
    let q = RunListQuery {
        state: param(&p, "state"),
        target: param(&p, "target"),
        mode: param(&p, "mode"),
        cursor: param(&p, "cursor"),
        limit: limit(&p)?,
    };
    let page = blocking(move || s.svc.run_page(&q)).await?;
    list_response(
        &headers,
        serde_json::to_value(page).unwrap_or_default(),
        "runs",
    )
}

/// A bounded lookup of ids and names for the web UI's search box: `q` only, no paging. The service decides what text
/// may be searched and what is looked at; an error names `q` and never repeats it.
pub async fn search(
    State(s): State<AppState>,
    caller: Caller,
    Query(p): Query<Params>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Read)?;
    let q = param(&p, "q");
    let results = blocking(move || s.svc.search(&s.engine, q.as_deref())).await?;
    Ok(Json(serde_json::to_value(results).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct CreateRun {
    target: String,
    seed: Option<u64>,
    budget: Option<Budget>,
    /// `search` (the default: stop at the first confirmed failure) or `sweep` (run the whole trial budget).
    mode: Option<RunMode>,
}

pub async fn create_run(
    State(s): State<AppState>,
    caller: Caller,
    ApiJson(body): ApiJson<CreateRun>,
) -> Result<Response, ApiError> {
    caller.need(Role::Run)?;
    let run = blocking(move || {
        s.svc.start_run_with_mode(
            &s.engine,
            &StartRun {
                target: body.target,
                seed: body.seed,
                budget: body.budget,
            },
            body.mode.unwrap_or_default(),
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
    headers: HeaderMap,
    Query(p): Query<Params>,
) -> Result<Response, ApiError> {
    caller.need(Role::Read)?;
    let q = FailureListQuery {
        run: param(&p, "run"),
        property: param(&p, "property"),
        status: param(&p, "status"),
        cursor: param(&p, "cursor"),
        limit: limit(&p)?,
    };
    let page = blocking(move || s.svc.failure_page(&q)).await?;
    list_response(
        &headers,
        serde_json::to_value(page).unwrap_or_default(),
        "failures",
    )
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
    /// Replay even when the host's dynamic loader differs from the recording. Off unless asked for.
    #[serde(default)]
    allow_environment_change: bool,
}

pub async fn replay(
    State(s): State<AppState>,
    caller: Caller,
    ApiJson(body): ApiJson<ReplayBody>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Run)?; // replay executes target code, so it needs more than read access
    let options = hairspring_service::ReplayOptions {
        allow_environment_change: body.allow_environment_change,
    };
    let out = blocking(move || s.svc.replay_bundle_with(&s.engine, &body.bundle, options)).await?;
    Ok(Json(serde_json::to_value(out).unwrap_or_default()))
}

/// The workers, each with its state, trust, capabilities and held jobs, and the queue behind them. The list is bounded
/// by the server's worker limit, so it is not paged. Worker ids act as lease credentials for the worker endpoints, so
/// only `run`+ callers may see them. Nothing here reads a secret, a secret digest, a job payload or guest output.
pub async fn workers(
    State(s): State<AppState>,
    caller: Caller,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    caller.need(Role::Read)?;
    let reveal_ids = caller.role.allows(Role::Run);
    let overview = blocking(move || s.svc.workers_overview()).await?;
    let items: Vec<Value> = overview
        .workers
        .into_iter()
        .map(|w| {
            let mut v = serde_json::to_value(w).unwrap_or_default();
            if !reveal_ids {
                if let Some(o) = v.as_object_mut() {
                    o.remove("id");
                }
            }
            v
        })
        .collect();
    list_response(
        &headers,
        json!({ "workers": items, "queue": overview.queue }),
        "workers",
    )
}

/// MCP over HTTP: one JSON-RPC message (or batch) per POST.
pub async fn mcp(
    State(s): State<AppState>,
    caller: Caller,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    // Browsers can send text/plain and form posts cross-site without a preflight; JSON-RPC must be application/json.
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| {
            ct.trim()
                .to_ascii_lowercase()
                .starts_with("application/json")
        });
    if !is_json {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        ));
    }
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

#[derive(Deserialize)]
pub struct TrustBody {
    trusted: bool,
}

/// Marks a worker's data as trusted or not. Claims need trusted workers; every other view shows untrusted data,
/// labelled.
pub async fn set_worker_trust(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
    ApiJson(b): ApiJson<TrustBody>,
) -> Result<Json<Value>, ApiError> {
    caller.need(Role::Admin)?;
    crate::worker_api::check_worker(&id).map_err(|_| ApiError::not_found())?;
    blocking(move || s.svc.set_worker_trust(&id, b.trusted)).await?;
    Ok(Json(json!({ "ok": true })))
}
