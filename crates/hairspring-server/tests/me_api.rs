//! `GET /api/me`: who the caller is and what they may do, for every role and for open mode.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const ADMIN: &str = "admin-token-0123456789abcdef";

fn boot(auth: Auth) -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    start(cfg, Service::new(store), engine).expect("server starts")
}

fn tokens() -> RunningServer {
    boot(Auth::from_spec(&format!("{READ}:read,{RUN}:run,{ADMIN}:admin")).unwrap())
}

struct Reply {
    status: u16,
    text: String,
    body: Value,
    cache_control: Option<String>,
}

fn get(srv: &RunningServer, path: &str, token: Option<&str>) -> Reply {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut req = agent.get(format!("http://{}{path}", srv.addr()));
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let mut r = req.call().expect("transport ok");
    let status = r.status().as_u16();
    let cache_control = r
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let text = r.body_mut().read_to_string().unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Reply {
        status,
        text,
        body,
        cache_control,
    }
}

fn expected(role: &str, open: bool, read: bool, run: bool, admin: bool) -> Value {
    json!({
        "role": role,
        "open_mode": open,
        "can": { "read": read, "run": run, "admin": admin },
        "roles": ["read", "run", "admin"],
    })
}

#[test]
fn read_token_may_only_read() {
    let r = get(&tokens(), "/api/me", Some(READ));
    assert_eq!(r.status, 200);
    assert_eq!(r.body, expected("read", false, true, false, false));
}

#[test]
fn run_token_may_read_and_run() {
    let r = get(&tokens(), "/api/me", Some(RUN));
    assert_eq!(r.status, 200);
    assert_eq!(r.body, expected("run", false, true, true, false));
}

#[test]
fn admin_token_may_do_everything() {
    let r = get(&tokens(), "/api/me", Some(ADMIN));
    assert_eq!(r.status, 200);
    assert_eq!(r.body, expected("admin", false, true, true, true));
}

#[test]
fn open_mode_says_so_and_grants_admin() {
    let r = get(&boot(Auth::open()), "/api/me", None);
    assert_eq!(r.status, 200);
    assert_eq!(r.body, expected("admin", true, true, true, true));
}

#[test]
fn missing_or_wrong_token_is_401_like_other_read_endpoints() {
    let srv = tokens();
    let me = get(&srv, "/api/me", None);
    let runs = get(&srv, "/api/runs", None);
    assert_eq!(me.status, 401);
    assert_eq!(me.status, runs.status);
    assert_eq!(me.body, runs.body);
    assert_eq!(
        get(&srv, "/api/me", Some("not-a-real-token-0123456789")).status,
        401
    );
}

#[test]
fn response_never_carries_the_token_or_a_digest() {
    use sha2::{Digest, Sha256};
    let srv = tokens();
    for token in [READ, RUN, ADMIN] {
        let r = get(&srv, "/api/me", Some(token));
        assert!(!r.text.contains(token), "token echoed: {}", r.text);
        assert!(!r.text.contains("token"), "token field present: {}", r.text);
        let digest: String = Sha256::digest(token.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(
            !r.text.contains(&digest[..16]),
            "digest present: {}",
            r.text
        );
    }
}

#[test]
fn response_is_not_cached() {
    let r = get(&tokens(), "/api/me", Some(READ));
    assert_eq!(r.cache_control.as_deref(), Some("no-store"));
}
