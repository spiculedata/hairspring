//! The per-client rate limit, over real HTTP: a valid token from an address that has used up its anonymous budget
//! is refused until that budget's minute is over, while legitimate use from a quiet address is unaffected.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const LIMIT: u32 = 2;
const GUESSES: usize = 10;

fn boot(auth: Auth, limit: u32) -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = limit;
    start(cfg, Service::new(store), engine).expect("server starts")
}

fn boot_with_token(limit: u32) -> RunningServer {
    boot(Auth::from_spec(&format!("{READ}:read")).unwrap(), limit)
}

struct Reply {
    status: u16,
    head: String,
}

/// Sends one GET with exactly the given extra header lines, so header spelling and duplicates are under the test's
/// control rather than an HTTP client's.
fn get(addr: SocketAddr, path: &str, headers: &[&str]) -> Reply {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for h in headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status line: {out:?}"));
    let head = out
        .split("\r\n\r\n")
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    Reply { status, head }
}

fn bearer(token: &str) -> String {
    format!("Authorization: Bearer {token}")
}

/// Spends the anonymous budget of 127.0.0.1 with wrong tokens and returns the statuses seen.
fn guess(addr: SocketAddr, extra: &[&str]) -> Vec<u16> {
    (0..GUESSES)
        .map(|i| {
            let wrong = bearer(&format!("wrong-guess-number-{i:04}-abcdef"));
            let mut headers = vec![wrong.as_str()];
            headers.extend_from_slice(extra);
            get(addr, "/api/targets", &headers).status
        })
        .collect()
}

fn expected_guess_statuses() -> Vec<u16> {
    let mut want = vec![401; LIMIT as usize];
    want.extend(vec![429; GUESSES - LIMIT as usize]);
    want
}

#[test]
fn a_valid_token_is_refused_while_its_address_has_used_up_its_anonymous_budget() {
    let srv = boot_with_token(LIMIT);
    assert_eq!(guess(srv.addr(), &[]), expected_guess_statuses());
    let real = get(srv.addr(), "/api/targets", &[&bearer(READ)]);
    assert_eq!(
        real.status, 429,
        "the right token must not get through once guessing is limited"
    );
    assert!(real.head.contains("retry-after: 60"), "{}", real.head);
}

#[test]
fn a_valid_token_from_a_quiet_address_works_and_is_limited_by_its_own_bucket() {
    let srv = boot_with_token(LIMIT);
    let statuses: Vec<u16> = (0..4)
        .map(|_| get(srv.addr(), "/api/targets", &[&bearer(READ)]).status)
        .collect();
    assert_eq!(statuses, vec![200, 200, 429, 429]);
}

#[test]
fn requests_with_a_valid_token_do_not_spend_the_anonymous_budget() {
    let srv = boot_with_token(LIMIT);
    for _ in 0..LIMIT {
        assert_eq!(
            get(srv.addr(), "/api/targets", &[&bearer(READ)]).status,
            200
        );
    }
    // The address's anonymous budget is untouched: a request without a token is still answered 401, not 429.
    assert_eq!(get(srv.addr(), "/api/targets", &[]).status, 401);
}

#[test]
fn health_without_a_token_stays_unlimited_while_the_address_is_limited() {
    let srv = boot_with_token(LIMIT);
    assert_eq!(guess(srv.addr(), &[]), expected_guess_statuses());
    for _ in 0..GUESSES {
        assert_eq!(get(srv.addr(), "/api/health", &[]).status, 200);
    }
    // Health answers 200 to a valid token too; it never becomes a way to tell a right token from a wrong one.
    assert_eq!(get(srv.addr(), "/api/health", &[&bearer(READ)]).status, 200);
}

#[test]
fn every_accepted_spelling_of_the_header_is_refused_while_the_address_is_limited() {
    let srv = boot_with_token(LIMIT);
    assert_eq!(guess(srv.addr(), &[]), expected_guess_statuses());
    let variants = [
        format!("Authorization: Bearer {READ}"),
        format!("authorization: bearer {READ}"),
        format!("AUTHORIZATION: BEARER {READ}"),
        format!("Authorization: Bearer\t{READ}"),
        format!("Authorization: Bearer    {READ}   "),
    ];
    for v in &variants {
        assert_eq!(get(srv.addr(), "/api/targets", &[v]).status, 429, "{v:?}");
    }
}

#[test]
fn a_duplicated_authorization_header_is_refused_either_way_round() {
    let srv = boot_with_token(LIMIT);
    assert_eq!(guess(srv.addr(), &[]), expected_guess_statuses());
    let real = bearer(READ);
    let wrong = bearer("wrong-guess-duplicate-0123456789");
    assert_eq!(
        get(srv.addr(), "/api/targets", &[&real, &wrong]).status,
        429
    );
    assert_eq!(
        get(srv.addr(), "/api/targets", &[&wrong, &real]).status,
        429
    );
}

#[test]
fn forwarding_headers_do_not_change_which_address_is_counted() {
    let srv = boot_with_token(LIMIT);
    let statuses: Vec<u16> = (0..GUESSES)
        .map(|i| {
            let wrong = bearer(&format!("wrong-guess-number-{i:04}-abcdef"));
            let xff = format!("X-Forwarded-For: 198.51.100.{i}");
            let real_ip = format!("X-Real-IP: 203.0.113.{i}");
            get(srv.addr(), "/api/targets", &[&wrong, &xff, &real_ip]).status
        })
        .collect();
    assert_eq!(statuses, expected_guess_statuses());
    let real = get(
        srv.addr(),
        "/api/targets",
        &[
            &bearer(READ),
            "X-Forwarded-For: 192.0.2.200",
            "Forwarded: for=192.0.2.201",
        ],
    );
    assert_eq!(real.status, 429);
}

#[test]
fn the_same_holds_on_the_mcp_endpoint() {
    let srv = boot_with_token(LIMIT);
    assert_eq!(guess(srv.addr(), &[]), expected_guess_statuses());
    assert_eq!(get(srv.addr(), "/mcp", &[&bearer(READ)]).status, 429);
}

#[test]
fn a_zero_limit_still_means_unlimited() {
    let srv = boot_with_token(0);
    let statuses = guess(srv.addr(), &[]);
    assert!(statuses.iter().all(|s| *s == 401), "{statuses:?}");
    assert_eq!(
        get(srv.addr(), "/api/targets", &[&bearer(READ)]).status,
        200
    );
}
