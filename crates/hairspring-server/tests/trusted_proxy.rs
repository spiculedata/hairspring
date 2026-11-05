//! Trusted proxies, over real HTTP. The tests connect from 127.0.0.1, which stands in for a reverse proxy: when it is
//! trusted, `X-Forwarded-For` names the client a request is counted under; when it is not, the header is ignored and
//! everything is counted under 127.0.0.1, as before (see also `rate_limit.rs`).
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig, TrustedProxies};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

const READ: &str = "read-token-0123456789abcdef";
const LIMIT: u32 = 2;
const GUESSES: usize = 6;
const LOOPBACK_PROXY: &str = "127.0.0.1";
const PATIENCE: Duration = Duration::from_secs(10);

fn boot(auth: Auth, trusted: &str, edit: impl FnOnce(&mut ServerConfig)) -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = LIMIT;
    cfg.trusted_proxies = TrustedProxies::parse(trusted).unwrap();
    edit(&mut cfg);
    start(cfg, Service::new(store), engine).expect("server starts")
}

fn boot_with_token(trusted: &str) -> RunningServer {
    boot(
        Auth::from_spec(&format!("{READ}:read")).unwrap(),
        trusted,
        |_| {},
    )
}

/// Sends one request with exactly the given extra header lines and returns the status, reading only the response
/// head (an event stream does not end by itself). The connection is returned so a test can hold it open.
fn send(addr: SocketAddr, path: &str, headers: &[&str]) -> (u16, TcpStream) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(PATIENCE)).unwrap();
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    for h in headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut got = Vec::new();
    let mut byte = [0u8; 1];
    while !got.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => got.push(byte[0]),
            Err(e) if e.kind() == ErrorKind::ConnectionReset => break,
            Err(e) => panic!("no response head: {e}"),
        }
    }
    let text = String::from_utf8_lossy(&got).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status line: {text:?}"));
    (status, s)
}

fn status(addr: SocketAddr, path: &str, headers: &[&str]) -> u16 {
    send(addr, path, headers).0
}

fn bearer(token: &str) -> String {
    format!("Authorization: Bearer {token}")
}

fn xff(value: &str) -> String {
    format!("X-Forwarded-For: {value}")
}

/// Spends one client's anonymous budget with wrong tokens; `forwarded` is that client's `X-Forwarded-For`, if any.
fn flood(addr: SocketAddr, forwarded: Option<&str>) -> Vec<u16> {
    (0..GUESSES)
        .map(|i| {
            let wrong = bearer(&format!("wrong-guess-number-{i:04}-abcdef"));
            let fwd = forwarded.map(xff);
            let mut headers = vec![wrong.as_str()];
            headers.extend(fwd.as_deref());
            status(addr, "/api/targets", &headers)
        })
        .collect()
}

fn flooded() -> Vec<u16> {
    let mut want = vec![401; LIMIT as usize];
    want.extend(vec![429; GUESSES - LIMIT as usize]);
    want
}

fn token_from(addr: SocketAddr, forwarded: &str) -> u16 {
    status(addr, "/api/targets", &[&bearer(READ), &xff(forwarded)])
}

#[test]
fn behind_a_trusted_proxy_one_clients_flood_does_not_lock_out_another_clients_token() {
    // Arrange
    let srv = boot_with_token(LOOPBACK_PROXY);

    // Act: one client behind the proxy uses up its anonymous budget.
    assert_eq!(flood(srv.addr(), Some("198.51.100.1")), flooded());

    // Assert: another client's valid token is let through, the flooding client's is still refused.
    assert_eq!(token_from(srv.addr(), "198.51.100.2"), 200);
    assert_eq!(token_from(srv.addr(), "198.51.100.1"), 429);
}

#[test]
fn without_trusted_proxies_the_flood_locks_out_every_token_through_the_proxy() {
    // The behaviour with no trusted proxy, unchanged: every request is counted under the connecting address.
    let srv = boot_with_token("");
    assert_eq!(flood(srv.addr(), Some("198.51.100.1")), flooded());
    assert_eq!(token_from(srv.addr(), "198.51.100.2"), 429);
}

#[test]
fn forwarding_headers_from_a_peer_that_is_not_trusted_are_ignored() {
    // Arrange: a proxy is trusted, but it is not the address these requests come from.
    let srv = boot_with_token("192.0.2.10, 2001:db8::/64");

    // Act: each guess claims a different client address.
    let statuses: Vec<u16> = (0..GUESSES)
        .map(|i| {
            let wrong = bearer(&format!("wrong-guess-number-{i:04}-abcdef"));
            status(
                srv.addr(),
                "/api/targets",
                &[&wrong, &xff(&format!("198.51.100.{i}"))],
            )
        })
        .collect();

    // Assert: all were counted under the real peer address.
    assert_eq!(statuses, flooded());
    assert_eq!(token_from(srv.addr(), "198.51.100.200"), 429);
}

