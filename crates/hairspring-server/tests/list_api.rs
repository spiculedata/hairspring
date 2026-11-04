//! `GET /api/runs` and `GET /api/failures` over real HTTP: the role, the limit, the cursor, the filters, and a 400 in
//! JSON (that does not echo the value) for anything that is not valid.
use hairspring_engine::config::{Budget, Config};
use hairspring_engine::engine::Engine;
use hairspring_engine::mode::RunMode;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::{Service, StartRun};
use hairspring_store::{NewCheck, NewConfirmation, NewFailure, SqliteStore, Store};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const NDJSON: &str = "application/x-ndjson";

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

fn get(addr: SocketAddr, token: Option<&str>, accept: Option<&str>, path: &str) -> Resp {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut r = agent.get(format!("http://{addr}{path}"));
    if let Some(t) = token {
        r = r.header("Authorization", format!("Bearer {t}"));
    }
    if let Some(a) = accept {
        r = r.header("Accept", a);
    }
    let mut resp = r.call().expect("transport ok");
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

struct Fixture {
    server: RunningServer,
    runs: Vec<String>,
    failures: Vec<String>,
}

fn start_run(svc: &Service, engine: &Engine, mode: RunMode) -> String {
    svc.start_run_with_mode(
        engine,
        &StartRun {
            target: "lock-lease".into(),
            seed: Some(1),
            budget: Some(Budget {
                max_steps: 1000,
                max_trials: 3,
                wall_secs: 30,
            }),
        },
        mode,
    )
    .unwrap()
    .id
}

fn check(kind: &str, status: &str) -> NewCheck {
    NewCheck {
        kind: kind.into(),
        status: status.into(),
        passed: 1,
        failed: 0,
        inconclusive: 0,
        command: "hermit run".into(),
        detail_json: "{}".into(),
        artefacts_json: "[]".into(),
        environment_json: "{}".into(),
        started_at: 1,
        finished_at: 2,
    }
}

/// Five runs (the third is a sweep, the fifth is cancelled) and five failures on the first, the second confirmed.
fn boot() -> Fixture {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store.clone());
    let runs: Vec<String> = (0..5)
        .map(|i| {
            start_run(
                &svc,
                &engine,
                if i == 2 {
                    RunMode::Sweep
                } else {
                    RunMode::Search
                },
            )
        })
        .collect();
    assert!(store.cancel_run(&runs[4], 9_999_999_999_999).unwrap());
    let failures: Vec<String> = (0..5)
        .map(|i| {
            store
                .add_failure(
                    &NewFailure {
                        run_id: runs[0].clone(),
                        target: "lock-lease".into(),
                        message: "boom".into(),
                        original_len: 3,
                        shrunk_len: 1,
                        bundle_hash: "0".repeat(64),
                        steps_json: "[]".into(),
                    },
                    100 + i,
                )
                .unwrap()
                .id
        })
        .collect();
    let checks = vec![
        check("same_seed_rerun", "pass"),
        check("bundle_replay", "pass"),
    ];
    store
        .add_confirmation(
            &NewConfirmation {
                id: NewConfirmation::content_id(&failures[1], "r", true, &checks),
                failure_id: failures[1].clone(),
                runner: "r".into(),
                runner_trusted: true,
                checks,
            },
            200,
        )
        .unwrap();
    let auth = Auth::from_spec(&format!("{READ}:read")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    Fixture {
        server: start(cfg, svc, engine).expect("server starts"),
        runs,
        failures,
    }
}

fn ids(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("no `{key}` array in {v}"))
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect()
}

fn newest_first(ids: &[String]) -> Vec<String> {
    ids.iter().rev().cloned().collect()
}

#[test]
fn both_lists_need_a_token() {
    let f = boot();
    for path in ["/api/runs", "/api/failures"] {
        assert_eq!(get(f.server.addr(), None, None, path).status, 401, "{path}");
        assert_eq!(
            get(f.server.addr(), Some(READ), None, path).status,
            200,
            "{path}"
        );
    }
}

#[test]
fn runs_page_with_limit_and_next_cursor_and_no_repeats() {
    let f = boot();
    let addr = f.server.addr();
    let whole = get(addr, Some(READ), None, "/api/runs").json();
    assert_eq!(ids(&whole, "runs"), newest_first(&f.runs));
    assert!(whole.get("next_cursor").is_none(), "{whole}");

    let mut walked = Vec::new();
    let mut path = "/api/runs?limit=2".to_owned();
    loop {
        let page = get(addr, Some(READ), None, &path).json();
        walked.extend(ids(&page, "runs"));
        match page.get("next_cursor").and_then(Value::as_str) {
            Some(next) => path = format!("/api/runs?limit=2&cursor={next}"),
            None => break,
        }
    }
    assert_eq!(walked, newest_first(&f.runs));
}

