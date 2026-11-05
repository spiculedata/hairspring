//! What a caller sees at the edges: worker ids in the event stream, per-caller stream slots, security headers and
//! the error shape for a bad query string.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const API_CSP: &str = "default-src 'none'; frame-ancestors 'none'";

fn boot(edit: impl FnOnce(&mut ServerConfig)) -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let auth = Auth::from_spec(&format!("{READ}:read,{RUN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 1;
    cfg.rate_limit_per_min = 0;
    edit(&mut cfg);
    start(cfg, Service::new(store), engine).expect("server starts")
}

struct Resp {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into()
}

fn finish(r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Resp {
    let mut resp = r.expect("transport ok");
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    Resp {
        status: resp.status().as_u16(),
        headers,
        body: resp.body_mut().read_to_string().unwrap_or_default(),
    }
}

fn get(addr: SocketAddr, token: Option<&str>, path: &str) -> Resp {
    let mut r = agent().get(format!("http://{addr}{path}"));
    if let Some(t) = token {
        r = r.header("Authorization", format!("Bearer {t}"));
    }
    finish(r.call())
}

fn post(addr: SocketAddr, token: &str, path: &str, body: &Value) -> Resp {
    finish(
        agent()
            .post(format!("http://{addr}{path}"))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"))
            .send(body.to_string()),
    )
}

fn wait_for(addr: SocketAddr, id: &str, done: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = get(addr, Some(READ), &format!("/api/runs/{id}")).json()["run"]["state"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if done(&state) {
            return;
        }
        assert!(Instant::now() < deadline, "run stuck in {state}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn start_run(addr: SocketAddr, body: Value) -> String {
    let r = post(addr, RUN, "/api/runs", &body);
    assert_eq!(r.status, 201, "{}", r.body);
    r.json()["run"]["id"].as_str().unwrap().to_string()
}

fn long_run(addr: SocketAddr) -> String {
    let id = start_run(
        addr,
        json!({"target": "pubsub-atomic", "seed": 1, "budget": {"max_steps": 40000000, "max_trials": 1000, "wall_secs": 30}}),
    );
    wait_for(addr, &id, |s| s == "running");
    id
}

#[test]
fn the_started_event_names_its_worker_only_to_run_callers() {
    // Arrange: a finished run, so its stream replays the stored events and ends.
    let srv = boot(|_| {});
    let id = start_run(srv.addr(), json!({"target": "lock-lease", "seed": 7}));
    wait_for(srv.addr(), &id, |s| s != "queued" && s != "running");

    // Act
    let as_read = get(srv.addr(), Some(READ), &format!("/api/runs/{id}/events"));
    let as_run = get(srv.addr(), Some(RUN), &format!("/api/runs/{id}/events"));

    // Assert
    assert_eq!(as_read.status, 200);
    assert!(as_read.body.contains("event: started"), "{}", as_read.body);
    assert!(
        !as_read.body.contains("wrk_"),
        "a read token must not learn worker ids: {}",
        as_read.body
    );
    assert!(as_run.body.contains("wrk_"), "{}", as_run.body);
}

#[test]
fn one_caller_cannot_take_every_event_stream_slot() {
    // Arrange: plenty of slots in total, one per caller, and a run that keeps its streams open.
    let srv = boot(|c| {
        c.max_sse_streams = 8;
        c.max_sse_streams_per_caller = 1;
    });
    let addr = srv.addr();
    let id = long_run(addr);
    let held = std::thread::spawn({
        let id = id.clone();
        move || get(addr, Some(READ), &format!("/api/runs/{id}/events")).status
    });
    std::thread::sleep(Duration::from_millis(500)); // let the first stream attach

    // Act
    let second_same_caller = get(addr, Some(READ), &format!("/api/runs/{id}/events"));
    let other = std::thread::spawn({
        let id = id.clone();
        move || get(addr, Some(RUN), &format!("/api/runs/{id}/events")).status
    });
    std::thread::sleep(Duration::from_millis(500));
    post(addr, RUN, &format!("/api/runs/{id}/cancel"), &json!({}));

    // Assert
    assert_eq!(
        second_same_caller.status, 429,
        "{}",
        second_same_caller.body
    );
    assert_eq!(
        second_same_caller.header("content-type"),
        Some("application/json")
    );
    assert_eq!(other.join().unwrap(), 200, "another caller still gets one");
    assert_eq!(held.join().unwrap(), 200);
    // Slots are given back when streams end.
    let again = get(addr, Some(READ), &format!("/api/runs/{id}/events"));
    assert_eq!(again.status, 200);
}

#[test]
fn api_responses_carry_a_restrictive_content_security_policy() {
    let srv = boot(|_| {});
    let health = get(srv.addr(), None, "/api/health");
    assert_eq!(health.header("content-security-policy"), Some(API_CSP));
    let denied = get(srv.addr(), None, "/api/runs");
    assert_eq!(denied.status, 401);
    assert_eq!(denied.header("content-security-policy"), Some(API_CSP));
}

#[test]
fn the_web_ui_policy_pins_base_uri_and_form_targets() {
    let srv = boot(|_| {});
    let page = get(srv.addr(), None, "/");
    let csp = page.header("content-security-policy").unwrap_or("");
    assert!(csp.contains("base-uri 'none'"), "{csp}");
    assert!(csp.contains("form-action 'self'"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
}

#[test]
fn a_query_string_that_does_not_parse_gets_the_json_error_shape() {
    let srv = boot(|_| {});
    let id = start_run(srv.addr(), json!({"target": "lock-lease", "seed": 7}));
    for path in [
        format!("/api/runs/{id}/events?after=abc"),
        format!("/api/runs/{id}/events?after=1&after=2"),
    ] {
        let r = get(srv.addr(), Some(READ), &path);
        assert_eq!(r.status, 400, "{path}: {}", r.body);
        assert_eq!(r.header("content-type"), Some("application/json"), "{path}");
        let err = r.json()["error"].as_str().unwrap().to_string();
        assert_eq!(err, "invalid query string", "{path}");
    }
}
