//! End-to-end tests against a real in-process server over real HTTP.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const ADMIN: &str = "admin-token-0123456789abcdef";

fn engine() -> Engine {
    let registry = Arc::new(default_registry());
    Engine::new(registry.clone(), Config::from_registry(&registry))
}

fn boot(auth: Auth, workers: usize, rate: u32) -> (RunningServer, Engine) {
    let engine = engine();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = workers;
    cfg.rate_limit_per_min = rate;
    (
        start(cfg, Service::new(store), engine.clone()).expect("server starts"),
        engine,
    )
}

fn tokens() -> Auth {
    Auth::from_spec(&format!("{READ}:read,{RUN}:run,{ADMIN}:admin")).unwrap()
}

struct Resp {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

struct Client {
    agent: ureq::Agent,
    base: String,
    token: Option<String>,
}

fn client(addr: SocketAddr, token: Option<&str>) -> Client {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    Client {
        agent,
        base: format!("http://{addr}"),
        token: token.map(str::to_string),
    }
}

impl Client {
    fn finish(&self, r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Resp {
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
    fn get(&self, path: &str) -> Resp {
        let mut r = self.agent.get(format!("{}{}", self.base, path));
        if let Some(t) = &self.token {
            r = r.header("Authorization", format!("Bearer {t}"));
        }
        self.finish(r.call())
    }
    fn post_raw(&self, path: &str, body: &str) -> Resp {
        let mut r = self
            .agent
            .post(format!("{}{}", self.base, path))
            .header("Content-Type", "application/json");
        if let Some(t) = &self.token {
            r = r.header("Authorization", format!("Bearer {t}"));
        }
        self.finish(r.send(body))
    }
    fn post(&self, path: &str, v: &Value) -> Resp {
        self.post_raw(path, &v.to_string())
    }
}

fn wait_for_state(c: &Client, run_id: &str, want: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let r = c.get(&format!("/api/runs/{run_id}")).json();
        if r["run"]["state"] == want {
            return r;
        }
        assert!(Instant::now() < deadline, "run never reached {want}: {r}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn health_is_public_and_responses_carry_security_headers() {
    let (srv, _) = boot(tokens(), 0, 0);
    let r = client(srv.addr(), None).get("/api/health");
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["status"], "ok");
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert!(r.header("cache-control").unwrap_or("").contains("no-store"));
}

#[test]
fn authentication_and_roles_are_enforced() {
    let (srv, _) = boot(tokens(), 0, 0);
    let anon = client(srv.addr(), None);
    assert_eq!(anon.get("/api/targets").status, 401);
    assert_eq!(
        anon.get("/api/targets").header("www-authenticate"),
        Some("Bearer")
    );
    assert_eq!(
        client(srv.addr(), Some("wrong-token-0123456789"))
            .get("/api/targets")
            .status,
        401
    );
    let read = client(srv.addr(), Some(READ));
    assert_eq!(read.get("/api/targets").status, 200);
    assert_eq!(
        read.post("/api/runs", &json!({"target": "lock-lease"}))
            .status,
        403
    );
    assert_eq!(
        read.post("/api/worker/register", &json!({"name": "w"}))
            .status,
        403
    );
    assert_eq!(
        read.post("/api/replay", &json!({"bundle": "x"})).status,
        403,
        "replay runs target code, so it needs the run role"
    );
    let run = client(srv.addr(), Some(RUN));
    assert_eq!(
        run.post("/api/runs", &json!({"target": "lock-lease"}))
            .status,
        201
    );
}

#[test]
fn auth_spec_parsing_is_strict() {
    assert!(
        Auth::from_spec("short:read").is_err(),
        "weak tokens rejected"
    );
    assert!(
        Auth::from_spec("0123456789abcdef0123:root").is_err(),
        "unknown role rejected"
    );
    assert!(
        Auth::from_spec("0123456789abcdef0123").is_err(),
        "role required"
    );
    assert!(Auth::from_spec("").is_err());
    assert!(Auth::from_spec("0123456789abcdef0123:admin").is_ok());
}

#[test]
fn refuses_to_listen_publicly_without_auth() {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let cfg = ServerConfig::new("0.0.0.0:0".parse().unwrap(), Auth::open());
    let err = start(cfg, Service::new(store.clone()), engine())
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains("token"), "{err}");
    let ok = ServerConfig::new(
        "0.0.0.0:0".parse().unwrap(),
        Auth::from_spec("0123456789abcdef0123:admin").unwrap(),
    );
    assert!(start(ok, Service::new(store), engine()).is_ok());
}

#[test]
fn full_lifecycle_over_http() {
    let (srv, _) = boot(tokens(), 2, 0);
    let c = client(srv.addr(), Some(ADMIN));
    assert!(c.get("/api/targets").json()["targets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "lock-lease"));
    let created = c.post("/api/runs", &json!({"target": "lock-lease", "seed": 7}));
    assert_eq!(created.status, 201, "{}", created.body);
    let id = created.json()["run"]["id"].as_str().unwrap().to_string();
    let done = wait_for_state(&c, &id, "failed");
    let fid = done["failures"][0]["id"].as_str().unwrap();
    let f = c.get(&format!("/api/failures/{fid}")).json();
    assert!(!f["steps"].as_array().unwrap().is_empty());
    let hash = f["record"]["bundle_hash"].as_str().unwrap();
    let bundle = c.get(&format!("/api/bundles/{hash}"));
    assert_eq!(bundle.status, 200);
    assert!(bundle.body.starts_with("hairspring-bundle v2"));
    assert!(bundle
        .header("content-type")
        .unwrap()
        .starts_with("text/plain"));
    let replay = c
        .post("/api/replay", &json!({"bundle": bundle.body}))
        .json();
    assert_eq!(replay["reproduced"], true);
    assert_eq!(
        c.get("/api/runs?limit=5").json()["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        c.get("/api/failures").json()["failures"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!c.get("/api/workers").json()["workers"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn input_validation_and_error_shapes() {
    let (srv, _) = boot(tokens(), 0, 0);
    let c = client(srv.addr(), Some(ADMIN));
    assert_eq!(c.post("/api/runs", &json!({})).status, 400);
    assert_eq!(c.post("/api/runs", &json!({"target": "nope"})).status, 400);
    assert_eq!(
        c.post("/api/runs", &json!({"target": "x".repeat(500)}))
            .status,
        400
    );
    assert_eq!(c.post_raw("/api/runs", "{broken").status, 400);
    assert_eq!(
        c.post("/api/replay", &json!({"bundle": "garbage"})).status,
        400
    );
    assert_eq!(c.get("/api/runs/not-a-run-id").status, 404);
    assert_eq!(c.get("/api/runs/run_0123456789abcdef").status, 404);
    assert_eq!(c.get("/api/bundles/zzzz").status, 404);
    assert_eq!(c.get("/api/failures/..%2f..%2fetc").status, 404);
    let err = c.post("/api/runs", &json!({"target": "nope"})).json();
    assert!(err["error"].as_str().unwrap().contains("nope"));
    let big = json!({"bundle": "x".repeat(2 * 1024 * 1024 + 4096)});
    assert_eq!(c.post("/api/replay", &big).status, 413);
}

#[test]
fn cancel_a_queued_run() {
    let (srv, _) = boot(tokens(), 0, 0);
    let c = client(srv.addr(), Some(RUN));
    let id = c.post("/api/runs", &json!({"target": "lock-lease"})).json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        c.post(&format!("/api/runs/{id}/cancel"), &json!({})).json()["cancelled"],
        true
    );
    assert_eq!(
        c.get(&format!("/api/runs/{id}")).json()["run"]["state"],
        "cancelled"
    );
    assert_eq!(
        c.post(&format!("/api/runs/{id}/cancel"), &json!({})).json()["cancelled"],
        false
    );
}

#[test]
fn sse_streams_events_until_the_run_finishes() {
    let (srv, _) = boot(tokens(), 1, 0);
    let c = client(srv.addr(), Some(ADMIN));
    let id = c.post("/api/runs", &json!({"target": "pubsub-atomic", "seed": 1, "budget": {"max_steps": 20000, "max_trials": 1, "wall_secs": 20}})).json()["run"]["id"].as_str().unwrap().to_string();
    let r = c.get(&format!("/api/runs/{id}/events"));
    assert_eq!(r.status, 200);
    assert!(r
        .header("content-type")
        .unwrap()
        .starts_with("text/event-stream"));
    assert!(
        r.body.contains("event: finished") && r.body.contains("event: started"),
        "{}",
        r.body
    );
}

#[test]
fn mcp_over_http_respects_roles() {
    let (srv, _) = boot(tokens(), 1, 0);
    let rpc = |c: &Client, v: Value| c.post("/mcp", &v);
    let read = client(srv.addr(), Some(READ));
    let init = rpc(
        &read,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
    );
    assert_eq!(init.status, 200);
    assert_eq!(init.json()["result"]["serverInfo"]["name"], "hairspring");
    let note = rpc(
        &read,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    assert_eq!(note.status, 202);
    assert!(note.body.is_empty());
    let denied = rpc(
        &read,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "start_run", "arguments": {"target": "lock-lease"}}}),
    );
    assert_eq!(denied.json()["result"]["isError"], true);
    let run = client(srv.addr(), Some(RUN));
    let ok = rpc(
        &run,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "start_run", "arguments": {"target": "lock-lease", "seed": 7, "wait": true}}}),
    );
    assert_eq!(
        ok.json()["result"]["structuredContent"]["run"]["state"],
        "failed",
        "{}",
        ok.body
    );
    assert_eq!(
        client(srv.addr(), None)
            .post(
                "/mcp",
                &json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})
            )
            .status,
        401
    );
}