#[test]
fn runs_filter_by_state_target_and_mode_and_the_filters_combine() {
    let f = boot();
    let addr = f.server.addr();
    let list = |q: &str| {
        ids(
            &get(addr, Some(READ), None, &format!("/api/runs?{q}")).json(),
            "runs",
        )
    };
    assert_eq!(list("state=cancelled"), vec![f.runs[4].clone()]);
    assert_eq!(list("mode=sweep"), vec![f.runs[2].clone()]);
    assert_eq!(list("mode=search&state=queued").len(), 3);
    assert_eq!(list("target=lock-lease").len(), 5);
    assert!(list("target=nothing-here").is_empty());
    assert!(list("mode=sweep&state=cancelled").is_empty());
    let sweep = get(addr, Some(READ), None, "/api/runs?mode=sweep").json();
    assert_eq!(
        sweep["runs"][0]["mode"], "sweep",
        "the stored mode is what is filtered on"
    );
}

#[test]
fn failures_page_and_filter_by_run_and_status() {
    let f = boot();
    let addr = f.server.addr();
    let list = |q: &str| get(addr, Some(READ), None, &format!("/api/failures?{q}")).json();
    assert_eq!(ids(&list(""), "failures"), newest_first(&f.failures));
    assert_eq!(
        ids(&list(&format!("run={}", f.runs[0])), "failures").len(),
        5
    );
    assert!(ids(&list(&format!("run={}", f.runs[1])), "failures").is_empty());

    let first = list("status=confirmed");
    assert_eq!(ids(&first, "failures"), vec![f.failures[1].clone()]);
    assert_eq!(first["failures"][0]["finding"]["status"], "confirmed");
    assert!(first.get("next_cursor").is_none());

    let page = list("limit=2");
    assert_eq!(
        ids(&page, "failures"),
        newest_first(&f.failures)[..2].to_vec()
    );
    assert_eq!(page["next_cursor"], "2");
    let rest = list("limit=2&cursor=2");
    assert_eq!(
        ids(&rest, "failures"),
        newest_first(&f.failures)[2..4].to_vec()
    );
}

#[test]
fn a_cursor_is_a_control_line_in_the_ndjson_form() {
    let f = boot();
    let r = get(
        f.server.addr(),
        Some(READ),
        Some(NDJSON),
        "/api/runs?limit=2",
    );
    assert_eq!(r.status, 200);
    assert_eq!(
        r.header("content-type"),
        Some("application/x-ndjson; charset=utf-8")
    );
    let lines: Vec<Value> = r
        .body
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3, "two runs and the control line");
    assert_eq!(lines[2]["_next"], "2");
}

#[test]
fn bad_parameters_are_a_json_400_that_does_not_echo_the_value() {
    let f = boot();
    let addr = f.server.addr();
    let secret = "zzz-do-not-echo";
    for (path, name) in [
        (format!("/api/runs?state={secret}"), "state"),
        (format!("/api/runs?mode={secret}"), "mode"),
        ("/api/runs?target=".to_owned(), "target"),
        (format!("/api/runs?target={}", "x".repeat(65)), "target"),
        ("/api/runs?target=a%00b".to_owned(), "target"),
        (format!("/api/runs?cursor={secret}"), "cursor"),
        ("/api/runs?cursor=-1".to_owned(), "cursor"),
        ("/api/runs?cursor=100001".to_owned(), "cursor"),
        (format!("/api/runs?limit={secret}"), "limit"),
        ("/api/runs?limit=-1".to_owned(), "limit"),
        ("/api/runs?limit=4294967296".to_owned(), "limit"),
        (format!("/api/failures?status={secret}"), "status"),
        (format!("/api/failures?run={secret}"), "run"),
        (
            format!("/api/failures?property={}", "p".repeat(129)),
            "property",
        ),
        ("/api/failures?property=a%00".to_owned(), "property"),
        (format!("/api/failures?cursor={secret}"), "cursor"),
        (format!("/api/failures?limit={secret}"), "limit"),
    ] {
        let r = get(addr, Some(READ), None, &path);
        assert_eq!(r.status, 400, "{path}");
        assert!(
            r.header("content-type")
                .is_some_and(|c| c.starts_with("application/json")),
            "{path}: {:?}",
            r.header("content-type")
        );
        let body = r.json();
        let message = body["error"].as_str().unwrap_or_else(|| panic!("{body}"));
        assert!(message.contains(&format!("`{name}`")), "{path}: {message}");
        assert!(!r.body.contains(secret), "{path}: {}", r.body);
    }
}

#[test]
fn an_oversized_limit_is_lowered_and_not_refused() {
    let f = boot();
    let r = get(f.server.addr(), Some(READ), None, "/api/runs?limit=1000000").json();
    assert_eq!(ids(&r, "runs").len(), 5);
    let r = get(f.server.addr(), Some(READ), None, "/api/runs?limit=0").json();
    assert_eq!(ids(&r, "runs").len(), 1);
}