#[test]
fn chained_trusted_proxies_are_skipped_to_find_the_client() {
    let srv = boot_with_token("127.0.0.1, 10.0.0.0/8");
    // The client claims to be 192.0.2.1 (left-most); the edge proxy 10.1.1.1 saw 198.51.100.1.
    assert_eq!(
        flood(srv.addr(), Some("192.0.2.1, 198.51.100.1, 10.1.1.1")),
        flooded()
    );
    assert_eq!(token_from(srv.addr(), "198.51.100.1, 10.1.1.1"), 429);
    assert_eq!(
        token_from(srv.addr(), "192.0.2.1, 198.51.100.2, 10.1.1.1"),
        200
    );
}

#[test]
fn a_header_that_cannot_be_used_counts_the_request_under_the_proxy() {
    // Arrange: the proxy's own address has used up its anonymous budget (requests with no forwarded address).
    let srv = boot_with_token(LOOPBACK_PROXY);
    assert_eq!(flood(srv.addr(), None), flooded());
    let too_many: String = (0..17)
        .map(|i| format!("198.51.100.{i}"))
        .collect::<Vec<_>>()
        .join(",");
    // Two addresses padded past the length bound (HTTP strips spaces at the ends of a value, not inside it).
    let too_long = format!("198.51.100.8,{}198.51.100.9", " ".repeat(1100));

    // Act and assert: each falls back to the proxy's address, and is refused with it.
    for bad in [
        vec![xff("unknown")],
        vec![xff("198.51.100.9:443")],
        vec![xff(&too_many)],
        vec![xff(&too_long)],
        vec![xff("198.51.100.9"), xff("not-an-address")],
        vec![xff("127.0.0.1")],
        vec!["Forwarded: for=198.51.100.9".to_string()],
        vec!["X-Real-IP: 198.51.100.9".to_string()],
    ] {
        let b = bearer(READ);
        let mut headers: Vec<&str> = vec![&b];
        headers.extend(bad.iter().map(String::as_str));
        assert_eq!(status(srv.addr(), "/api/targets", &headers), 429, "{bad:?}");
    }
    // A usable header still works.
    assert_eq!(token_from(srv.addr(), "198.51.100.9"), 200);
}

#[test]
fn several_forwarded_headers_are_read_as_one_list() {
    let srv = boot_with_token(LOOPBACK_PROXY);
    assert_eq!(flood(srv.addr(), Some("198.51.100.1")), flooded());
    let b = bearer(READ);
    // The right-most entry across both headers is the flooding client.
    let headers = [
        b.as_str(),
        "X-Forwarded-For: 198.51.100.2",
        "X-Forwarded-For: 198.51.100.1",
    ];
    assert_eq!(status(srv.addr(), "/api/targets", &headers), 429);
    let headers = [
        b.as_str(),
        "X-Forwarded-For: 198.51.100.1",
        "X-Forwarded-For: 198.51.100.2",
    ];
    assert_eq!(status(srv.addr(), "/api/targets", &headers), 200);
}

#[test]
fn forwarded_ipv6_clients_are_counted_by_their_64() {
    let srv = boot_with_token(LOOPBACK_PROXY);
    assert_eq!(flood(srv.addr(), Some("2001:db8:1:2::1")), flooded());
    assert_eq!(token_from(srv.addr(), "2001:db8:1:2:ffff::99"), 429);
    assert_eq!(token_from(srv.addr(), "2001:db8:1:3::1"), 200);
}

#[test]
fn health_with_a_token_counts_the_forwarded_client() {
    let srv = boot(
        Auth::from_spec(&format!("{READ}:read")).unwrap(),
        LOOPBACK_PROXY,
        |c| c.organisation = Some("Acme Ltd".to_string()),
    );
    assert_eq!(flood(srv.addr(), Some("198.51.100.1")), flooded());
    let named = |fwd: &str| {
        let (_, mut s) = send(
            srv.addr(),
            "/api/health",
            &[&bearer(READ), &xff(fwd), "Connection: close"],
        );
        let mut body = String::new();
        let _ = s.read_to_string(&mut body);
        body.contains("Acme Ltd")
    };
    assert!(!named("198.51.100.1"));
    assert!(named("198.51.100.2"));
}

