//! Server-sent events for a run: stored events are replayed, then new ones follow
//! until the run is finished (or the server stops).
use crate::api::blocking;
use crate::auth::Caller;
use crate::error::ApiError;
use crate::AppState;
use anthill_mcp::validate;
use anthill_service::Role;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::OwnedSemaphorePermit;

const POLL: Duration = Duration::from_millis(400);
const PAGE: u32 = 100;
const MAX_STREAM: Duration = Duration::from_secs(600);

#[derive(Deserialize)]
pub struct After {
    after: Option<i64>,
}

struct Cursor {
    state: AppState,
    run_id: String,
    last_id: i64,
    started: Instant,
    done: bool,
    /// Held for the life of the stream; released when it ends.
    _slot: OwnedSemaphorePermit,
}

pub async fn events(
    State(s): State<AppState>,
    caller: Caller,
    Path(run_id): Path<String>,
    Query(q): Query<After>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    caller.need(Role::Read)?;
    if !validate::run_id(&run_id) {
        return Err(ApiError::not_found());
    }
    let (svc, id) = (s.svc.clone(), run_id.clone());
    blocking(move || svc.get_run(&id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    let slot = s.sse_slots.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "too many open event streams",
        )
    })?;

    let cursor = Cursor {
        state: s,
        run_id,
        last_id: q.after.unwrap_or(0).max(0),
        started: Instant::now(),
        done: false,
        _slot: slot,
    };
    let stream = stream::unfold(cursor, |mut c| async move {
        if c.done {
            return None;
        }
        loop {
            let (svc, id, after) = (c.state.svc.clone(), c.run_id.clone(), c.last_id);
            let batch = blocking(move || svc.events(&id, after, PAGE))
                .await
                .unwrap_or_default();
            if !batch.is_empty() {
                c.last_id = batch.last().map(|e| e.id).unwrap_or(c.last_id);
                let events: Vec<Result<Event, Infallible>> = batch
                    .into_iter()
                    .map(|e| {
                        Ok(Event::default()
                            .id(e.id.to_string())
                            .event(e.kind)
                            .data(e.payload_json))
                    })
                    .collect();
                return Some((stream::iter(events), c));
            }
            let (svc, id) = (c.state.svc.clone(), c.run_id.clone());
            let finished = blocking(move || svc.get_run(&id))
                .await
                .ok()
                .flatten()
                .map(|d| d.run.state.is_terminal())
                .unwrap_or(true);
            if finished
                || c.state.stopping.load(Ordering::SeqCst)
                || c.started.elapsed() > MAX_STREAM
            {
                return None;
            }
            tokio::time::sleep(POLL).await;
        }
    })
    .flatten();
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
