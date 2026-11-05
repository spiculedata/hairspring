//! Server-sent events for a run: stored events are replayed, then new ones follow
//! until the run is finished (or the server stops).
use crate::api::{blocking, ApiQuery};
use crate::auth::Caller;
use crate::error::ApiError;
use crate::limits::CallerStreamSlot;
use crate::AppState;
use axum::extract::{ConnectInfo, Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use hairspring_mcp::validate;
use hairspring_service::Role;
use hairspring_store::EventRecord;
use serde::Deserialize;
use std::convert::Infallible;
use std::net::SocketAddr;
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
    /// Whether the caller may see worker ids (`run` and above).
    reveal_workers: bool,
    /// Held for the life of the stream; released when it ends.
    _slot: OwnedSemaphorePermit,
    /// This caller's share of the slots, held and released the same way.
    _caller_slot: CallerStreamSlot,
}

pub async fn events(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    caller: Caller,
    Path(run_id): Path<String>,
    ApiQuery(q): ApiQuery<After>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    caller.need(Role::Read)?;
    if !validate::run_id(&run_id) {
        return Err(ApiError::not_found());
    }
    let (svc, id) = (s.svc.clone(), run_id.clone());
    // The run's own row only: the full detail (failures, findings, properties) is not needed to know it exists.
    blocking(move || Ok(svc.store().get_run(&id)?))
        .await?
        .ok_or_else(ApiError::not_found)?;
    // Keyed like the rate limit: the token, or the client address when there are no tokens.
    let caller_key = if s.auth.is_open() {
        crate::ratelimit::anonymous_key(crate::client_ip(&s, peer, &headers))
    } else {
        caller.key.clone()
    };
    let caller_slot = s.caller_streams.try_acquire(caller_key).ok_or_else(|| {
        ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "too many open event streams for this caller",
        )
    })?;
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
        reveal_workers: caller.role.allows(Role::Run),
        _slot: slot,
        _caller_slot: caller_slot,
    };
    let stream = stream::unfold(cursor, |mut c| async move {
        if c.done {
            return None;
        }
        loop {
            let (feed, after) = (
                ServiceFeed {
                    state: c.state.clone(),
                    run_id: c.run_id.clone(),
                },
                c.last_id,
            );
            let step = tokio::task::spawn_blocking(move || poll_once(&feed, after))
                .await
                .unwrap_or(Poll::Idle { finished: true });
            let batch = match step {
                Poll::Events(batch) => batch,
                Poll::Idle { finished } => {
                    if finished
                        || c.state.stopping.load(Ordering::SeqCst)
                        || c.started.elapsed() > MAX_STREAM
                    {
                        return None;
                    }
                    tokio::time::sleep(POLL).await;
                    continue;
                }
            };
            c.last_id = batch.last().map(|e| e.id).unwrap_or(c.last_id);
            let reveal = c.reveal_workers;
            let events: Vec<Result<Event, Infallible>> = batch
                .into_iter()
                .map(|e| {
                    let data = payload_for(&e.payload_json, reveal);
                    Ok(Event::default()
                        .id(e.id.to_string())
                        .event(e.kind)
                        .data(data))
                })
                .collect();
            return Some((stream::iter(events), c));
        }
    })
    .flatten();
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Worker ids act as lease credentials, so callers below `run` do not see them (as in `GET /api/workers`). An
/// event's payload is a JSON object; the `worker` key is removed from it for those callers.
fn payload_for(payload_json: &str, reveal_workers: bool) -> String {
    if reveal_workers {
        return payload_json.to_string();
    }
    match serde_json::from_str::<serde_json::Value>(payload_json) {
        Ok(serde_json::Value::Object(mut fields)) => {
            if fields.remove("worker").is_none() {
                return payload_json.to_string();
            }
            serde_json::Value::Object(fields).to_string()
        }
        _ => payload_json.to_string(),
    }
}

