//! Tests for the simulated Dapr sidecar, using in-process stub applications.
use anthill_dapr::http::{request, serve, Request, Response};
use anthill_dapr::{run_simulation, AppLauncher, FaultPlan, Scenario, Verdict};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn hit(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, String, Vec<(String, String)>) {
    let r = request(
        addr,
        method,
        path,
        &[],
        body.as_bytes(),
        Duration::from_secs(5),
    )
    .expect("request");
    (
        r.status,
        String::from_utf8_lossy(&r.body).into_owned(),
        r.headers,
    )
}

// ---- mini HTTP layer ----
fn echo_server() -> (std::net::SocketAddr, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    serve(
        listener,
        Arc::new(|r: Request| {
            Response::json(
                200,
                &json!({"method": r.method, "path": r.path, "body": String::from_utf8_lossy(&r.body)}),
            )
        }),
        stop.clone(),
    );
    (addr, stop)
}

#[test]
fn http_round_trips_methods_paths_and_bodies() {
    let (addr, stop) = echo_server();
    let (status, body, _) = hit(addr, "POST", "/a/b?x=1", "hello");
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (v["method"].as_str(), v["path"].as_str(), v["body"].as_str()),
        (Some("POST"), Some("/a/b"), Some("hello")),
        "query string is split off the path"
    );
    stop.store(true, Ordering::SeqCst);
}

