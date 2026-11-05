//! Per-request limits that sit below the connection limits in [`crate::conn`]: a deadline for reading the request
//! body, and a cap on event streams per caller.
use crate::AppState;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{ready, Context, Poll};
use tokio::time::{Instant, Sleep};

/// Gives a request body a deadline, counted from when the request reaches this layer (just after its head has
/// arrived). Nothing is read here: the body is still read by the handler's own extractor, after the token check, and
/// a request that is refused earlier, or whose handler takes no body, never has its body read at all. A body that is
/// still arriving when the deadline passes gets a 408 and the connection is closed. The deadline covers the body
/// only, never the handler or the response.
pub(crate) async fn time_body(State(s): State<AppState>, req: Request, next: Next) -> Response {
    if req.body().is_end_stream() {
        return next.run(req).await;
    }
    let expired = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + s.body_read_timeout;
    let flag = expired.clone();
    let req = req.map(|inner| {
        Body::new(DeadlineBody {
            inner,
            deadline,
            sleep: None,
            expired: flag,
        })
    });
    let res = next.run(req).await;
    if expired.load(Ordering::SeqCst) {
        return body_timed_out();
    }
    res
}

fn body_timed_out() -> Response {
    let mut res = crate::error::ApiError::new(
        StatusCode::REQUEST_TIMEOUT,
        "the request body did not arrive in time",
    )
    .into_response();
    res.headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    res
}

/// A request body that fails once its deadline has passed, and records that it did. The timer is started only when
/// the body is first waited for, so a body that is already there, or is never read, costs no timer.
struct DeadlineBody {
    inner: Body,
    deadline: Instant,
    sleep: Option<Pin<Box<Sleep>>>,
    expired: Arc<AtomicBool>,
}

#[derive(Debug)]
struct BodyTimedOut;

impl std::fmt::Display for BodyTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the request body did not arrive in time")
    }
}

impl std::error::Error for BodyTimedOut {}

impl HttpBody for DeadlineBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = self.get_mut();
        if let Poll::Ready(frame) = Pin::new(&mut this.inner).poll_frame(cx) {
            return Poll::Ready(frame);
        }
        let deadline = this.deadline;
        let sleep = this
            .sleep
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        ready!(sleep.as_mut().poll(cx));
        this.expired.store(true, Ordering::SeqCst);
        Poll::Ready(Some(Err(axum::Error::new(BodyTimedOut))))
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// Counts open event streams per caller, so one caller cannot hold every slot of the server-wide cap.
pub struct CallerStreams {
    max_per_caller: usize,
    open: Mutex<HashMap<String, usize>>,
}

/// One open stream for one caller; counted until dropped.
pub(crate) struct CallerStreamSlot {
    owner: Arc<CallerStreams>,
    key: String,
}

impl CallerStreams {
    pub(crate) fn new(max_per_caller: usize) -> CallerStreams {
        CallerStreams {
            max_per_caller,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// A slot for `key` if it has fewer than the cap open. An entry exists only while its caller has a stream
    /// open, so the table never holds more entries than there are open streams.
    pub(crate) fn try_acquire(self: &Arc<Self>, key: String) -> Option<CallerStreamSlot> {
        let mut open = self.open.lock().unwrap_or_else(|p| p.into_inner());
        let count = open.entry(key.clone()).or_insert(0);
        if *count >= self.max_per_caller {
            if *count == 0 {
                open.remove(&key);
            }
            return None;
        }
        *count += 1;
        Some(CallerStreamSlot {
            owner: self.clone(),
            key,
        })
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.open.lock().map(|m| m.len()).unwrap_or(0)
    }
}

impl Drop for CallerStreamSlot {
    fn drop(&mut self) {
        let mut open = self.owner.open.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(count) = open.get_mut(&self.key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_caller_has_its_own_cap_and_slots_come_back_on_drop() {
        let streams = Arc::new(CallerStreams::new(2));
        let a1 = streams.try_acquire("a".into()).unwrap();
        let _a2 = streams.try_acquire("a".into()).unwrap();
        assert!(streams.try_acquire("a".into()).is_none());
        let _b = streams
            .try_acquire("b".into())
            .expect("another caller is unaffected");
        drop(a1);
        assert!(streams.try_acquire("a".into()).is_some());
    }

    #[test]
    fn the_table_forgets_callers_with_nothing_open() {
        let streams = Arc::new(CallerStreams::new(1));
        let a = streams.try_acquire("a".into()).unwrap();
        assert_eq!(streams.tracked(), 1);
        drop(a);
        assert_eq!(streams.tracked(), 0);
        let none = Arc::new(CallerStreams::new(0));
        assert!(none.try_acquire("x".into()).is_none());
        assert_eq!(none.tracked(), 0);
    }
}
