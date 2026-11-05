//! Connection-level limits: slow or idle connections are closed, the number of connections is capped, and long
//! responses (event streams) are not cut off by the read timeouts.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SHORT: Duration = Duration::from_millis(300);
/// How long a test waits for the server to close a connection before calling it left open.
const PATIENCE: Duration = Duration::from_secs(5);

fn boot(edit: impl FnOnce(&mut ServerConfig)) -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), Auth::open());
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    edit(&mut cfg);
    start(cfg, Service::new(store), engine).expect("server starts")
}

fn connect(addr: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(PATIENCE)).unwrap();
    s
}

/// Reads until the server closes the connection. Panics if it is still open after [`PATIENCE`].
fn read_until_closed(s: &mut TcpStream) -> String {
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::ConnectionReset => break,
            Err(e) => panic!(
                "connection still open after {PATIENCE:?} ({e}); read so far: {:?}",
                String::from_utf8_lossy(&got)
            ),
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

fn health(addr: SocketAddr) -> u16 {
    let mut s = connect(addr);
    s.write_all(b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let text = read_until_closed(&mut s);
    text.split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

#[test]
fn a_half_sent_request_head_is_dropped_after_the_header_timeout() {
    // Arrange
    let srv = boot(|c| c.header_read_timeout = SHORT);
    let mut s = connect(srv.addr());

    // Act: the request head never ends (no blank line).
    s.write_all(b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .unwrap();
    let t0 = Instant::now();
    let reply = read_until_closed(&mut s);

    // Assert: closed by the server, and never served.
    assert!(t0.elapsed() < PATIENCE, "{:?}", t0.elapsed());
    assert!(!reply.contains(" 200 "), "{reply}");
}

#[test]
fn a_connection_that_sends_nothing_is_closed_after_the_header_timeout() {
    let srv = boot(|c| c.header_read_timeout = SHORT);
    let mut s = connect(srv.addr());
    let reply = read_until_closed(&mut s);
    assert!(!reply.contains(" 200 "), "{reply}");
}

#[test]
fn an_idle_keep_alive_connection_is_closed_after_the_header_timeout() {
    let srv = boot(|c| c.header_read_timeout = SHORT);
    let mut s = connect(srv.addr());
    s.write_all(b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    let reply = read_until_closed(&mut s);
    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "first request served: {reply}"
    );
}

#[test]
fn a_request_body_that_stops_arriving_gets_a_408_and_the_connection_is_closed() {
    // Arrange
    let srv = boot(|c| c.body_read_timeout = SHORT);
    let mut s = connect(srv.addr());

    // Act: promise 100 bytes, send 10, then stall.
    s.write_all(b"POST /api/runs HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"target\":")
        .unwrap();
    let t0 = Instant::now();
    let reply = read_until_closed(&mut s);

    // Assert
    assert!(t0.elapsed() < PATIENCE, "{:?}", t0.elapsed());
    assert!(reply.starts_with("HTTP/1.1 408"), "{reply}");
    assert!(reply.contains("\"error\""), "JSON error body: {reply}");
}

#[test]
fn connections_beyond_the_cap_are_refused_until_one_closes() {
    // Arrange: room for two connections, both taken by clients that have not sent anything yet.
    let srv = boot(|c| c.max_connections = 2);
    let idle_one = connect(srv.addr());
    let _idle_two = connect(srv.addr());
    std::thread::sleep(Duration::from_millis(200)); // let the server accept both

    // Act
    let mut third = connect(srv.addr());
    let refused = read_until_closed(&mut third);

    // Assert: told the server is busy, with a time to come back, and never served.
    assert!(refused.starts_with("HTTP/1.1 503"), "{refused}");
    assert!(refused.contains("retry-after: 1"), "{refused}");

    // A slot frees up when a connection closes.
    drop(idle_one);
    let deadline = Instant::now() + PATIENCE;
    while health(srv.addr()) != 200 {
        assert!(Instant::now() < deadline, "no slot came free");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn limits_that_would_refuse_everything_are_refused_at_start() {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let attempt = |edit: &dyn Fn(&mut ServerConfig)| {
        let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
        let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), Auth::open());
        cfg.embedded_workers = 0;
        edit(&mut cfg);
        start(cfg, Service::new(store), engine.clone()).err()
    };
    assert!(attempt(&|c| c.max_connections = 0).is_some());
    assert!(attempt(&|c| c.max_replays = 0).is_some());
    assert!(attempt(&|c| c.max_replays = hairspring_server::MAX_MAX_REPLAYS + 1).is_some());
    assert!(attempt(&|c| c.max_replays = hairspring_server::MAX_MAX_REPLAYS).is_none());
    assert!(attempt(&|c| c.header_read_timeout = Duration::ZERO).is_some());
    assert!(attempt(&|c| c.body_read_timeout = Duration::ZERO).is_some());
    assert!(attempt(&|_| {}).is_none());
}

fn post_json(addr: SocketAddr, path: &str, body: &Value) -> Value {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let text = agent
        .post(format!("http://{addr}{path}"))
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

fn run_state(addr: SocketAddr, id: &str) -> String {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let text = agent
        .get(format!("http://{addr}/api/runs/{id}"))
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    v["run"]["state"].as_str().unwrap_or("").to_string()
}

#[test]
fn an_event_stream_keeps_streaming_well_past_the_read_timeouts() {
    // Arrange: a long run and short read timeouts.
    let srv = boot(|c| {
        c.embedded_workers = 1;
        c.header_read_timeout = SHORT;
        c.body_read_timeout = SHORT;
    });
    let addr = srv.addr();
    let run = post_json(
        addr,
        "/api/runs",
        &json!({"target": "pubsub-atomic", "seed": 1, "budget": {"max_steps": 40000000, "max_trials": 1000, "wall_secs": 30}}),
    );
    let id = run["run"]["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(30);
    while run_state(addr, &id) != "running" {
        assert!(Instant::now() < deadline, "run never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut s = connect(addr);
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(
        format!("GET /api/runs/{id}/events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes(),
    )
    .unwrap();

    // Act: hold the stream open for several timeouts, then end the run.
    std::thread::sleep(SHORT * 5);
    post_json(addr, &format!("/api/runs/{id}/cancel"), &json!({}));
    let text = read_until_closed_within(&mut s, Duration::from_secs(30));

    // Assert: the whole stream arrived, including its chunked end.
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.contains("event: started"), "{text}");
    assert!(text.ends_with("0\r\n\r\n"), "stream was cut off: {text}");
}

/// Like [`read_until_closed`], for a response the server ends itself: stops at the chunked terminator.
fn read_until_closed_within(s: &mut TcpStream, limit: Duration) -> String {
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if got.ends_with(b"0\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

#[test]
fn every_connection_over_the_cap_is_told_the_server_is_busy() {
    // Arrange: one slot, held by an idle client.
    let srv = boot(|c| c.max_connections = 1);
    let _idle = connect(srv.addr());
    std::thread::sleep(Duration::from_millis(200));

    // Act: several refused clients that have each already sent a request.
    let clients: Vec<TcpStream> = (0..8)
        .map(|_| {
            let mut s = connect(srv.addr());
            s.write_all(b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .unwrap();
            s
        })
        .collect();
    let replies: Vec<String> = clients
        .into_iter()
        .map(|mut s| read_until_closed(&mut s))
        .collect();

    // Assert
    for r in &replies {
        assert!(r.starts_with("HTTP/1.1 503"), "{r:?}");
    }
}

const TOKEN: &str = "run-token-0123456789abcdef";

fn boot_with_tokens() -> RunningServer {
    boot(|c| c.auth = Auth::from_spec(&format!("{TOKEN}:run")).unwrap())
}

/// Sends a request head that promises a large body and asks to be told to go ahead first, then reads the reply
/// without sending any of the body.
fn head_promising_a_large_body(addr: SocketAddr, authorization: Option<&str>) -> String {
    let mut s = connect(addr);
    let auth = authorization
        .map(|a| format!("Authorization: {a}\r\n"))
        .unwrap_or_default();
    s.write_all(
        format!(
            "POST /api/runs HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n{auth}\
Content-Length: 2000000\r\nExpect: 100-continue\r\n\r\n"
        )
        .as_bytes(),
    )
    .unwrap();
    let mut buf = [0u8; 4096];
    let n = s.read(&mut buf).expect("a reply without sending the body");
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

#[test]
fn a_wrong_or_missing_token_gets_401_before_the_body_is_sent() {
    // Arrange
    let srv = boot_with_tokens();

    for auth in [None, Some("Bearer wrong-token-0123456789abcdef")] {
        // Act
        let reply = head_promising_a_large_body(srv.addr(), auth);

        // Assert: refused at once, not told to go ahead and send the body.
        assert!(reply.starts_with("HTTP/1.1 401"), "{auth:?}: {reply}");
        assert!(!reply.contains("100 Continue"), "{auth:?}: {reply}");
    }
}

#[test]
fn a_valid_token_is_told_to_send_its_body() {
    let srv = boot_with_tokens();
    let reply = head_promising_a_large_body(srv.addr(), Some(&format!("Bearer {TOKEN}")));
    assert!(reply.starts_with("HTTP/1.1 100 Continue"), "{reply}");
}
