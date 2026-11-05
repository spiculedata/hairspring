//! The accept loop: HTTP/1.1 connections, a cap on how many are open at once, and a deadline for each request head.
//!
//! The head deadline runs whenever the server waits for a request: on a new connection and between requests on a
//! kept-alive one. It does not run while a response is being produced or sent, so a long replay or an event stream
//! is not cut off by it. Request bodies have their own deadline ([`crate::limits::time_body`]).
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::Router;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt;

/// How long to back off after an accept error that is not about one connection (for example, out of descriptors).
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);
/// What a refused connection is told to wait before trying again, in seconds.
const BUSY_RETRY_AFTER_SECS: u64 = 1;
const BUSY_BODY: &str = "{\"error\":\"too many open connections\"}";
/// Refused connections that may be in the middle of being told the server is busy at once. Beyond this a refused
/// connection is closed without a reply, so a flood of connections cannot make refusing them cost more.
pub(crate) const MAX_REFUSALS: usize = 128;
/// How long one refused connection may take, in all, to be told the server is busy and closed.
const REFUSAL_TIME: Duration = Duration::from_secs(1);
/// At most this much of a refused connection's request is read, and thrown away, before it is closed.
const REFUSAL_DRAIN_BYTES: usize = 16 * 1024;

pub(crate) struct ConnLimits {
    pub header_read_timeout: Duration,
    pub max_connections: usize,
    /// Refused connections that may be being told the server is busy at once ([`MAX_REFUSALS`] when served).
    pub max_refusals: usize,
}

/// Serves `app` until `shutdown` resolves, then lets open connections finish their current request and returns once
/// every connection has closed.
pub(crate) async fn serve(
    listener: TcpListener,
    app: Router,
    limits: ConnLimits,
    shutdown: impl Future<Output = ()>,
) {
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    let refusals = Arc::new(Semaphore::new(limits.max_refusals));
    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(conn) => conn,
                Err(e) => {
                    back_off_after(e).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            refuse(stream, &refusals);
            continue;
        };
        tokio::spawn(serve_connection(
            stream,
            peer,
            app.clone(),
            limits.header_read_timeout,
            stop_rx.clone(),
            permit,
        ));
    }
    drop(listener);
    let _ = stop_tx.send(true);
    // Every connection holds a slot until it closes, so taking them all waits for the last one.
    let all = u32::try_from(limits.max_connections).unwrap_or(u32::MAX);
    let _ = slots.acquire_many(all).await;
}

async fn serve_connection(
    stream: TcpStream,
    peer: SocketAddr,
    app: Router,
    header_read_timeout: Duration,
    mut stop: watch::Receiver<bool>,
    _slot: OwnedSemaphorePermit,
) {
    let service = hyper::service::service_fn(move |mut req: hyper::Request<Incoming>| {
        req.extensions_mut().insert(ConnectInfo(peer));
        app.clone().oneshot(req.map(Body::new))
    });
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(header_read_timeout);
    let conn = builder.serve_connection(TokioIo::new(stream), service);
    tokio::pin!(conn);
    tokio::select! {
        _ = conn.as_mut() => return,
        _ = stop.wait_for(|stopping| *stopping) => {}
    }
    conn.as_mut().graceful_shutdown();
    let _ = conn.await;
}

/// Over the cap: answer 503 and close, in a short task of its own so the accept loop never waits on a client. At most
/// [`MAX_REFUSALS`] such tasks run at once, each for at most [`REFUSAL_TIME`]; past that the connection is closed
/// without a reply. Returns whether a reply is being sent.
fn refuse(stream: TcpStream, refusals: &Arc<Semaphore>) -> bool {
    let Ok(permit) = refusals.clone().try_acquire_owned() else {
        return false;
    };
    tokio::spawn(async move {
        let _ = tokio::time::timeout(REFUSAL_TIME, tell_busy(stream)).await;
        drop(permit);
    });
    true
}

async fn tell_busy(mut stream: TcpStream) -> std::io::Result<()> {
    stream.write_all(busy_response().as_bytes()).await?;
    stream.shutdown().await?;
    // Read what the client has sent before closing: closing with unread data resets the connection, and the client
    // may then lose the reply before it reads it.
    let mut buf = [0u8; 4096];
    let mut left = REFUSAL_DRAIN_BYTES;
    while left > 0 {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        left = left.saturating_sub(n);
    }
    Ok(())
}

