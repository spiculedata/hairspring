//! Read endpoints for a run's properties, trials and coverage.
//!
//! Every handler checks the read role first, validates the run id exactly as the other run endpoints do, and hands the
//! query text to the service unchanged: the service is the one place that decides what a parameter may be, so REST and
//! MCP accept the same inputs. Path indexes that are not plain numbers are a 404, like an id of the wrong shape.
//! Nothing here builds a response from guest text; the service puts it under a `guest` object.
use crate::api::blocking;
use crate::auth::Caller;
use crate::error::ApiError;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use hairspring_mcp::validate;
use hairspring_service::{PropertyQuery, Role, SeedQuery, StripQuery, TrialQuery};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

type Params = BTreeMap<String, String>;

fn read_run_id(caller: &Caller, id: &str) -> Result<(), ApiError> {
    caller.need(Role::Read)?;
    if validate::run_id(id) {
        Ok(())
    } else {
        Err(ApiError::not_found())
    }
}

/// A path number of at most `max_digits` plain digits.
fn path_number<T: std::str::FromStr>(s: &str, max_digits: usize) -> Result<T, ApiError> {
    if s.is_empty() || s.len() > max_digits || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::not_found());
    }
    s.parse().map_err(|_| ApiError::not_found())
}

fn limit(p: &Params) -> Result<Option<u32>, ApiError> {
    p.get("limit")
        .map(|v| {
            if !v.is_empty() && v.len() <= 10 && v.bytes().all(|b| b.is_ascii_digit()) {
                v.parse()
                    .map_err(|_| ApiError::bad_request("`limit` is not valid"))
            } else {
                Err(ApiError::bad_request("`limit` is not valid"))
            }
        })
        .transpose()
}

fn param(p: &Params, key: &str) -> Option<String> {
    p.get(key).cloned()
}

fn json<T: Serialize>(v: Option<T>) -> Result<Json<Value>, ApiError> {
    let v = v.ok_or_else(ApiError::not_found)?;
    Ok(Json(serde_json::to_value(v).unwrap_or_default()))
}

pub async fn properties(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
    Query(p): Query<Params>,
) -> Result<Json<Value>, ApiError> {
    read_run_id(&caller, &id)?;
    let q = PropertyQuery {
        status: param(&p, "status"),
        kind: param(&p, "kind"),
        q: param(&p, "q"),
        sort: param(&p, "sort"),
        cursor: param(&p, "cursor"),
        limit: limit(&p)?,
    };
    // A run's label is not something a caller can supply: until runs record one, none is declared.
    json(blocking(move || s.svc.property_page(&id, &q, None)).await?)
}

pub async fn property(
    State(s): State<AppState>,
    caller: Caller,
    Path((id, idx)): Path<(String, String)>,
    Query(p): Query<Params>,
) -> Result<Json<Value>, ApiError> {
    read_run_id(&caller, &id)?;
    let idx: u32 = path_number(&idx, 10)?;
    let q = SeedQuery {
        seeds: param(&p, "seeds"),
        cursor: param(&p, "cursor"),
        limit: limit(&p)?,
    };
    json(blocking(move || s.svc.property_detail(&id, idx, &q)).await?)
}

pub async fn trials(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
    Query(p): Query<Params>,
) -> Result<Json<Value>, ApiError> {
    read_run_id(&caller, &id)?;
    let q = TrialQuery {
        verdict: param(&p, "verdict"),
        fails: param(&p, "fails"),
        hits: param(&p, "hits"),
        misses: param(&p, "misses"),
        degraded: param(&p, "degraded"),
        perturbing: param(&p, "perturbing"),
        fp: param(&p, "fp"),
        seed: param(&p, "seed"),
        cursor: param(&p, "cursor"),
        limit: limit(&p)?,
    };
    json(blocking(move || s.svc.trial_page(&id, &q)).await?)
}

pub async fn trial(
    State(s): State<AppState>,
    caller: Caller,
    Path((id, seq)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    read_run_id(&caller, &id)?;
    let seq: u64 = path_number(&seq, 19)?;
    json(blocking(move || s.svc.trial_detail(&id, seq)).await?)
}

/// A stream of a trial's kept output as a plain-text download. The file name is built from the run id, the sequence
/// number and the stream name, all of which this function has checked; nothing from the program under test is in it.
pub async fn trial_output(
    State(s): State<AppState>,
    caller: Caller,
    Path((id, seq, stream)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    read_run_id(&caller, &id)?;
    let seq: u64 = path_number(&seq, 19)?;
    let (run, name) = (id.clone(), stream.clone());
    let text = blocking(move || s.svc.trial_output(&id, seq, &stream))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{run}-{seq}-{name}.txt\""),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            // The text is the program's own output. If a browser ever renders it, it gets no scripts and no origin.
            (header::CONTENT_SECURITY_POLICY, "sandbox".to_owned()),
        ],
        text,
    )
        .into_response())
}

/// The seed strip: a code per judged trial, in pages. See `hairspring_service::view::StripPage`.
pub async fn strip(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
    Query(p): Query<Params>,
) -> Result<Json<Value>, ApiError> {
    read_run_id(&caller, &id)?;
    let q = StripQuery {
        cursor: param(&p, "cursor"),
        limit: limit(&p)?,
    };
    json(blocking(move || s.svc.strip(&id, &q)).await?)
}

pub async fn coverage(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    read_run_id(&caller, &id)?;
    json(blocking(move || s.svc.coverage(&id)).await?)
}