#[test]
fn http_rejects_malformed_and_oversized_requests() {
    use std::io::{Read, Write};
    let (addr, stop) = echo_server();
    let send = |raw: &str| {
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    };
    assert!(send("garbage\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert!(send("POST / HTTP/1.1\r\nContent-Length: 99999999\r\n\r\n").starts_with("HTTP/1.1 413"));
    assert!(send("POST / HTTP/1.1\r\nContent-Length: abc\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert!(
        hit(addr, "GET", "/still-alive", "").0 == 200,
        "server survives bad clients"
    );
    stop.store(true, Ordering::SeqCst);
}

// ---- state store API ----
fn sidecar() -> (anthill_dapr::SidecarHandle, std::net::SocketAddr) {
    let h = anthill_dapr::start_sidecar("127.0.0.1:0", 1, FaultPlan::none()).unwrap();
    let addr = h.addr();
    (h, addr)
}

#[test]
fn state_store_get_save_delete_with_etags() {
    let (h, addr) = sidecar();
    assert_eq!(
        hit(addr, "GET", "/v1.0/state/s/k1", "").0,
        204,
        "missing key"
    );
    assert_eq!(
        hit(
            addr,
            "POST",
            "/v1.0/state/s",
            r#"[{"key":"k1","value":{"n":1}}]"#
        )
        .0,
        204
    );
    let (st, body, headers) = hit(addr, "GET", "/v1.0/state/s/k1", "");
    assert_eq!(
        (st, serde_json::from_str::<Value>(&body).unwrap()),
        (200, json!({"n": 1}))
    );
    let etag1 = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("etag"))
        .map(|(_, v)| v.clone())
        .expect("etag header");
    hit(
        addr,
        "POST",
        "/v1.0/state/s",
        r#"[{"key":"k1","value":{"n":2}}]"#,
    );
    let etag2 = hit(addr, "GET", "/v1.0/state/s/k1", "")
        .2
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("etag"))
        .map(|(_, v)| v.clone())
        .unwrap();
    assert_ne!(etag1, etag2, "etag changes on every write");
    let stale = format!(r#"[{{"key":"k1","value":3,"etag":"{etag1}"}}]"#);
    assert_eq!(
        hit(addr, "POST", "/v1.0/state/s", &stale).0,
        409,
        "stale etag conflicts"
    );
    let fresh = format!(r#"[{{"key":"k1","value":3,"etag":"{etag2}"}}]"#);
    assert_eq!(hit(addr, "POST", "/v1.0/state/s", &fresh).0, 204);
    assert_eq!(hit(addr, "DELETE", "/v1.0/state/s/k1", "").0, 204);
    assert_eq!(hit(addr, "GET", "/v1.0/state/s/k1", "").0, 204);
    h.stop();
}

#[test]
fn first_write_wins_concurrency_option() {
    let (h, addr) = sidecar();
    let body = r#"[{"key":"once","value":1,"options":{"concurrency":"first-write"}}]"#;
    assert_eq!(hit(addr, "POST", "/v1.0/state/s", body).0, 204);
    assert_eq!(
        hit(addr, "POST", "/v1.0/state/s", body).0,
        409,
        "second first-write on an existing key conflicts"
    );
    h.stop();
}

#[test]
fn state_api_validates_input() {
    let (h, addr) = sidecar();
    assert_eq!(hit(addr, "POST", "/v1.0/state/s", "not json").0, 400);
    assert_eq!(
        hit(addr, "POST", "/v1.0/state/s", r#"{"key":"x"}"#).0,
        400,
        "body must be an array"
    );
    assert_eq!(
        hit(addr, "POST", "/v1.0/state/s", r#"[{"value":1}]"#).0,
        400,
        "key required"
    );
    assert_eq!(hit(addr, "POST", "/v1.0/state/bad%20name", "[]").0, 400);
    assert_eq!(hit(addr, "GET", "/v1.0/state/s/", "").0, 400);
    assert_eq!(hit(addr, "GET", "/v1.0/unknown", "").0, 404);
    assert_eq!(hit(addr, "GET", "/v1.0/healthz", "").0, 204);
    h.stop();
}

#[test]
fn injected_state_faults_are_deterministic_per_seed() {
    let plan = FaultPlan {
        state_error: 0.5,
        ..FaultPlan::none()
    };
    let outcome = |seed| {
        let h = anthill_dapr::start_sidecar("127.0.0.1:0", seed, plan.clone()).unwrap();
        let codes: Vec<u16> = (0..20)
            .map(|i| {
                hit(
                    h.addr(),
                    "POST",
                    "/v1.0/state/s",
                    &format!(r#"[{{"key":"k{i}","value":1}}]"#),
                )
                .0
            })
            .collect();
        h.stop();
        codes
    };
    assert_eq!(outcome(5), outcome(5));
    assert_ne!(outcome(5), outcome(6));
    assert!(outcome(5).contains(&500) && outcome(5).contains(&204));
}

// ---- scenarios ----
#[test]
fn scenario_parsing_and_validation() {
    let s = Scenario::parse(
        r#"
[app]
command = ["python3", "app.py"]
[faults]
duplicate = 0.25
crash_app = 0.1
[[publish]]
pubsub = "pubsub"
topic = "orders"
data = { id = 1 }
count = 3
"#,
    )
    .unwrap();
    assert_eq!(s.faults.duplicate, 0.25);
    assert_eq!(s.total_messages(), 3);
    assert!(
        Scenario::parse("[faults]\nduplicate = 1.5\n").is_err(),
        "probabilities must be within 0..=1"
    );
    assert!(
        Scenario::parse("[[publish]]\ntopic = \"t\"\ncount = 100000\n").is_err(),
        "message count is bounded"
    );
    assert!(
        Scenario::parse("[[publish]]\npubsub = \"p\"\n").is_err(),
        "topic required"
    );
    assert!(Scenario::parse("not toml [").is_err());
}

// ---- simulator against stub apps ----
/// A consumer app. `idempotent` chooses how it deduplicates:
/// - true: first-write-wins marker in the Dapr state store (survives crashes)
/// - false: an in-memory set (lost when the app crashes) - the classic bug.
struct StubApp {
    idempotent: bool,
    never_ack: Option<i64>,
    stop: Arc<AtomicBool>,
    sidecar_port: u16,
    seen: Arc<Mutex<HashSet<i64>>>,
}

impl StubApp {
    fn new(idempotent: bool) -> StubApp {
        StubApp {
            idempotent,
            never_ack: None,
            stop: Arc::new(AtomicBool::new(false)),
            sidecar_port: 0,
            seen: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl AppLauncher for StubApp {
    fn start(&mut self, sidecar_port: u16) -> Result<u16, String> {
        self.sidecar_port = sidecar_port;
        self.stop = Arc::new(AtomicBool::new(false));
        self.seen = Arc::new(Mutex::new(HashSet::new())); // a (re)start loses memory
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().unwrap().port();
        let (idempotent, never_ack, seen) = (self.idempotent, self.never_ack, self.seen.clone());
        let sidecar: std::net::SocketAddr = format!("127.0.0.1:{sidecar_port}").parse().unwrap();
        serve(
            listener,
            Arc::new(
                move |r: Request| match (r.method.as_str(), r.path.as_str()) {
                    ("GET", "/dapr/subscribe") => Response::json(
                        200,
                        &json!([{"pubsubname": "pubsub", "topic": "orders", "route": "/orders"}]),
                    ),
                    ("POST", "/orders") => {
                        let event: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
                        let id = event["data"]["id"].as_i64().unwrap_or(-1);
                        if never_ack == Some(id) {
                            return Response::json(200, &json!({"status": "DROP"}));
                        }
                        let first = if idempotent {
                            let marker = format!(
                                r#"[{{"key":"seen-{id}","value":1,"options":{{"concurrency":"first-write"}}}}]"#
                            );
                            request(
                                sidecar,
                                "POST",
                                "/v1.0/state/s",
                                &[],
                                marker.as_bytes(),
                                Duration::from_secs(5),
                            )
                            .map(|x| x.status == 204)
                            .unwrap_or(false)
                        } else {
                            seen.lock().unwrap().insert(id)
                        };
                        if first {
                            let cur = request(
                                sidecar,
                                "GET",
                                &format!("/v1.0/state/s/effect-{id}"),
                                &[],
                                b"",
                                Duration::from_secs(5),
                            )
                            .ok();
                            let n = cur
                                .filter(|c| c.status == 200)
                                .and_then(|c| {
                                    String::from_utf8_lossy(&c.body).trim().parse::<i64>().ok()
                                })
                                .unwrap_or(0);
                            let body = format!(r#"[{{"key":"effect-{id}","value":{}}}]"#, n + 1);
                            let _ = request(
                                sidecar,
                                "POST",
                                "/v1.0/state/s",
                                &[],
                                body.as_bytes(),
                                Duration::from_secs(5),
                            );
                        }
                        Response::json(200, &json!({"status": "SUCCESS"}))
                    }
                    ("GET", "/anthill/check") => {
                        for id in 1..=3 {
                            let cur = request(
                                sidecar,
                                "GET",
                                &format!("/v1.0/state/s/effect-{id}"),
                                &[],
                                b"",
                                Duration::from_secs(5),
                            )
                            .ok();
                            let n = cur
                                .filter(|c| c.status == 200)
                                .and_then(|c| {
                                    String::from_utf8_lossy(&c.body).trim().parse::<i64>().ok()
                                })
                                .unwrap_or(0);
                            if n != 1 && never_ack != Some(id) {
                                return Response::text(
                                    500,
                                    &format!("order {id} was applied {n} times"),
                                );
                            }
                        }
                        Response::text(200, "ok")
                    }
                    _ => Response::text(404, "not found"),
                },
            ),
            self.stop.clone(),
        );
        Ok(port)
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(60));
    }
}

fn scenario(faults: &str) -> Scenario {
    Scenario::parse(&format!("[[publish]]\npubsub = \"pubsub\"\ntopic = \"orders\"\ndata = {{ id = 1 }}\n[[publish]]\npubsub = \"pubsub\"\ntopic = \"orders\"\ndata = {{ id = 2 }}\n[[publish]]\npubsub = \"pubsub\"\ntopic = \"orders\"\ndata = {{ id = 3 }}\n[faults]\n{faults}\n")).unwrap()
}

const CHAOS: &str = "duplicate = 0.6\nreorder = 0.5\ncrash_app = 0.35";

#[test]
fn a_correct_idempotent_app_survives_every_fault_seed() {
    for seed in 1..=15u64 {
        let mut app = StubApp::new(true);
        match run_simulation(seed, &scenario(CHAOS), &mut app) {
            Verdict::Ok(report) => assert!(report.delivered >= 3, "seed {seed}: {report:?}"),
            other => panic!("seed {seed}: {other:?}"),
        }
    }
}

#[test]
fn an_app_with_in_memory_dedupe_is_caught_when_it_crashes_mid_stream() {
    let failing: Vec<u64> = (1..=25u64)
        .filter(|seed| matches!(run_simulation(*seed, &scenario(CHAOS), &mut StubApp::new(false)), Verdict::Violation(m) if m.contains("applied 2 times") || m.contains("applied 3 times")))
        .collect();
    assert!(
        !failing.is_empty(),
        "crash + duplicate must expose the lost in-memory dedupe on some seed"
    );
    assert!(
        failing.len() < 25,
        "...but not on every seed: it depends on the faults drawn"
    );
}

#[test]
fn without_faults_even_the_naive_app_passes() {
    let mut app = StubApp::new(false);
    assert!(matches!(
        run_simulation(1, &scenario("duplicate = 0.0"), &mut app),
        Verdict::Ok(_)
    ));
}

#[test]
fn a_message_that_is_never_processed_is_a_violation() {
    let mut app = StubApp::new(true);
    app.never_ack = Some(2);
    let v = run_simulation(1, &scenario("duplicate = 0.0"), &mut app);
    assert!(
        matches!(&v, Verdict::Violation(m) if m.contains("never processed")),
        "{v:?}"
    );
}

#[test]
fn the_simulation_is_deterministic_for_a_seed() {
    let run = |seed| {
        format!(
            "{:?}",
            run_simulation(seed, &scenario(CHAOS), &mut StubApp::new(false))
        )
    };
    for seed in [3, 9, 17] {
        assert_eq!(run(seed), run(seed), "seed {seed}");
    }
}

#[test]
fn app_startup_failure_is_an_error_not_a_finding() {
    struct Broken;
    impl AppLauncher for Broken {
        fn start(&mut self, _p: u16) -> Result<u16, String> {
            Err("could not launch".to_string())
        }
        fn stop(&mut self) {}
    }
    assert!(
        matches!(run_simulation(1, &scenario("duplicate = 0.0"), &mut Broken), Verdict::Error(m) if m.contains("could not launch"))
    );
}