/// What one look at a run's stored state found.
#[derive(Debug)]
enum Poll {
    /// New events past the cursor.
    Events(Vec<EventRecord>),
    /// Nothing new; `finished` says the run is over so nothing more will come.
    Idle { finished: bool },
}

/// The two reads a poll makes, so the ordering between them can be tested.
trait RunFeed {
    fn events_after(&self, after: i64) -> Vec<EventRecord>;
    fn is_terminal(&self) -> bool;
}

struct ServiceFeed {
    state: AppState,
    run_id: String,
}

impl RunFeed for ServiceFeed {
    fn events_after(&self, after: i64) -> Vec<EventRecord> {
        self.state
            .svc
            .events(&self.run_id, after, PAGE)
            .unwrap_or_default()
    }

    fn is_terminal(&self) -> bool {
        // The run's own row only; see the note at connect.
        self.state
            .svc
            .store()
            .get_run(&self.run_id)
            .ok()
            .flatten()
            .map(|run| run.state.is_terminal())
            .unwrap_or(true)
    }
}

/// The run's state is read before its events. A run's final event is committed in
/// the same transaction as its terminal state, so a run seen as terminal already has
/// every event stored; reading the events second therefore cannot miss one. The other
/// order could see no new events, then see a run that finished in between, and end
/// the stream without its `finished` event.
fn poll_once(feed: &impl RunFeed, after: i64) -> Poll {
    let finished = feed.is_terminal();
    let batch = feed.events_after(after);
    if batch.is_empty() {
        Poll::Idle { finished }
    } else {
        Poll::Events(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A run that is still going when its events are read for the second time and
    /// finishes right after that read (the final event and the terminal state are
    /// committed together, as the store does).
    struct FinishesAfterSecondRead {
        reads: Cell<u32>,
    }

    impl FinishesAfterSecondRead {
        fn finished(&self) -> bool {
            self.reads.get() >= 2
        }
    }

    impl RunFeed for FinishesAfterSecondRead {
        fn events_after(&self, after: i64) -> Vec<EventRecord> {
            let finished_visible = self.finished();
            self.reads.set(self.reads.get() + 1);
            let all = [(1, "started"), (2, "finished")];
            all.iter()
                .filter(|(id, kind)| *id > after && (*kind != "finished" || finished_visible))
                .map(|(id, kind)| EventRecord {
                    id: *id,
                    run_id: "run".into(),
                    ts: 0,
                    kind: (*kind).into(),
                    payload_json: "{}".into(),
                })
                .collect()
        }

        fn is_terminal(&self) -> bool {
            self.finished()
        }
    }

    fn follow(feed: &impl RunFeed) -> Vec<String> {
        let (mut after, mut kinds) = (0, Vec::new());
        for _ in 0..10 {
            match poll_once(feed, after) {
                Poll::Events(batch) => {
                    after = batch.last().map_or(after, |e| e.id);
                    kinds.extend(batch.into_iter().map(|e| e.kind));
                }
                Poll::Idle { finished: true } => return kinds,
                Poll::Idle { finished: false } => {}
            }
        }
        panic!("stream never ended: {kinds:?}");
    }

    #[test]
    fn worker_ids_are_left_out_of_event_payloads_for_callers_below_run() {
        let started = r#"{"worker":"wrk_0123456789abcdef"}"#;
        assert_eq!(payload_for(started, true), started);
        assert_eq!(payload_for(started, false), "{}");
        let progress = r#"{"steps":5,"worker":"wrk_0123456789abcdef"}"#;
        assert_eq!(payload_for(progress, false), r#"{"steps":5}"#);
        let plain = r#"{"steps":5}"#;
        assert_eq!(payload_for(plain, false), plain);
        assert_eq!(payload_for("not json", false), "not json");
    }

    #[test]
    fn a_run_that_finishes_between_two_reads_still_delivers_its_final_event() {
        let feed = FinishesAfterSecondRead {
            reads: Cell::new(0),
        };
        assert_eq!(follow(&feed), ["started", "finished"]);
    }
}
