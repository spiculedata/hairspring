//! `GET /api/workers` over real HTTP: state, trust, capabilities, held jobs and queue depth for every role, and that
//! no worker id reaches a read caller and no secret, digest or job payload reaches anyone.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::{Service, StartRun};
use hairspring_store::{SqliteStore, Store};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const NDJSON: &str = "application/x-ndjson";

struct Fixture {
    server: RunningServer,
    store: Arc<dyn Store>,
    busy_id: String,
    busy_secret: String,
    run_id: String,
}

struct Reply {
    status: u16,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

fn get(srv: &RunningServer, token: Option<&str>, accept: Option<&str>, path: &str) -> Reply {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut req = agent.get(format!("http://{}{path}", srv.addr()));
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    if let Some(a) = accept {
        req = req.header("Accept", a);
    }
    let mut r = req.call().expect("transport ok");
    Reply {
        status: r.status().as_u16(),
        body: r.body_mut().read_to_string().unwrap_or_default(),
    }
}

/// One remote worker that holds a job, one local trusted worker that is idle, one run still waiting.
fn boot() -> Fixture {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store.clone());
    let start_run = || {
        svc.start_run(
            &engine,
            &StartRun {
                target: "lock-lease".into(),
                seed: Some(1),
                budget: None,
            },
        )
        .unwrap()
        .id
    };
    let (busy, busy_secret) = svc
        .register_remote_worker("remote-1", "{\"backends\":[\"sim\"]}")
        .unwrap();
    svc.register_local_worker("local-1", "{\"backends\":[\"embedded\"]}")
        .unwrap();
    let run_id = start_run();
    start_run();
    let job = svc.lease(&busy.id).unwrap().expect("a job");
    assert_eq!(job.run_id, run_id);
    let auth = Auth::from_spec(&format!("{READ}:read,{RUN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    Fixture {
        server: start(cfg, svc, engine).expect("server starts"),
        store,
        busy_id: busy.id,
        busy_secret,
        run_id,
    }
}

fn by_name<'a>(body: &'a Value, name: &str) -> &'a Value {
    body["workers"]
        .as_array()
        .expect("workers array")
        .iter()
        .find(|w| w["name"] == name)
        .unwrap_or_else(|| panic!("no worker {name} in {body}"))
}

#[test]
fn a_read_caller_sees_state_trust_capabilities_held_jobs_and_the_queue() {
    let f = boot();
    let r = get(&f.server, Some(READ), None, "/api/workers");
    assert_eq!(r.status, 200);
    let body = r.json();
    assert_eq!(body["queue"]["queued"], 1);
    assert_eq!(body["queue"]["running"], 1);
    let remote = by_name(&body, "remote-1");
    assert_eq!(remote["state"], "busy");
    assert_eq!(remote["trusted"], false);
    assert_eq!(remote["capabilities"]["backends"][0], "sim");
    assert_eq!(remote["leased_jobs"], 1);
    assert_eq!(remote["current_jobs"][0]["run_id"], f.run_id.as_str());
    assert_eq!(remote["current_jobs"][0]["kind"], "run");
    let local = by_name(&body, "local-1");
    assert_eq!(local["state"], "idle");
    assert_eq!(local["trusted"], true);
    assert_eq!(local["leased_jobs"], 0);
    assert_eq!(local["current_jobs"], Value::Array(vec![]));
}

#[test]
fn a_read_caller_never_gets_a_worker_id_and_a_run_caller_does() {
    let f = boot();
    let read = get(&f.server, Some(READ), None, "/api/workers");
    assert!(!read.body.contains(&f.busy_id), "{}", read.body);
    for w in read.json()["workers"].as_array().unwrap() {
        assert!(w.get("id").is_none(), "{w}");
    }
    let run = get(&f.server, Some(RUN), None, "/api/workers");
    assert_eq!(by_name(&run.json(), "remote-1")["id"], f.busy_id.as_str());
}

#[test]
fn no_secret_digest_or_job_payload_reaches_any_caller() {
    let f = boot();
    let digest = f
        .store
        .worker_secret_hash(&f.busy_id)
        .unwrap()
        .expect("a stored digest");
    for token in [READ, RUN] {
        for accept in [None, Some(NDJSON)] {
            let r = get(&f.server, Some(token), accept, "/api/workers");
            assert!(
                !r.body.contains(&f.busy_secret),
                "secret leaked: {}",
                r.body
            );
            assert!(!r.body.contains(&digest), "digest leaked: {}", r.body);
            for banned in ["secret", "hash", "payload", "last_error", "token"] {
                assert!(!r.body.contains(banned), "`{banned}` in {}", r.body);
            }
        }
    }
}

#[test]
fn ndjson_lists_one_worker_per_line_and_puts_the_queue_on_the_control_line() {
    let f = boot();
    let r = get(&f.server, Some(READ), Some(NDJSON), "/api/workers");
    assert_eq!(r.status, 200);
    let lines: Vec<Value> = r
        .body
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3, "{}", r.body);
    assert_eq!(lines[2]["page"]["queue"]["queued"], 1);
}

#[test]
fn a_caller_without_a_token_is_refused() {
    let f = boot();
    assert_eq!(get(&f.server, None, None, "/api/workers").status, 401);
}
