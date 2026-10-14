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
    with_check: bool,
    stop: Arc<AtomicBool>,
    sidecar_port: u16,
    seen: Arc<Mutex<HashSet<i64>>>,
}

impl StubApp {
    fn new(idempotent: bool) -> StubApp {
        StubApp {
            idempotent,
            never_ack: None,
            with_check: true,
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
        let (idempotent, never_ack, seen, with_check) = (
            self.idempotent,
            self.never_ack,
            self.seen.clone(),
            self.with_check,
        );
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
                    ("GET", "/anthill/check") if with_check => {
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

// ---- review findings ----
#[test]
fn simulated_transit_drops_never_create_false_violations() {
    for seed in 1..=30u64 {
        let v = run_simulation(
            seed,
            &scenario("drop = 0.5\nduplicate = 0.3"),
            &mut StubApp::new(true),
        );
        assert!(
            matches!(v, Verdict::Ok(_)),
            "seed {seed}: the sim's own message loss must not be blamed on the app: {v:?}"
        );
    }
}

#[test]
fn running_out_of_simulation_budget_is_an_error_not_a_finding() {
    let sc = Scenario::parse(
        "[[publish]]\ntopic = \"orders\"\ndata = { id = 1 }\ncount = 3\n[limits]\nmax_rounds = 1\n",
    )
    .unwrap();
    let v = run_simulation(1, &sc, &mut StubApp::new(true));
    assert!(
        matches!(&v, Verdict::Error(m) if m.contains("budget")),
        "{v:?}"
    );
}

#[test]
fn a_missing_invariant_endpoint_is_reported_not_silently_passed() {
    let mut with = StubApp::new(true);
    let Verdict::Ok(r) = run_simulation(1, &scenario("duplicate = 0.0"), &mut with) else {
        panic!("should pass")
    };
    assert!(r.invariants_checked);
    let mut without = StubApp::new(true);
    without.with_check = false;
    let Verdict::Ok(r) = run_simulation(1, &scenario("duplicate = 0.0"), &mut without) else {
        panic!("should pass")
    };
    assert!(
        !r.invariants_checked,
        "the report must say that only delivery was checked"
    );
}

#[test]
fn state_keys_are_percent_decoded() {
    let (h, addr) = sidecar();
    assert_eq!(
        hit(
            addr,
            "POST",
            "/v1.0/state/s",
            r#"[{"key":"a b/c","value":7}]"#
        )
        .0,
        204
    );
    let (status, body, _) = hit(addr, "GET", "/v1.0/state/s/a%20b%2Fc", "");
    assert_eq!((status, body.trim()), (200, "7"));
    assert_eq!(
        hit(addr, "GET", "/v1.0/state/s/bad%zz", "").0,
        400,
        "malformed escapes are rejected"
    );
    h.stop();
}

#[test]
fn chunked_request_bodies_are_understood() {
    use std::io::{Read, Write};
    let (addr, stop) = echo_server();
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    assert!(
        out.starts_with("HTTP/1.1 200") && out.contains("hello world"),
        "{out}"
    );
    stop.store(true, Ordering::SeqCst);
}

#[test]
fn the_sidecar_refuses_browsers_and_foreign_hosts() {
    use std::io::{Read, Write};
    let (h, addr) = sidecar();
    let send = |extra: &str, host: &str| {
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(
            format!(
                "GET /v1.0/healthz HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    };
    assert!(send("", &addr.to_string()).starts_with("HTTP/1.1 204"));
    assert!(
        send("Origin: http://evil.example\r\n", &addr.to_string()).starts_with("HTTP/1.1 403"),
        "a web page must not drive the sidecar"
    );
    assert!(
        send("", "evil.example").starts_with("HTTP/1.1 403"),
        "DNS rebinding"
    );
    h.stop();
}

#[test]
fn slow_clients_cannot_hold_a_connection_past_the_request_deadline() {
    use anthill_dapr::http::{serve_with, ServeLimits};
    use std::io::{Read, Write};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    serve_with(
        listener,
        Arc::new(|_r: Request| Response::text(200, "ok")),
        stop.clone(),
        ServeLimits {
            request_deadline: Duration::from_millis(400),
            max_connections: 2,
        },
    );
    let mut slow = std::net::TcpStream::connect(addr).unwrap();
    slow.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let started = std::time::Instant::now();
    for _ in 0..10 {
        let _ = slow.write_all(b"G"); // one byte at a time, never finishing the request
        std::thread::sleep(Duration::from_millis(150));
    }
    let mut out = String::new();
    let _ = slow.read_to_string(&mut out);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "dribbling does not extend the deadline: {:?}",
        started.elapsed()
    );
    assert!(!out.contains("200 OK"));
    stop.store(true, Ordering::SeqCst);
}

#[test]
fn concurrent_connections_are_capped() {
    use anthill_dapr::http::{serve_with, ServeLimits};
    use std::io::{Read, Write};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    serve_with(
        listener,
        Arc::new(|_r: Request| Response::text(200, "ok")),
        stop.clone(),
        ServeLimits {
            request_deadline: Duration::from_secs(5),
            max_connections: 2,
        },
    );
    let _a = std::net::TcpStream::connect(addr).unwrap();
    let _b = std::net::TcpStream::connect(addr).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let mut c = std::net::TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let _ = c.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    let mut out = String::new();
    let _ = c.read_to_string(&mut out);
    assert!(
        out.starts_with("HTTP/1.1 503"),
        "the third concurrent connection is refused: {out}"
    );
    stop.store(true, Ordering::SeqCst);
}

#[test]
fn a_dribbling_server_cannot_stall_the_client_past_its_total_timeout() {
    use std::io::Write;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n");
            for _ in 0..40 {
                std::thread::sleep(Duration::from_millis(200));
                if s.write_all(b"x").is_err() {
                    break;
                }
            }
        }
    });
    let started = std::time::Instant::now();
    let r = request(addr, "GET", "/", &[], b"", Duration::from_secs(1));
    assert!(r.is_err(), "an incomplete, trickling response is a timeout");
    assert!(
        started.elapsed() < Duration::from_millis(2500),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn sidecar_state_is_bounded() {
    use anthill_dapr::SidecarLimits;
    let limits = SidecarLimits {
        max_keys_per_store: 3,
        max_stores: 2,
        max_total_bytes: 200,
        max_published: 2,
    };
    let h = anthill_dapr::start_sidecar_with("127.0.0.1:0", 1, FaultPlan::none(), limits).unwrap();
    let addr = h.addr();
    for k in 0..3 {
        assert_eq!(
            hit(
                addr,
                "POST",
                "/v1.0/state/s",
                &format!(r#"[{{"key":"k{k}","value":1}}]"#)
            )
            .0,
            204
        );
    }
    assert_eq!(
        hit(addr, "POST", "/v1.0/state/s", r#"[{"key":"k4","value":1}]"#).0,
        413,
        "key cap"
    );
    assert_eq!(
        hit(addr, "POST", "/v1.0/state/s", r#"[{"key":"k0","value":2}]"#).0,
        204,
        "overwriting an existing key is fine"
    );
    assert_eq!(
        hit(addr, "POST", "/v1.0/state/t", r#"[{"key":"a","value":1}]"#).0,
        204
    );
    assert_eq!(
        hit(addr, "POST", "/v1.0/state/u", r#"[{"key":"a","value":1}]"#).0,
        413,
        "store cap"
    );
    let big = format!(r#"[{{"key":"big","value":"{}"}}]"#, "x".repeat(300));
    assert_eq!(hit(addr, "POST", "/v1.0/state/t", &big).0, 413, "byte cap");
    assert_eq!(hit(addr, "POST", "/v1.0/publish/p/t", "1").0, 204);
    assert_eq!(hit(addr, "POST", "/v1.0/publish/p/t", "2").0, 204);
    assert_eq!(
        hit(addr, "POST", "/v1.0/publish/p/t", "3").0,
        429,
        "publish backlog cap"
    );
    h.stop();
}

#[test]
fn subscriptions_with_only_routing_rules_are_understood_and_unparseable_ones_are_errors() {
    struct RulesApp(Arc<AtomicBool>, bool);
    impl AppLauncher for RulesApp {
        fn start(&mut self, _p: u16) -> Result<u16, String> {
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
            let port = listener.local_addr().unwrap().port();
            let bad = self.1;
            self.0 = Arc::new(AtomicBool::new(false));
            serve(
                listener,
                Arc::new(
                    move |r: Request| match (r.method.as_str(), r.path.as_str()) {
                        ("GET", "/dapr/subscribe") if bad => Response::json(
                            200,
                            &json!([{"pubsubname": "pubsub", "topic": "orders"}]),
                        ),
                        ("GET", "/dapr/subscribe") => Response::json(
                            200,
                            &json!([{"pubsubname": "pubsub", "topic": "orders", "routes": {"rules": [{"match": "", "path": "/orders"}]}}]),
                        ),
                        ("POST", "/orders") => Response::json(200, &json!({"status": "SUCCESS"})),
                        _ => Response::text(404, "no"),
                    },
                ),
                self.0.clone(),
            );
            Ok(port)
        }
        fn stop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    assert!(matches!(
        run_simulation(
            1,
            &scenario("duplicate = 0.0"),
            &mut RulesApp(Arc::new(AtomicBool::new(false)), false)
        ),
        Verdict::Ok(_)
    ));
    let v = run_simulation(
        1,
        &scenario("duplicate = 0.0"),
        &mut RulesApp(Arc::new(AtomicBool::new(false)), true),
    );
    assert!(
        matches!(&v, Verdict::Error(m) if m.contains("subscription")),
        "{v:?}"
    );
}

#[cfg(unix)]
#[test]
fn the_process_launcher_kills_the_whole_group_and_passes_a_minimal_environment() {
    use anthill_dapr::scenario::AppSection;
    std::env::set_var("ANTHILL_SECRET_FOR_TEST", "leak-me");
    let dir = std::env::temp_dir().join(format!(
        "anthill-dapr-launch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let (pidfile, envfile) = (dir.join("pid"), dir.join("env"));
    let script = format!(
        "env > {}; sleep 31 & echo $! > {}; wait",
        envfile.display(),
        pidfile.display()
    );
    let section = AppSection {
        command: vec!["/bin/sh".into(), "-c".into(), script],
        env: [("MODE".to_string(), "test".to_string())].into(),
        ..AppSection::default()
    };
    let mut app = anthill_dapr::ProcessApp::new(section);
    app.start(4242).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !pidfile.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(100));
    let env = std::fs::read_to_string(&envfile).unwrap();
    assert!(
        !env.contains("ANTHILL_SECRET_FOR_TEST"),
        "host secrets must not reach the app:\n{env}"
    );
    assert!(
        env.contains("DAPR_HTTP_PORT=4242") && env.contains("MODE=test") && env.contains("PATH="),
        "{env}"
    );
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = |pid: i32| {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    assert!(alive(pid), "the grandchild runs while the app is up");
    app.stop();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while alive(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !alive(pid),
        "stopping the app must also stop what it started"
    );
}