fn busy_response() -> String {
    format!(
        "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\nretry-after: {BUSY_RETRY_AFTER_SECS}\r\n\
connection: close\r\ncontent-length: {}\r\n\r\n{BUSY_BODY}",
        BUSY_BODY.len()
    )
}

/// An error about one connection (reset before it was accepted) needs nothing; anything else (no descriptors left)
/// would repeat at once, so wait a moment.
async fn back_off_after(e: std::io::Error) {
    use std::io::ErrorKind::{ConnectionAborted, ConnectionRefused, ConnectionReset};
    if matches!(
        e.kind(),
        ConnectionAborted | ConnectionRefused | ConnectionReset
    ) {
        return;
    }
    eprintln!("hairspring-server: cannot accept a connection: {e}");
    tokio::time::sleep(ACCEPT_BACKOFF).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pair() -> (TcpStream, std::net::TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (server, client)
    }

    fn read_all(mut client: std::net::TcpStream) -> String {
        use std::io::Read;
        let mut got = String::new();
        let _ = client.read_to_string(&mut got);
        got
    }

    #[tokio::test]
    async fn a_refused_connection_is_answered_503_and_its_slot_comes_back() {
        // Arrange
        let refusals = Arc::new(Semaphore::new(1));
        let (server, client) = pair().await;

        // Act
        let replying = refuse(server, &refusals);
        let reply = tokio::task::spawn_blocking(move || read_all(client))
            .await
            .unwrap();

        // Assert
        assert!(replying);
        assert!(reply.starts_with("HTTP/1.1 503 "), "{reply:?}");
        let deadline = tokio::time::Instant::now() + REFUSAL_TIME * 3;
        while refusals.available_permits() != 1 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "permit not returned"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn refusals_beyond_the_bound_are_closed_without_a_reply() {
        // Arrange: no refusal slots free.
        let refusals = Arc::new(Semaphore::new(0));
        let (server, client) = pair().await;

        // Act
        let replying = refuse(server, &refusals);
        let reply = tokio::task::spawn_blocking(move || read_all(client))
            .await
            .unwrap();

        // Assert
        assert!(!replying);
        assert_eq!(reply, "");
    }

    #[tokio::test]
    async fn a_refused_client_that_never_reads_or_closes_is_let_go_in_time() {
        // Arrange: a client that keeps its connection open and sends nothing more.
        let refusals = Arc::new(Semaphore::new(1));
        let (server, _client) = pair().await;

        // Act
        assert!(refuse(server, &refusals));

        // Assert: the refusal slot comes back within the refusal time.
        let deadline = tokio::time::Instant::now() + REFUSAL_TIME * 3;
        while refusals.available_permits() != 1 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "refusal never ended"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn no_more_than_the_bound_of_refused_connections_are_replied_to_at_once() {
        use std::io::Read;

        // Arrange: one connection allowed, held by an idle client; two refusals may be in progress at once.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let limits = ConnLimits {
            header_read_timeout: Duration::from_secs(30),
            max_connections: 1,
            max_refusals: 2,
        };
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, Router::new(), limits, async {
            let _ = stopped.await;
        }));
        let idle = std::net::TcpStream::connect(addr).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Act: refused clients that neither read nor close, so each refusal in progress lasts its full time.
        let replies = tokio::task::spawn_blocking(move || {
            let clients: Vec<std::net::TcpStream> = (0..5)
                .map(|_| {
                    let c = std::net::TcpStream::connect(addr).unwrap();
                    std::thread::sleep(Duration::from_millis(20));
                    c
                })
                .collect();
            clients
                .into_iter()
                .map(|mut c| {
                    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut buf = [0u8; 64];
                    let n = c.read(&mut buf).unwrap_or(0);
                    String::from_utf8_lossy(&buf[..n]).into_owned()
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();

        // Assert
        let busy = replies
            .iter()
            .filter(|r| r.starts_with("HTTP/1.1 503"))
            .count();
        assert_eq!(busy, 2, "{replies:?}");
        assert_eq!(
            replies.iter().filter(|r| r.is_empty()).count(),
            3,
            "{replies:?}"
        );
        drop(idle);
        let _ = stop.send(());
        let _ = server.await;
    }

    #[test]
    fn the_busy_response_is_well_formed() {
        let text = busy_response();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 503 "));
        assert!(head.contains(&format!("content-length: {}", body.len())));
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        assert!(v["error"].is_string());
    }
}
