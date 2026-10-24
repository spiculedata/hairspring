//! Starting a sweep over HTTP: `mode` on `POST /api/runs`, every trial readable afterwards, search still the default.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ADMIN: &str = "admin-token-0123456789abcdef";

fn boot() -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let auth = Auth::from_spec(&format!("{ADMIN}:admin")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 1;
    cfg.rate_limit_per_min = 0;
    start(cfg, Service::new(store), engine).expect("server starts")
}

struct Client {
    agent: ureq::Agent,
    base: String,
}

impl Client {
    fn new(srv: &RunningServer) -> Client {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Client {
            agent,
            base: format!("http://{}", srv.addr()),
        }
    }

    fn get(&self, path: &str) -> (u16, Value) {
        let mut r = self
            .agent
            .get(format!("{}{path}", self.base))
            .header("Authorization", format!("Bearer {ADMIN}"))
            .call()
            .expect("transport ok");
        let status = r.status().as_u16();
        let body = r.body_mut().read_to_string().unwrap_or_default();
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    fn post(&self, path: &str, body: &Value) -> (u16, Value) {
        let mut r = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {ADMIN}"))
            .send(body.to_string())
            .expect("transport ok");
        let status = r.status().as_u16();
        let text = r.body_mut().read_to_string().unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    fn finished(&self, run_id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let (_, run) = self.get(&format!("/api/runs/{run_id}"));
            if run["run"]["state"] == "failed" || run["run"]["state"] == "passed" {
                return run;
            }
            assert!(Instant::now() < deadline, "run never finished: {run}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn start_run(c: &Client, body: Value) -> String {
    let (status, created) = c.post("/api/runs", &body);
    assert_eq!(status, 201, "{created}");
    created["run"]["id"].as_str().unwrap().to_string()
}

#[test]
fn a_sweep_started_over_http_runs_its_whole_budget_and_every_trial_is_readable() {
    let srv = boot();
    let c = Client::new(&srv);
    let id = start_run(
        &c,
        json!({"target": "lock-lease", "seed": 7, "mode": "sweep",
               "budget": {"max_steps": 10000000, "max_trials": 5, "wall_secs": 60}}),
    );
    let run = c.finished(&id);
    assert_eq!(run["run"]["trials"], 5, "{run}");
    assert_eq!(run["run"]["stopped_at_first_failure"], false);
    assert_eq!(
        run["sweep"]["trials"], 5,
        "the summary is served with the run: {run}"
    );
    assert_eq!(run["sweep"]["first_failing_seeds"][0], "7");
    let (_, trials) = c.get(&format!("/api/runs/{id}/trials"));
    let seeds: Vec<&str> = trials["trials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["seed"].as_str().unwrap())
        .collect();
    assert_eq!(seeds, vec!["7", "8", "9", "10", "11"], "{trials}");
}

#[test]
fn a_run_without_a_mode_is_a_search_and_stops_at_its_first_failure() {
    let srv = boot();
    let c = Client::new(&srv);
    let id = start_run(&c, json!({"target": "lock-lease", "seed": 7}));
    let run = c.finished(&id);
    assert_eq!(run["run"]["stopped_at_first_failure"], true, "{run}");
    assert!(
        run.get("sweep").is_none(),
        "a search has no sweep summary: {run}"
    );
    let (_, trials) = c.get(&format!("/api/runs/{id}/trials"));
    assert!(trials["trials"].as_array().unwrap().is_empty());
}

#[test]
fn an_unknown_mode_is_a_bad_request() {
    let srv = boot();
    let c = Client::new(&srv);
    for mode in [json!("fast"), json!("SWEEP"), json!(3), json!(null)] {
        let body = json!({"target": "lock-lease", "mode": mode});
        let (status, reply) = c.post("/api/runs", &body);
        // `null` is the same as leaving the field out; anything else that is not a mode is refused.
        let expected = if mode.is_null() { 201 } else { 400 };
        assert_eq!(status, expected, "{mode}: {reply}");
    }
}
