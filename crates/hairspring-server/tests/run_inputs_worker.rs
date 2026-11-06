//! A real remote worker against a real server: the worker reports the inputs of the job it runs, and the server keeps
//! them. Removing the worker's report fails this test.
use hairspring_engine::config::Config;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RUN: &str = "run-token-0123456789abcdef";

fn get_json(agent: &ureq::Agent, url: &str) -> Value {
    let mut resp = agent
        .get(url)
        .header("Authorization", format!("Bearer {RUN}"))
        .call()
        .expect("transport ok");
    serde_json::from_str(&resp.body_mut().read_to_string().unwrap()).unwrap()
}

#[test]
fn a_remote_worker_reports_the_inputs_of_the_run_it_executes() {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Arc::new(Engine::new(
        registry.clone(),
        Config::from_registry(&registry),
    ));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let auth = Auth::from_spec(&format!("{RUN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    let srv = start(cfg, Service::new(store.clone()), (*engine).clone()).expect("server starts");
    let addr = srv.addr();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut created = agent
        .post(format!("http://{addr}/api/runs"))
        .header("Authorization", format!("Bearer {RUN}"))
        .header("Content-Type", "application/json")
        .send(
            json!({"target": "pubsub-atomic", "seed": 1,
                     "budget": {"max_steps": 1000, "max_trials": 3, "wall_secs": 30}})
            .to_string(),
        )
        .expect("transport ok");
    let body: Value = serde_json::from_str(&created.body_mut().read_to_string().unwrap()).unwrap();
    let run_id = body["run"]["id"].as_str().unwrap().to_string();

    let stop = Arc::new(AtomicBool::new(false));
    let (stop2, engine2) = (stop.clone(), engine.clone());
    let worker = std::thread::spawn(move || {
        let mut cfg =
            hairspring_worker::WorkerConfig::new(format!("http://{addr}"), "remote-inputs");
        cfg.token = Some(RUN.to_string());
        cfg.poll = Duration::from_millis(50);
        hairspring_worker::run_loop(&engine2, &cfg, &stop2)
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let run = get_json(&agent, &format!("http://{addr}/api/runs/{run_id}"));
        if matches!(run["run"]["state"].as_str(), Some("passed" | "failed")) {
            break;
        }
        assert!(Instant::now() < deadline, "run never finished: {run}");
        std::thread::sleep(Duration::from_millis(50));
    }
    stop.store(true, Ordering::SeqCst);
    worker.join().unwrap().unwrap();

    let stored = store
        .get_run_inputs(&run_id)
        .unwrap()
        .expect("the worker reported the inputs of its run");
    assert_eq!(stored.scheme, "sim/1");
    assert!(
        stored.reporter_id.starts_with("wrk_"),
        "{}",
        stored.reporter_id
    );
    assert!(
        !stored.reporter_trusted,
        "a remote worker is untrusted until an administrator says so"
    );
    assert_eq!(stored.attempt, 1);
    assert!(stored.missing.is_empty());
    let names: Vec<&str> = stored.fields.keys().map(String::as_str).collect();
    assert_eq!(names, ["backend", "hairspring_sha", "target"]);
}
