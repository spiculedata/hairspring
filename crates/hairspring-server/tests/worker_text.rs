//! Text a remote worker sends over HTTP (its name, a progress message, a failure reason) is refused or cleaned on the
//! way in, so it cannot reach `GET /api/workers`, the event stream or `GET /api/runs/{id}` with a line break or a
//! character that reorders text.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const RUN: &str = "run-token-0123456789abcdef";
const HOSTILE: &str = "a\nb\u{7}c\u{202E}d\u{2028}e\u{2029}f";
const CLEANED: &str = "a b c d e f";
const MAX_ATTEMPTS: usize = 3;

fn boot() -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let auth = Auth::from_spec(&format!("{RUN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    start(cfg, Service::new(store), engine).expect("server starts")
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}

fn call(srv: &RunningServer, method: &str, path: &str, body: Option<&Value>) -> (u16, String) {
    let url = format!("http://{}{path}", srv.addr());
    let auth = format!("Bearer {RUN}");
    let mut resp = match body {
        Some(b) => agent()
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", auth)
            .send(b.to_string()),
        None => {
            assert_eq!(method, "GET");
            agent().get(url).header("Authorization", auth).call()
        }
    }
    .expect("transport ok");
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    (resp.status().as_u16(), text)
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn assert_clean(text: &str) {
    assert!(
        !text
            .chars()
            .any(|c| matches!(c, '\u{7}' | '\u{202E}' | '\u{2028}' | '\u{2029}')),
        "{text:?}"
    );
}

#[test]
fn a_worker_name_with_a_line_break_or_a_bidirectional_character_is_a_400() {
    let srv = boot();
    for bad in ["a\nb", "a\u{202E}b", "a\u{2028}b", "a\u{2029}b"] {
        let (status, text) = call(
            &srv,
            "POST",
            "/api/worker/register",
            Some(&json!({ "name": bad })),
        );
        assert_eq!(status, 400, "{bad:?}: {text}");
        let error = json_of(&text)["error"].as_str().unwrap().to_string();
        assert!(error.contains("worker name"), "{error}");
        assert_clean(&error);
    }
    let (_, workers) = call(&srv, "GET", "/api/workers", None);
    assert_eq!(json_of(&workers)["workers"], json!([]));
}

#[test]
fn a_progress_message_and_failure_reason_come_back_as_clean_text() {
    let srv = boot();
    let (_, text) = call(
        &srv,
        "POST",
        "/api/runs",
        Some(&json!({ "target": "pubsub-atomic", "seed": 1 })),
    );
    let run_id = json_of(&text)["run"]["id"].as_str().unwrap().to_string();
    let (status, text) = call(
        &srv,
        "POST",
        "/api/worker/register",
        Some(&json!({ "name": "remote" })),
    );
    assert_eq!(status, 200, "{text}");
    let reg = json_of(&text);
    let (worker_id, secret) = (reg["worker_id"].clone(), reg["worker_secret"].clone());
    for attempt in 0..MAX_ATTEMPTS {
        let (_, text) = call(
            &srv,
            "POST",
            "/api/worker/lease",
            Some(&json!({ "worker_id": worker_id, "worker_secret": secret })),
        );
        let job_id = json_of(&text)["job_id"].as_str().unwrap().to_string();
        if attempt == 0 {
            let (status, _) = call(
                &srv,
                "POST",
                &format!("/api/worker/jobs/{job_id}/progress"),
                Some(&json!({ "worker_id": worker_id, "worker_secret": secret,
                    "steps": 1, "trials": 1, "message": HOSTILE })),
            );
            assert_eq!(status, 200);
        }
        let (status, _) = call(
            &srv,
            "POST",
            &format!("/api/worker/jobs/{job_id}/fail"),
            Some(&json!({ "worker_id": worker_id, "worker_secret": secret, "error": HOSTILE })),
        );
        assert_eq!(status, 200);
    }
    let (_, run) = call(&srv, "GET", &format!("/api/runs/{run_id}"), None);
    assert_eq!(json_of(&run)["run"]["error"], CLEANED, "{run}");
    let (_, events) = call(&srv, "GET", &format!("/api/runs/{run_id}/events"), None);
    assert!(events.contains(CLEANED), "{events:?}");
    assert_clean(&events);
}
