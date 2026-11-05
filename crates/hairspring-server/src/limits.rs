//! Per-request limits that sit below the connection limits in [`crate::conn`]: a deadline for reading the request
//! body, and a cap on event streams per caller.
use crate::AppState;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Reads the whole request body (at most [`crate::MAX_BODY_BYTES`]) within the configured deadline before the
/// handler runs. A body that does not arrive in time gets a 408 and the connection is closed; one that is too large
/// gets a 413. The deadline covers the body only, never the handler or the response.
pub(crate) async fn read_body_in_time(
    State(s): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let (parts, body) = req.into_parts();
    let read = axum::body::to_bytes(body, crate::MAX_BODY_BYTES);
    match tokio::time::timeout(s.body_read_timeout, read).await {
        Ok(Ok(bytes)) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        Ok(Err(e)) if is_too_large(&e) => {
            error(StatusCode::PAYLOAD_TOO_LARGE, "request body too large")
        }
        Ok(Err(_)) => error(
            StatusCode::BAD_REQUEST,
            "the request body could not be read",
        ),
        Err(_) => {
            let mut res = error(
                StatusCode::REQUEST_TIMEOUT,
                "the request body did not arrive in time",
            );
            res.headers_mut()
                .insert(header::CONNECTION, HeaderValue::from_static("close"));
            res
        }
    }
}

fn is_too_large(e: &axum::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = source {
        if err.is::<http_body_util::LengthLimitError>() {
            return true;
        }
        source = err.source();
    }
    false
}

fn error(status: StatusCode, message: &str) -> Response {
    crate::error::ApiError::new(status, message).into_response()
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
