//! Where request bodies are read: never before the token check, the open-mode guard or the rate limit, never on
//! routes that take no body, and always within the body deadline.
use crate::tests::{state_with, LIMIT};
use crate::{router, AppState, Auth};
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use futures_util::stream::{self, StreamExt};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

const RUN_TOKEN: &str = "run-token-0123456789abcdef";
/// The size a body chunk is handed to the server in, and how many make up one large body (just under 2 MiB).
const CHUNK: usize = 64 * 1024;
const CHUNKS: usize = 31;
/// Long enough for a healthy handler, short enough for a test.
const SHORT: Duration = Duration::from_millis(300);

fn tokens() -> Auth {
    Auth::from_spec(&format!("{RUN_TOKEN}:run")).unwrap()
}

/// A large body that counts every byte the server pulls from it.
fn counted_body(read: &Arc<AtomicUsize>) -> Body {
    let read = read.clone();
    Body::from_stream(stream::iter(0..CHUNKS).map(move |_| {
        read.fetch_add(CHUNK, Ordering::SeqCst);
        Ok::<_, Infallible>(Bytes::from(vec![b' '; CHUNK]))
    }))
}

/// A body that sends the start of a JSON document and then nothing more, without ending.
fn stalled_body() -> Body {
    let start = stream::once(async { Ok::<_, Infallible>(Bytes::from_static(b"{\"target\":")) });
    Body::from_stream(start.chain(stream::pending()))
}

fn request(method: &str, path: &str, host: &str, token: Option<&str>, body: Body) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, host)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let mut req = b.body(body).unwrap();
    let peer: SocketAddr = "127.0.0.1:4000".parse().unwrap();
    req.extensions_mut().insert(ConnectInfo(peer));
    req
}

async fn send(state: &AppState, req: Request<Body>) -> Response {
    router(state.clone()).oneshot(req).await.unwrap()
}

#[tokio::test]
async fn a_missing_or_wrong_token_is_refused_before_any_of_the_body_is_read() {
    // Arrange
    let state = state_with(Duration::from_secs(60), tokens());
    let read = Arc::new(AtomicUsize::new(0));

    for token in [None, Some("wrong-token-0123456789abcdef")] {
        // Act
        let res = send(
            &state,
            request("POST", "/api/runs", "127.0.0.1", token, counted_body(&read)),
        )
        .await;

        // Assert
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{token:?}");
    }
    assert_eq!(read.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn many_refused_large_bodies_are_never_read() {
    // Arrange: a proxy for memory use. The server holds a request's body only once it reads it, so if no byte of
    // these bodies is read, none of them is held, however many arrive.
    let mut state = state_with(Duration::from_secs(60), tokens());
    state.limiter = Arc::new(crate::RateLimiter::new(0));
    let read = Arc::new(AtomicUsize::new(0));

    // Act
    for i in 0..200 {
        let path = ["/api/runs", "/api/replay", "/mcp", "/api/worker/register"][i % 4];
        let res = send(
            &state,
            request(
                "POST",
                path,
                "127.0.0.1",
                Some("wrong-token-0123456789abcdef"),
                counted_body(&read),
            ),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{path}");
    }

    // Assert
    assert_eq!(read.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_rate_limited_request_is_refused_before_its_body_is_read() {
    // Arrange: the client address has used up its budget.
    let state = state_with(Duration::from_secs(60), tokens());
    for _ in 0..=LIMIT {
        send(
            &state,
            request(
                "GET",
                "/api/me",
                "127.0.0.1",
                Some("wrong-guess"),
                Body::empty(),
            ),
        )
        .await;
    }
    let read = Arc::new(AtomicUsize::new(0));

    // Act
    let res = send(
        &state,
        request(
            "POST",
            "/api/runs",
            "127.0.0.1",
            Some(RUN_TOKEN),
            counted_body(&read),
        ),
    )
    .await;

    // Assert
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(read.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_open_mode_guard_refuses_before_the_body_is_read() {
    // Arrange
    let state = state_with(Duration::from_secs(60), Auth::open());
    let read = Arc::new(AtomicUsize::new(0));

    // Act: a Host that is not loopback (DNS rebinding).
    let res = send(
        &state,
        request(
            "POST",
            "/api/runs",
            "attacker.example",
            None,
            counted_body(&read),
        ),
    )
    .await;

    // Assert
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(read.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn routes_that_take_no_body_never_read_one() {
    // Arrange: health and the web UI are outside the rate limit; neither takes a body.
    let state = state_with(Duration::from_secs(60), tokens());
    let read = Arc::new(AtomicUsize::new(0));

    for path in ["/api/health", "/", "/runs"] {
        // Act
        let res = send(
            &state,
            request("GET", path, "127.0.0.1", None, counted_body(&read)),
        )
        .await;

        // Assert
        assert!(res.status().is_success(), "{path}: {}", res.status());
    }
    assert_eq!(read.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_body_that_stops_arriving_gets_a_408_and_the_connection_is_closed() {
    // Arrange
    let mut state = state_with(Duration::from_secs(60), tokens());
    state.body_read_timeout = SHORT;

    // Act
    let res = tokio::time::timeout(
        SHORT * 10,
        send(
            &state,
            request(
                "POST",
                "/api/runs",
                "127.0.0.1",
                Some(RUN_TOKEN),
                stalled_body(),
            ),
        ),
    )
    .await
    .expect("answered within the deadline, not left waiting");

    // Assert
    assert_eq!(res.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(res.headers().get(header::CONNECTION).unwrap(), "close");
    let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["error"].is_string(), "{v}");
}

#[tokio::test]
async fn a_body_that_arrives_in_time_reaches_the_handler() {
    // Arrange
    let mut state = state_with(Duration::from_secs(60), tokens());
    state.body_read_timeout = SHORT;
    let chunks =
        stream::iter([&b"{\"target\":"[..], &b"\"no-such-target\"}"[..]]).then(|c| async move {
            tokio::time::sleep(SHORT / 4).await;
            Ok::<_, Infallible>(Bytes::from_static(c))
        });

    // Act
    let res = send(
        &state,
        request(
            "POST",
            "/api/runs",
            "127.0.0.1",
            Some(RUN_TOKEN),
            Body::from_stream(chunks),
        ),
    )
    .await;

    // Assert: the handler judged the body (unknown target), and the deadline did not cut in.
    assert_ne!(res.status(), StatusCode::REQUEST_TIMEOUT);
    assert!(res.status().is_client_error(), "{}", res.status());
    assert!(res.headers().get(header::CONNECTION).is_none());
}

#[tokio::test]
async fn an_oversized_body_is_still_refused_with_413() {
    let state = state_with(Duration::from_secs(60), tokens());
    let big = Body::from(vec![b' '; crate::MAX_BODY_BYTES + 1]);
    let res = send(
        &state,
        request("POST", "/api/runs", "127.0.0.1", Some(RUN_TOKEN), big),
    )
    .await;
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
