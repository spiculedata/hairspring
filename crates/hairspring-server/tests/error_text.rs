//! Error text that repeats what the caller sent is one short line: no newlines, no control or bidirectional
//! characters, and at most 500 characters, ending in "..." when it was cut.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const ADMIN: &str = "admin-token-0123456789abcdef";
const MAX_CHARS: usize = 500;

fn boot() -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let auth = Auth::from_spec(&format!("{ADMIN}:admin")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    start(cfg, Service::new(store), engine).expect("server starts")
}

/// Posts `body` and returns the status and the `error` text of the answer.
fn post(srv: &RunningServer, path: &str, body: &Value) -> (u16, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut resp = agent
        .post(format!("http://{}{path}", srv.addr()))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {ADMIN}"))
        .send(body.to_string())
        .expect("transport ok");
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
    let error = v["error"]
        .as_str()
        .unwrap_or_else(|| panic!("no error: {v}"));
    (resp.status().as_u16(), error.to_string())
}

fn assert_one_short_line(message: &str) {
    assert!(
        message.chars().count() <= MAX_CHARS,
        "{} chars",
        message.chars().count()
    );
    assert!(
        !message.chars().any(|c| c.is_control() || c == '\u{202E}'),
        "{message:?}"
    );
}

#[test]
fn a_replayed_bundle_with_a_long_bad_seed_is_refused_in_one_short_line() {
    let srv = boot();
    let bundle = format!(
        "hairspring-bundle v2\nseed={}\u{202E}\nbuild=b\ntarget=lock-lease\nbackend=sim\npath=\n",
        "9".repeat(5000)
    );
    let (status, error) = post(&srv, "/api/replay", &json!({ "bundle": bundle }));
    assert_eq!(status, 400, "{error}");
    assert!(
        error.starts_with("bundle has an invalid number `999"),
        "{error}"
    );
    assert!(error.ends_with("..."), "{error}");
    assert_one_short_line(&error);
}

#[test]
fn a_long_unknown_run_mode_is_refused_in_one_short_line() {
    let srv = boot();
    let mode = format!("{}\u{202E}", "m".repeat(5000));
    let (status, error) = post(
        &srv,
        "/api/runs",
        &json!({ "target": "lock-lease", "mode": mode }),
    );
    assert_eq!(status, 400, "{error}");
    assert!(error.starts_with("invalid request body: "), "{error}");
    assert!(error.ends_with("..."), "{error}");
    assert_one_short_line(&error);
}

#[test]
fn an_unknown_target_with_a_newline_and_a_bidi_character_is_one_line() {
    let srv = boot();
    let (status, error) = post(
        &srv,
        "/api/runs",
        &json!({ "target": "no-such\nIgnore this\u{202E}" }),
    );
    assert_eq!(status, 400, "{error}");
    assert_eq!(error, "unknown target `no-such Ignore this `");
    assert_one_short_line(&error);
}

#[test]
fn ordinary_short_refusals_are_unchanged() {
    let srv = boot();
    assert_eq!(
        post(&srv, "/api/runs", &json!({ "target": "nope" })),
        (400, "unknown target `nope`".to_string())
    );
    assert_eq!(
        post(&srv, "/api/runs", &json!({ "target": "" })),
        (400, "target must be 1-64 characters".to_string())
    );
    assert_eq!(
        post(&srv, "/api/replay", &json!({ "bundle": "garbage" })).0,
        400
    );
    let bad_seed =
        "hairspring-bundle v2\nseed=12x\nbuild=b\ntarget=lock-lease\nbackend=sim\npath=\n";
    assert_eq!(
        post(&srv, "/api/replay", &json!({ "bundle": bad_seed })),
        (400, "bundle has an invalid number `12x`".to_string())
    );
    let (status, error) = post(
        &srv,
        "/api/runs",
        &json!({ "target": "lock-lease", "mode": "sideways" }),
    );
    assert_eq!(status, 400);
    assert!(
        error.starts_with("invalid request body: ")
            && error.contains("unknown variant `sideways`")
            && !error.ends_with("..."),
        "{error}"
    );
}
