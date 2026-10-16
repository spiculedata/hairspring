//! Server-sent events for a run: stored events are replayed, then new ones follow
//! until the run is finished (or the server stops).
use crate::api::blocking;
use crate::auth::Caller;
use crate::error::ApiError;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use hairspring_mcp::validate;
use hairspring_service::Role;
use hairspring_store::EventRecord;
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
    })
    .flatten();
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
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
        self.state
            .svc
            .get_run(&self.run_id)
            .ok()
            .flatten()
            .map(|d| d.run.state.is_terminal())
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
    fn a_run_that_finishes_between_two_reads_still_delivers_its_final_event() {
        let feed = FinishesAfterSecondRead {
            reads: Cell::new(0),
        };
        assert_eq!(follow(&feed), ["started", "finished"]);
    }
}