fn create_run(addr: SocketAddr) -> String {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let text = agent
        .post(format!("http://{addr}/api/runs"))
        .header("Content-Type", "application/json")
        .send(r#"{"target": "pubsub-atomic", "seed": 1}"#)
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["run"]["id"].as_str().expect("run created").to_string()
}

fn boot_open_with_one_stream_each(trusted: &str) -> RunningServer {
    boot(Auth::open(), trusted, |c| {
        c.rate_limit_per_min = 0;
        c.max_sse_streams_per_caller = 1;
    })
}

#[test]
fn without_tokens_the_stream_cap_is_per_forwarded_client_behind_a_trusted_proxy() {
    // Arrange: a queued run (no workers), so its event stream stays open.
    let srv = boot_open_with_one_stream_each(LOOPBACK_PROXY);
    let path = format!("/api/runs/{}/events", create_run(srv.addr()));
    let (first, _held) = send(srv.addr(), &path, &[&xff("198.51.100.1")]);
    assert_eq!(first, 200);

    // Act and assert: the same client is at its cap, another client is not.
    assert_eq!(status(srv.addr(), &path, &[&xff("198.51.100.1")]), 429);
    assert_eq!(status(srv.addr(), &path, &[&xff("198.51.100.2")]), 200);
}

#[test]
fn without_tokens_or_trusted_proxies_the_stream_cap_ignores_forwarding_headers() {
    let srv = boot_open_with_one_stream_each("");
    let path = format!("/api/runs/{}/events", create_run(srv.addr()));
    let (first, _held) = send(srv.addr(), &path, &[&xff("198.51.100.1")]);
    assert_eq!(first, 200);
    assert_eq!(status(srv.addr(), &path, &[&xff("198.51.100.2")]), 429);
}

#[test]
fn forwarding_headers_do_not_get_past_the_open_mode_host_guard() {
    let srv = boot(Auth::open(), LOOPBACK_PROXY, |_| {});
    let mut s = TcpStream::connect(srv.addr()).unwrap();
    s.set_read_timeout(Some(PATIENCE)).unwrap();
    s.write_all(
        b"GET /api/targets HTTP/1.1\r\nHost: example.com\r\nX-Forwarded-Host: 127.0.0.1\r\n\
          X-Forwarded-For: 127.0.0.1\r\nForwarded: host=127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 403"), "{out}");
}

#[test]
fn the_connection_cap_is_unchanged_by_trusted_proxies() {
    // Arrange: room for one connection, taken by a client that has sent nothing.
    let srv = boot(Auth::open(), LOOPBACK_PROXY, |c| c.max_connections = 1);
    let idle = TcpStream::connect(srv.addr()).unwrap();
    std::thread::sleep(Duration::from_millis(200));

    // Act: a second connection, claiming to be forwarded for someone else.
    let mut s = TcpStream::connect(srv.addr()).unwrap();
    s.set_read_timeout(Some(PATIENCE)).unwrap();
    let _ = s.write_all(
        b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Forwarded-For: 198.51.100.1\r\n\r\n",
    );
    let mut out = String::new();
    let t0 = Instant::now();
    let _ = s.read_to_string(&mut out);

    // Assert
    assert!(t0.elapsed() < PATIENCE);
    assert!(out.starts_with("HTTP/1.1 503"), "{out}");
    drop(idle);
}

#[test]
fn a_trusted_proxy_entry_that_covers_everyone_is_refused() {
    for spec in ["0.0.0.0/0", "::/0", "0.0.0.0", "::", "0.0.0.0/1", "::/96"] {
        assert!(TrustedProxies::parse(spec).is_err(), "{spec}");
    }
}

#[test]
fn text_a_client_put_left_of_the_proxys_entry_does_not_move_it_to_the_proxys_bucket() {
    // Arrange: a proxy that appended the address it saw to whatever the client sent.
    let srv = boot_with_token(LOOPBACK_PROXY);
    let junk = "x".repeat(4000);

    // Act: one client floods, each request with unusable text to the left of the proxy's entry.
    let statuses: Vec<u16> = (0..GUESSES)
        .map(|i| {
            let wrong = bearer(&format!("wrong-guess-number-{i:04}-abcdef"));
            let fwd = xff(&format!("{junk}, not-an-address, 198.51.100.1"));
            status(srv.addr(), "/api/targets", &[&wrong, &fwd])
        })
        .collect();

    // Assert: it is counted under its own address, so another client behind the proxy is unaffected.
    assert_eq!(statuses, flooded());
    assert_eq!(token_from(srv.addr(), "198.51.100.2"), 200);
}
