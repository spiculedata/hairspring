//! `POST /api/worker/jobs/{job}/inputs`: a remote worker reports the hashes of the inputs of the run it holds. The
//! worker proves itself with its secret, a caller below the `run` role is refused, and a refusal never repeats what
//! was sent.
use hairspring_engine::config::Config;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{sha256_hex, SqliteStore, Store};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";

fn boot() -> (RunningServer, Arc<dyn Store>) {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let auth = Auth::from_spec(&format!("{READ}:read,{RUN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    (
        start(cfg, Service::new(store.clone()), engine).expect("server starts"),
        store,
    )
}

struct Resp {
    status: u16,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

fn post(addr: SocketAddr, token: &str, path: &str, body: &Value) -> Resp {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut resp = agent
        .post(format!("http://{addr}{path}"))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .send(body.to_string())
        .expect("transport ok");
    Resp {
        status: resp.status().as_u16(),
        body: resp.body_mut().read_to_string().unwrap_or_default(),
    }
}

struct Leased {
    run_id: String,
    job_id: String,
    worker: String,
    secret: String,
}

fn lease(srv: &RunningServer) -> Leased {
    let addr = srv.addr();
    let run_id = post(
        addr,
        RUN,
        "/api/runs",
        &json!({"target": "pubsub-atomic", "seed": 1,
                "budget": {"max_steps": 1000, "max_trials": 2, "wall_secs": 30}}),
    )
    .json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let reg = post(
        addr,
        RUN,
        "/api/worker/register",
        &json!({"name": "remote"}),
    )
    .json();
    let (worker, secret) = (
        reg["worker_id"].as_str().unwrap().to_string(),
        reg["worker_secret"].as_str().unwrap().to_string(),
    );
    let job_id = post(
        addr,
        RUN,
        "/api/worker/lease",
        &json!({"worker_id": worker, "worker_secret": secret}),
    )
    .json()["job_id"]
        .as_str()
        .expect("leased")
        .to_string();
    Leased {
        run_id,
        job_id,
        worker,
        secret,
    }
}

fn digest(c: char) -> String {
    c.to_string().repeat(64)
}

fn body(l: &Leased, inputs: Value) -> Value {
    json!({"worker_id": l.worker, "worker_secret": l.secret, "inputs": inputs})
}

fn sim_inputs() -> Value {
    json!({"scheme": "sim/1", "fields": {"hairspring_sha": digest('a'), "target": sha256_hex(b"pubsub-atomic"), "backend": sha256_hex(b"sim")}})
}

#[test]
fn a_remote_worker_reports_its_inputs_once_and_repeats_are_accepted() {
    let (srv, store) = boot();
    let l = lease(&srv);
    let path = format!("/api/worker/jobs/{}/inputs", l.job_id);
    let first = post(srv.addr(), RUN, &path, &body(&l, sim_inputs()));
    assert_eq!(first.status, 200, "{}", first.body);
    assert_eq!(first.json(), json!({"ok": true, "outcome": "stored"}));
    let stored = store.get_run_inputs(&l.run_id).unwrap().expect("stored");
    assert_eq!(
        (stored.scheme.as_str(), stored.reporter_trusted),
        ("sim/1", false)
    );
    assert_eq!(stored.reporter_id, l.worker);
    let again = post(srv.addr(), RUN, &path, &body(&l, sim_inputs()));
    assert_eq!(again.json(), json!({"ok": true, "outcome": "duplicate"}));
    let mut changed = sim_inputs();
    changed["fields"]["hairspring_sha"] = json!(digest('d'));
    let refused = post(srv.addr(), RUN, &path, &body(&l, changed));
    assert_eq!(refused.status, 400, "{}", refused.body);
    assert!(
        refused.body.contains("different inputs"),
        "{}",
        refused.body
    );
}

#[test]
fn the_worker_must_prove_itself_and_hold_the_lease() {
    let (srv, store) = boot();
    let l = lease(&srv);
    let path = format!("/api/worker/jobs/{}/inputs", l.job_id);
    for bad in [
        json!({"worker_id": l.worker, "inputs": sim_inputs()}),
        json!({"worker_id": l.worker, "worker_secret": "guess", "inputs": sim_inputs()}),
        json!({"worker_id": l.worker, "worker_secret": "", "inputs": sim_inputs()}),
    ] {
        let r = post(srv.addr(), RUN, &path, &bad);
        assert_eq!(r.status, 400, "{}", r.body);
        assert!(!r.body.contains(&l.secret));
    }
    // Another registered worker's credentials, naming the job it does not hold.
    let reg = post(
        srv.addr(),
        RUN,
        "/api/worker/register",
        &json!({"name": "other"}),
    )
    .json();
    let other = json!({"worker_id": reg["worker_id"], "worker_secret": reg["worker_secret"], "inputs": sim_inputs()});
    assert_eq!(post(srv.addr(), RUN, &path, &other).status, 400);
    assert!(store.get_run_inputs(&l.run_id).unwrap().is_none());
}

#[test]
fn a_caller_below_the_run_role_is_refused_and_a_bad_job_id_is_not_found() {
    let (srv, store) = boot();
    let l = lease(&srv);
    let path = format!("/api/worker/jobs/{}/inputs", l.job_id);
    let read = post(srv.addr(), READ, &path, &body(&l, sim_inputs()));
    assert_eq!(read.status, 403, "{}", read.body);
    let bad_job = post(
        srv.addr(),
        RUN,
        "/api/worker/jobs/not-a-job/inputs",
        &body(&l, sim_inputs()),
    );
    assert_eq!(bad_job.status, 404, "{}", bad_job.body);
    assert!(store.get_run_inputs(&l.run_id).unwrap().is_none());
}

#[test]
fn a_hostile_report_is_refused_without_being_repeated() {
    let (srv, store) = boot();
    let l = lease(&srv);
    let path = format!("/api/worker/jobs/{}/inputs", l.job_id);
    let plant = "TOPSECRET<script>alert(1)</script>";
    let cases = [
        json!({"scheme": plant, "fields": {"target": digest('a')}}),
        json!({"scheme": "sim/1", "fields": {plant: digest('a')}}),
        json!({"scheme": "sim/1", "fields": {"target": plant}}),
        json!({"scheme": "sim/1", "fields": {"target": digest('a')}, "missing": [plant]}),
        json!({"scheme": "hermit/1", "fields": {"hermit_sha": digest('a')}}),
        json!({"scheme": "sim/1", "fields": {}}),
        json!({"scheme": "sim/1", "fields": ["not", "an", "object"]}),
        json!({"scheme": 7, "fields": {}}),
        json!(plant),
    ];
    for inputs in cases {
        let r = post(srv.addr(), RUN, &path, &body(&l, inputs.clone()));
        assert_eq!(r.status, 400, "{inputs}: {}", r.body);
        assert!(
            !r.body.contains("TOPSECRET") && !r.body.contains("<script>"),
            "{inputs}: {}",
            r.body
        );
        assert!(r.body.len() < 300, "{inputs}: {}", r.body);
    }
    assert!(store.get_run_inputs(&l.run_id).unwrap().is_none());
}