#[test]
fn a_remote_worker_executes_queued_runs() {
    let (srv, engine) = boot(tokens(), 0, 0);
    let stop = Arc::new(AtomicBool::new(false));
    let mut cfg =
        hairspring_worker::WorkerConfig::new(format!("http://{}", srv.addr()), "remote-test");
    cfg.token = Some(RUN.to_string());
    cfg.poll = Duration::from_millis(50);
    let (stop2, cfg2) = (stop.clone(), cfg.clone());
    let handle = std::thread::spawn(move || hairspring_worker::run_loop(&engine, &cfg2, &stop2));
    let c = client(srv.addr(), Some(RUN));
    let id = c
        .post("/api/runs", &json!({"target": "pubsub-buggy", "seed": 5}))
        .json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let done = wait_for_state(&c, &id, "failed");
    assert_eq!(done["failures"].as_array().unwrap().len(), 1);
    let workers = c.get("/api/workers").json();
    assert!(workers["workers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["name"] == "remote-test"));
    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap().unwrap();
}

#[test]
fn worker_endpoints_enforce_lease_ownership() {
    let (srv, _) = boot(tokens(), 0, 0);
    let c = client(srv.addr(), Some(RUN));
    let run_id = c
        .post("/api/runs", &json!({"target": "pubsub-atomic", "seed": 1}))
        .json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let w1 = c
        .post("/api/worker/register", &json!({"name": "w1"}))
        .json()["worker_id"]
        .as_str()
        .unwrap()
        .to_string();
    let w2 = c
        .post("/api/worker/register", &json!({"name": "w2"}))
        .json()["worker_id"]
        .as_str()
        .unwrap()
        .to_string();
    let job_id = c
        .post("/api/worker/lease", &json!({"worker_id": w1}))
        .json()["job_id"]
        .as_str()
        .expect("job leased")
        .to_string();
    let outcome = json!({"status": "passed", "steps_executed": 1, "trials": 1, "elapsed_ms": 1, "failure": null, "error": null});
    // w2 is a REAL registered worker with a valid id: it must still be refused because w1 holds the lease.
    for action in ["complete", "fail", "progress", "heartbeat"] {
        let body = match action {
            "complete" => json!({"worker_id": w2, "outcome": outcome}),
            "fail" => json!({"worker_id": w2, "error": "x"}),
            "progress" => json!({"worker_id": w2, "steps": 1, "trials": 1}),
            _ => json!({"worker_id": w2}),
        };
        let r = c.post(&format!("/api/worker/jobs/{job_id}/{action}"), &body);
        let rejected = r.status == 400
            || (action == "heartbeat" && r.json()["ok"] == false)
            || (action == "fail" && r.status == 200);
        assert!(
            rejected,
            "{action} by a non-owner must be rejected or a no-op: {} {}",
            r.status, r.body
        );
    }
    assert_eq!(
        c.get(&format!("/api/runs/{run_id}")).json()["run"]["state"],
        "running",
        "the non-owner changed nothing"
    );
    assert_eq!(
        c.post("/api/worker/lease", &json!({"worker_id": w2}))
            .status,
        204,
        "queue empty: the job is still leased by w1"
    );
    let ok = c.post(
        &format!("/api/worker/jobs/{job_id}/complete"),
        &json!({"worker_id": w1, "outcome": outcome}),
    );
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert_eq!(
        c.get(&format!("/api/runs/{run_id}")).json()["run"]["state"],
        "passed"
    );
}

#[test]
fn rate_limiting_returns_429() {
    let (srv, _) = boot(Auth::open(), 0, 5);
    let c = client(srv.addr(), None);
    let statuses: Vec<u16> = (0..9).map(|_| c.get("/api/targets").status).collect();
    assert!(statuses.iter().take(5).all(|s| *s == 200), "{statuses:?}");
    assert!(statuses.contains(&429), "{statuses:?}");
    assert_eq!(
        c.get("/api/health").status,
        200,
        "health stays reachable for probes"
    );
}

#[test]
fn ui_is_served_with_spa_fallback_and_no_directory_escape() {
    let (srv, _) = boot(tokens(), 0, 0);
    let c = client(srv.addr(), None);
    let root = c.get("/");
    assert_eq!(root.status, 200);
    assert!(root
        .header("content-type")
        .unwrap()
        .starts_with("text/html"));
    assert!(root.body.contains("Hairspring"));
    assert_eq!(
        c.get("/runs/run_0123456789abcdef").status,
        200,
        "client-side routes fall back to the app shell"
    );
    assert_ne!(c.get("/../../etc/passwd").status, 500);
    assert!(!c.get("/..%2f..%2fetc/passwd").body.contains("root:"));
}

// ---- hardening (security review) ----
use std::io::{Read, Write};
use std::net::TcpStream;

fn raw(addr: SocketAddr, request: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

#[test]
fn rate_limit_cannot_be_bypassed_with_garbage_tokens() {
    let (srv, _) = boot(tokens(), 0, 5);
    let statuses: Vec<u16> = (0..14)
        .map(|i| {
            client(srv.addr(), Some(&format!("garbage-token-number-{i:04}")))
                .get("/api/targets")
                .status
        })
        .collect();
    assert!(
        statuses.contains(&429),
        "unique bad tokens must still be throttled per client: {statuses:?}"
    );
    assert!(
        statuses.iter().all(|s| *s == 401 || *s == 429),
        "{statuses:?}"
    );
    let good = client(srv.addr(), Some(READ));
    assert_eq!(
        good.get("/api/targets").status,
        200,
        "a valid token has its own bucket and is unaffected by the noise"
    );
}

#[test]
fn mcp_requires_a_json_content_type() {
    let (srv, _) = boot(Auth::open(), 0, 0);
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let req = |ct: &str| {
        format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    };
    assert!(
        raw(srv.addr(), &req("text/plain")).starts_with("HTTP/1.1 415"),
        "text/plain is a CSRF-able 'simple' content type"
    );
    assert!(raw(srv.addr(), &req("application/x-www-form-urlencoded")).starts_with("HTTP/1.1 415"));
    assert!(raw(srv.addr(), &req("application/json")).starts_with("HTTP/1.1 200"));
    assert!(raw(srv.addr(), &req("application/json; charset=utf-8")).starts_with("HTTP/1.1 200"));
}

#[test]
fn open_loopback_mode_rejects_foreign_hosts_and_origins() {
    let (srv, _) = boot(Auth::open(), 0, 0);
    let get = |extra: &str, host: &str| {
        raw(
            srv.addr(),
            &format!(
                "GET /api/targets HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n"
            ),
        )
    };
    assert!(get("", &format!("127.0.0.1:{}", srv.addr().port())).starts_with("HTTP/1.1 200"));
    assert!(get("", &format!("localhost:{}", srv.addr().port())).starts_with("HTTP/1.1 200"));
    assert!(
        get("", "evil.example").starts_with("HTTP/1.1 403"),
        "DNS rebinding: unknown Host"
    );
    let host = format!("127.0.0.1:{}", srv.addr().port());
    assert!(
        get("Origin: http://evil.example\r\n", &host).starts_with("HTTP/1.1 403"),
        "cross-site Origin"
    );
    assert!(
        get(&format!("Origin: http://{host}\r\n"), &host).starts_with("HTTP/1.1 200"),
        "same-origin is fine"
    );
}

#[test]
fn tokens_make_origin_checks_unnecessary_because_browsers_cannot_attach_them() {
    let (srv, _) = boot(tokens(), 0, 0);
    let r = raw(srv.addr(), &format!("GET /api/targets HTTP/1.1\r\nHost: api.example.com\r\nAuthorization: Bearer {READ}\r\nConnection: close\r\n\r\n"));
    assert!(
        r.starts_with("HTTP/1.1 200"),
        "token auth works behind any hostname: {r}"
    );
}

#[test]
fn worker_ids_are_hidden_from_read_only_tokens() {
    let (srv, _) = boot(tokens(), 0, 0);
    let run = client(srv.addr(), Some(RUN));
    let id = run
        .post("/api/worker/register", &json!({"name": "w"}))
        .json()["worker_id"]
        .as_str()
        .unwrap()
        .to_string();
    let as_read = client(srv.addr(), Some(READ)).get("/api/workers");
    assert!(
        !as_read.body.contains(&id),
        "a read token must not learn worker ids: {}",
        as_read.body
    );
    assert_eq!(as_read.json()["workers"][0]["name"], "w");
    assert!(
        run.get("/api/workers").body.contains(&id),
        "run tokens may see them"
    );
}

#[test]
fn concurrent_event_streams_are_capped() {
    let engine = engine();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), Auth::open());
    cfg.embedded_workers = 1;
    cfg.rate_limit_per_min = 0;
    cfg.max_sse_streams = 1;
    let srv = start(cfg, Service::new(store), engine).unwrap();
    let c = client(srv.addr(), None);
    let id = c.post("/api/runs", &json!({"target": "pubsub-atomic", "seed": 1, "budget": {"max_steps": 40000000, "max_trials": 1000, "wall_secs": 30}})).json()["run"]["id"].as_str().unwrap().to_string();
    wait_for_state(&c, &id, "running");
    let addr = srv.addr();
    let first = std::thread::spawn({
        let id = id.clone();
        move || {
            client(addr, None)
                .get(&format!("/api/runs/{id}/events"))
                .status
        }
    });
    std::thread::sleep(Duration::from_millis(500)); // let the first stream attach
    assert_eq!(
        c.get(&format!("/api/runs/{id}/events")).status,
        429,
        "second concurrent stream is refused"
    );
    assert_eq!(
        c.post(&format!("/api/runs/{id}/cancel"), &json!({})).json()["cancelled"],
        true
    );
    assert_eq!(
        first.join().unwrap(),
        200,
        "the first stream ends once the run does"
    );
}

#[test]
fn a_cancelled_running_run_is_stopped_by_the_embedded_worker() {
    let (srv, _) = boot(Auth::open(), 1, 0);
    let c = client(srv.addr(), None);
    let id = c.post("/api/runs", &json!({"target": "pubsub-atomic", "seed": 1, "budget": {"max_steps": 40000000, "max_trials": 1000, "wall_secs": 30}})).json()["run"]["id"].as_str().unwrap().to_string();
    wait_for_state(&c, &id, "running");
    let t0 = Instant::now();
    c.post(&format!("/api/runs/{id}/cancel"), &json!({}));
    wait_for_state(&c, &id, "cancelled");
    // the worker must free itself: a follow-up run completes quickly
    let id2 = c
        .post("/api/runs", &json!({"target": "lock-lease", "seed": 7}))
        .json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    wait_for_state(&c, &id2, "failed");
    assert!(
        t0.elapsed() < Duration::from_secs(20),
        "worker was freed by cancellation"
    );
}
