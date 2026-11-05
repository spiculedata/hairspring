//! The per-client rate limit, over real HTTP: a valid token from an address that has used up its anonymous budget
//! is refused until that budget's minute is over, while legitimate use from a quiet address is unaffected. Health
//! requests that carry a token are counted by the same rule, so one token cannot lock others out through health.
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
    body: String,
}

/// Sends one GET with exactly the given extra header lines, so header spelling and duplicates are under the test's
/// control rather than an HTTP client's.
fn get(addr: SocketAddr, path: &str, headers: &[&str]) -> Reply {
    let lines: Vec<&[u8]> = headers.iter().map(|h| h.as_bytes()).collect();
    get_raw(addr, path, &lines)
}

/// As [`get`], with header lines given as bytes, so a header value that is not valid UTF-8 can be sent.
fn get_raw(addr: SocketAddr, path: &str, headers: &[&[u8]]) -> Reply {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n").into_bytes();
    for h in headers {
        req.extend_from_slice(h);
        req.extend_from_slice(b"\r\n");
    }
    req.extend_from_slice(b"\r\n");
    s.write_all(&req).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let out = String::from_utf8_lossy(&raw).into_owned();
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
    let body = out.split_once("\r\n\r\n").map_or("", |(_, b)| b).to_owned();
    Reply { status, head, body }
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

// Health with a token. The middleware does not limit `/api/health`, but before the organisation name is added, a
// request to it that carries an `Authorization` header goes through the same admission rule as the limited
// endpoints: a valid token spends its own bucket, anything else spends the address's anonymous bucket.

const OTHER: &str = "other-token-0123456789abcdef";
const ORGANISATION: &str = "Acme Ltd";

fn boot_with_organisation(limit: u32) -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let auth = Auth::from_spec(&format!("{READ}:read,{OTHER}:read")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = limit;
    cfg.organisation = Some(ORGANISATION.to_owned());
    start(cfg, Service::new(store), engine).expect("server starts")
}

/// Calls health with the given header lines, checks it answered 200, and says whether the name was included.
fn health_shows_name(addr: SocketAddr, headers: &[&[u8]]) -> bool {
    let reply = get_raw(addr, "/api/health", headers);
    assert_eq!(reply.status, 200, "health always answers 200");
    let body: serde_json::Value = serde_json::from_str(&reply.body).expect("health body is JSON");
    assert_eq!(body["status"], "ok");
    match body.get("organisation") {
        None => false,
        Some(name) => {
            assert_eq!(name, ORGANISATION);
            true
        }
    }
}

#[test]
fn a_valid_tokens_health_polling_does_not_lock_other_tokens_out() {
    let srv = boot_with_organisation(LIMIT);
    let read = bearer(READ);
    for _ in 0..GUESSES {
        health_shows_name(srv.addr(), &[read.as_bytes()]);
    }
    assert_eq!(
        get(srv.addr(), "/api/targets", &[&bearer(OTHER)]).status,
        200,
        "another token on the same address is unaffected by one token's health requests"
    );
    assert_eq!(
        get(srv.addr(), "/api/targets", &[]).status,
        401,
        "the address's anonymous budget was not spent"
    );
    assert!(
        health_shows_name(srv.addr(), &[bearer(OTHER).as_bytes()]),
        "the other token still sees the name"
    );
}

#[test]
fn a_valid_tokens_health_requests_count_against_its_own_bucket() {
    let srv = boot_with_organisation(LIMIT);
    let read = bearer(READ);
    let shown: Vec<bool> = (0..4)
        .map(|_| health_shows_name(srv.addr(), &[read.as_bytes()]))
        .collect();
    assert_eq!(shown, vec![true, true, false, false]);
    assert_eq!(
        get(srv.addr(), "/api/targets", &[&read]).status,
        429,
        "health and the other endpoints share the token's bucket"
    );
}

#[test]
fn a_valid_tokens_other_requests_and_health_share_one_bucket() {
    let srv = boot_with_organisation(LIMIT);
    let read = bearer(READ);
    for _ in 0..LIMIT {
        assert_eq!(get(srv.addr(), "/api/targets", &[&read]).status, 200);
    }
    assert!(!health_shows_name(srv.addr(), &[read.as_bytes()]));
    assert!(health_shows_name(srv.addr(), &[bearer(OTHER).as_bytes()]));
}

#[test]
fn every_spelling_of_one_token_at_health_spends_the_same_bucket() {
    let srv = boot_with_organisation(LIMIT);
    let first = format!("authorization: bearer {READ}");
    let second = format!("Authorization: Bearer\t{READ}   ");
    let third = format!("AUTHORIZATION: BEARER {READ}");
    assert!(health_shows_name(srv.addr(), &[first.as_bytes()]));
    assert!(health_shows_name(srv.addr(), &[second.as_bytes()]));
    assert!(!health_shows_name(srv.addr(), &[third.as_bytes()]));
    assert_eq!(
        get(srv.addr(), "/api/targets", &[&bearer(OTHER)]).status,
        200
    );
}

#[test]
fn wrong_tokens_at_health_spend_the_address_budget_and_lock_valid_tokens_out() {
    let srv = boot_with_organisation(LIMIT);
    for i in 0..GUESSES {
        let wrong = bearer(&format!("wrong-guess-number-{i:04}-abcdef"));
        assert!(!health_shows_name(srv.addr(), &[wrong.as_bytes()]));
    }
    assert!(
        !health_shows_name(srv.addr(), &[bearer(READ).as_bytes()]),
        "the right token is not confirmed once the address is over the limit"
    );
    assert_eq!(
        get(srv.addr(), "/api/targets", &[&bearer(READ)]).status,
        429
    );
    assert!(
        !health_shows_name(srv.addr(), &[]),
        "tokenless health still answers"
    );
}

#[test]
fn malformed_authorization_at_health_spends_the_address_budget() {
    let srv = boot_with_organisation(LIMIT);
    let malformed: [&[u8]; 4] = [
        b"Authorization: Basic cmVhZDpyZWFk",
        b"Authorization: Bearer",
        b"Authorization: ",
        b"Authorization: Bearer \xff\xfe-not-utf8",
    ];
    for line in malformed {
        assert!(!health_shows_name(srv.addr(), &[line]), "{line:?}");
    }
    assert_eq!(
        get(srv.addr(), "/api/targets", &[&bearer(READ)]).status,
        429,
        "headers that are not a valid token count against the address"
    );
}

#[test]
fn concurrent_health_requests_with_one_token_show_the_name_exactly_limit_times() {
    let srv = boot_with_organisation(LIMIT);
    let addr = srv.addr();
    let handles: Vec<_> = (0..16)
        .map(|_| std::thread::spawn(move || health_shows_name(addr, &[bearer(READ).as_bytes()])))
        .collect();
    let shown = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|s| *s)
        .count();
    assert_eq!(shown, LIMIT as usize);
    assert_eq!(get(addr, "/api/targets", &[&bearer(OTHER)]).status, 200);
    assert_eq!(get(addr, "/api/targets", &[]).status, 401);
}
