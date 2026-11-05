//! The accept loop: HTTP/1.1 connections, a cap on how many are open at once, and a deadline for each request head.
//!
//! The head deadline runs whenever the server waits for a request: on a new connection and between requests on a
//! kept-alive one. It does not run while a response is being produced or sent, so a long replay or an event stream
//! is not cut off by it. Request bodies have their own deadline ([`crate::limits::read_body_in_time`]).
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
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt;

/// How long to back off after an accept error that is not about one connection (for example, out of descriptors).
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);
/// What a refused connection is told to wait before trying again, in seconds.
const BUSY_RETRY_AFTER_SECS: u64 = 1;
const BUSY_BODY: &str = "{\"error\":\"too many open connections\"}";

pub(crate) struct ConnLimits {
    pub header_read_timeout: Duration,
    pub max_connections: usize,
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
            refuse(&stream);
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

/// Over the cap: say so if the socket will take it without waiting, then close. Nothing is spawned or awaited, so
/// refusing costs no more than the accept did.
fn refuse(stream: &TcpStream) {
    let _ = stream.try_write(busy_response().as_bytes());
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
