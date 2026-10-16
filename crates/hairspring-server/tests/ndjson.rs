//! NDJSON content negotiation on the list endpoints, over real HTTP. One line per item, then one control line
//! `{"_next": ...}`; errors and every other response stay JSON.
use hairspring_engine::config::{Budget, Config};
use hairspring_engine::engine::Engine;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_props::{RunCatalog, TrialEvents, TrialProps};
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::{Service, StartRun};
use hairspring_store::{SqliteStore, Store};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const NDJSON: &str = "application/x-ndjson";
/// Guest text with markup, quotes and a look-alike control line. (The service already rejects control characters in
/// guest text, and JSON escapes them anyway, so no guest string can contain a raw line break.)
const HOSTILE: &str = "<script>alert(1)</script> \"{\"_next\":1}";

struct Resp {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
    /// Every line of an NDJSON body, each parsed on its own. The body must end in a line feed.
    fn lines(&self) -> Vec<Value> {
        assert!(self.body.ends_with('\n'), "no final line feed");
        self.body
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad line ({e}): {l}")))
            .collect()
    }
    fn is_ndjson(&self) -> bool {
        self.header("content-type") == Some("application/x-ndjson; charset=utf-8")
    }
}

fn get(addr: SocketAddr, token: Option<&str>, accept: Option<&str>, path: &str) -> Resp {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
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

fn props(falses: u64) -> TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let witness = if falses > 0 {
        r#"{"thread":"w-0","ord":5,"vals":[1,2,3,4]}"#
    } else {
        "null"
    };
    let text = [
        ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
        ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"b.broken","type":"always","must_hit":true,"msg":{}}}"#, serde_json::to_string(HOSTILE).unwrap())),
        ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"b.broken","hits":4,"trues":{},"falses":{falses},"first_false":{witness},"first_true":null}}"#, 4 - falses)),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
    ]
    .concat();
    TrialEvents::from_output(text.as_bytes()).fold()
}

fn start_one(svc: &Service, engine: &Engine) -> hairspring_store::RunRecord {
    svc.start_run(
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
    )
    .unwrap()
}

/// A finished run of three trials; the second failed. Returns the run id.
fn seed_run(svc: &Service, engine: &Engine) -> String {
    let run = start_one(svc, engine);
    let worker = Service::local_worker_id();
    let job = svc.lease(&worker).unwrap().expect("a job");
    let mut catalog = RunCatalog::new();
    let mut reports = Vec::new();
    for (seq, falses) in [0u64, 2, 0].into_iter().enumerate() {
        let p = props(falses);
        let rec = std::mem::take(&mut catalog).record(&p, false, 16384);
        catalog = rec.catalog;
        let violation = falses > 0;
        reports.push(TrialReport {
            seq: seq as u64,
            seed: 10 + seq as u64,
            verdict: if violation {
                ReportedVerdict::Violation
            } else {
                ReportedVerdict::Pass
            },
            reason: if violation {
                format!("{HOSTILE} failed")
            } else {
                String::new()
            },
            exit_code: Some(0),
            signal: None,
            behaviour_hash: Some("00aa11bb22cc33dd".into()),
            schedule_fp: Some(format!("{:016x}", seq % 2)),
            syscalls: 1,
            work_ops: 1,
            elapsed_ms: 1,
            catalog: rec.updates,
            digest: rec.digest.encode(),
            detail: None,
            output: None,
        });
    }
    svc.ingest_trials(&job.job_id, &worker, &reports).unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(3),
        ..RunOutcome::passed(3, 3, 5)
    };
    svc.complete(&job.job_id, &worker, &outcome).unwrap();
    run.id
}

fn boot_with(extra_runs: usize, rate_limit_per_min: u32) -> (RunningServer, String) {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store);
    let run = seed_run(&svc, &engine);
    svc.register_remote_worker("remote", "{}").unwrap();
    for _ in 0..extra_runs {
        start_one(&svc, &engine);
    }
    let auth = Auth::from_spec(&format!("{READ}:read,{RUN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = rate_limit_per_min;
    (start(cfg, svc, engine).expect("server starts"), run)
}

fn boot() -> (RunningServer, String) {
    boot_with(0, 0)
}

/// Each list endpoint with the key its JSON form keeps the items under.
fn list_endpoints(run: &str) -> Vec<(String, &'static str)> {
    vec![
        ("/api/targets".into(), "targets"),
        ("/api/runs".into(), "runs"),
        ("/api/failures".into(), "failures"),
        ("/api/workers".into(), "workers"),
        (format!("/api/runs/{run}/properties"), "properties"),
        (format!("/api/runs/{run}/trials"), "trials"),
        (format!("/api/runs/{run}/coverage"), "points"),
    ]
}

fn paths_with(v: &Value, needle: &str, path: &str, out: &mut Vec<String>) {
    match v {
        Value::String(s) if s.contains(needle) => out.push(path.to_owned()),
        Value::Array(a) => a
            .iter()
            .for_each(|x| paths_with(x, needle, &format!("{path}[]"), out)),
        Value::Object(o) => o
            .iter()
            .for_each(|(k, x)| paths_with(x, needle, &format!("{path}.{k}"), out)),
        _ => {}
    }
}

#[test]
fn every_list_endpoint_answers_ndjson_items_then_a_control_line() {
    let (server, run) = boot();
    for (path, key) in list_endpoints(&run) {
        let json = get(server.addr(), Some(READ), None, &path).json();
        let r = get(server.addr(), Some(READ), Some(NDJSON), &path);
        assert_eq!(r.status, 200, "{path}");
        assert!(r.is_ndjson(), "{path}: {:?}", r.header("content-type"));
        assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(r.header("cache-control"), Some("no-store"));
        assert_eq!(r.header("vary"), Some("Accept"), "{path}");
        assert_eq!(
            get(server.addr(), Some(READ), None, &path).header("vary"),
            Some("Accept"),
            "{path}: the JSON form varies on Accept too"
        );
        let lines = r.lines();
        let (control, items) = lines.split_last().expect("a control line");
        assert_eq!(
            items,
            json[key].as_array().unwrap().as_slice(),
            "{path}: items are the JSON form's items, in order"
        );
        assert!(
            control.get("_next").is_some_and(Value::is_null),
            "{path}: {control}"
        );
        assert!(items.iter().all(|i| i.get("_next").is_none()), "{path}");
    }
}

#[test]
fn the_seeded_run_is_listed_as_one_line() {
    let (server, run) = boot();
    let lines = get(server.addr(), Some(READ), Some(NDJSON), "/api/runs").lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0]["id"], run);
    assert_eq!(lines[1], serde_json::json!({ "_next": null }));
}

#[test]
fn a_page_with_more_carries_the_cursor_and_it_resumes() {
    let (server, run) = boot();
    let path = format!("/api/runs/{run}/trials");
    let first = get(
        server.addr(),
        Some(READ),
        Some(NDJSON),
        &format!("{path}?limit=2"),
    )
    .lines();
    assert_eq!(first.len(), 3, "{first:?}");
    let cursor = first[2]["_next"].as_str().expect("a cursor").to_owned();
    assert_eq!(
        first[2].as_object().unwrap().keys().next().unwrap(),
        "_next"
    );
    let second = get(
        server.addr(),
        Some(READ),
        Some(NDJSON),
        &format!("{path}?limit=2&cursor={cursor}"),
    )
    .lines();
    assert_eq!(second.len(), 2, "{second:?}");
    assert!(second[1]["_next"].is_null());
    let seqs: Vec<_> = first[..2].iter().chain(&second[..1]).collect();
    assert_ne!(seqs[0], seqs[2], "the second page continues, not repeats");
}

#[test]
fn page_level_labels_ride_on_the_control_line_not_the_items() {
    let (server, run) = boot();
    let lines = get(
        server.addr(),
        Some(READ),
        Some(NDJSON),
        &format!("/api/runs/{run}/properties"),
    )
    .lines();
    let control = lines.last().unwrap();
    let page = &control["page"];
    for key in [
        "claim",
        "headline",
        "counts_by_status",
        "total",
        "guest_note",
    ] {
        assert!(page.get(key).is_some(), "page.{key} missing: {control}");
    }
    assert!(page.get("properties").is_none() && page.get("next_cursor").is_none());
    assert!(lines[0].get("claim").is_none());
}

#[test]
fn other_accept_values_fall_back_to_json() {
    let (server, _) = boot();
    for accept in [
        "application/xml",
        "garbage;;;,,",
        "*/*",
        "application/json",
        "application/x-ndjson;q=0",
        "application/json, application/x-ndjson",
        "application/x-ndjsonx",
        "",
    ] {
        let r = get(server.addr(), Some(READ), Some(accept), "/api/runs");
        assert_eq!(r.status, 200, "{accept:?}");
        assert!(!r.is_ndjson(), "{accept:?}");
        assert!(r.json()["runs"].is_array(), "{accept:?}");
    }
    let r = get(server.addr(), Some(READ), None, "/api/runs");
    assert!(r.json()["runs"].is_array());
}

#[test]
fn ndjson_is_chosen_by_the_header_alone() {
    let (server, _) = boot();
    for accept in [
        "Application/X-NDJSON",
        "application/x-ndjson; charset=utf-8",
        "text/html, application/x-ndjson;q=0.5",
        "application/json;q=0.5, application/x-ndjson",
    ] {
        let r = get(server.addr(), Some(READ), Some(accept), "/api/runs");
        assert!(r.is_ndjson(), "{accept:?}: {:?}", r.header("content-type"));
    }
    let r = get(server.addr(), Some(READ), None, "/api/runs?format=ndjson");
    assert!(!r.is_ndjson(), "there is no query parameter form");
}

#[test]
fn errors_stay_json_with_the_right_status() {
    let (server, run) = boot();
    for (path, token, want) in [
        ("/api/runs".to_owned(), None, 401),
        (
            "/api/runs".to_owned(),
            Some("wrong-token-0123456789abcdef"),
            401,
        ),
        (format!("/api/runs/{run}/trials?limit=abc"), Some(READ), 400),
        ("/api/runs/run_nope/trials".to_owned(), Some(READ), 404),
        (
            "/api/runs/run_00000000000000000000000000000000/properties".to_owned(),
            Some(READ),
            404,
        ),
    ] {
        let r = get(server.addr(), token, Some(NDJSON), &path);
        assert_eq!(r.status, want, "{path}");
        assert!(!r.is_ndjson(), "{path}");
        assert_eq!(r.header("content-type"), Some("application/json"), "{path}");
        assert!(r.json()["error"].is_string(), "{path}");
    }
}

#[test]
fn a_rate_limited_request_is_still_a_json_429() {
    let (server, _) = boot_with(0, 3);
    let statuses: Vec<(u16, bool)> = (0..6)
        .map(|_| {
            let r = get(server.addr(), Some(READ), Some(NDJSON), "/api/runs");
            (r.status, r.is_ndjson())
        })
        .collect();
    assert!(
        statuses[..3].iter().all(|s| *s == (200, true)),
        "{statuses:?}"
    );
    assert!(
        statuses[3..].iter().all(|s| *s == (429, false)),
        "{statuses:?}"
    );
}

#[test]
fn detail_endpoints_ignore_the_header() {
    let (server, run) = boot();
    for path in [
        format!("/api/runs/{run}"),
        format!("/api/runs/{run}/trials/1"),
        format!("/api/runs/{run}/properties/0"),
    ] {
        let r = get(server.addr(), Some(READ), Some(NDJSON), &path);
        assert_eq!(r.status, 200, "{path}");
        assert!(!r.is_ndjson(), "{path}");
        assert!(r.json().is_object());
    }
}

#[test]
fn hostile_guest_text_stays_inside_guest_and_never_breaks_a_line() {
    let (server, run) = boot();
    for path in [
        format!("/api/runs/{run}/trials"),
        format!("/api/runs/{run}/properties"),
    ] {
        let r = get(server.addr(), Some(READ), Some(NDJSON), &path);
        let raw_lines = r.body.split('\n').filter(|l| !l.is_empty()).count();
        let lines = r.lines();
        assert_eq!(lines.len(), raw_lines, "{path}: every line parses alone");
        let mut found = Vec::new();
        for line in &lines {
            paths_with(line, "<script>", "", &mut found);
            paths_with(line, "b.broken", "", &mut found);
        }
        assert!(!found.is_empty(), "{path}: the hostile text is present");
        for p in found {
            assert!(p.contains(".guest"), "{path}: outside the fence at {p}");
        }
        assert!(
            !lines.iter().any(
                |l| l.as_object().unwrap().len() == 1 && l.get("_next").is_some_and(|n| n == 1)
            ),
            "{path}: guest text forged a control line"
        );
        assert!(!r.body.contains("\r"), "{path}: raw carriage return");
    }
}

#[test]
fn workers_ids_stay_hidden_from_read_callers_in_ndjson_too() {
    let (server, _) = boot();
    let as_run = get(server.addr(), Some(RUN), None, "/api/workers").json();
    let id = as_run["workers"][0]["id"].as_str().expect("a worker id");
    let read = get(server.addr(), Some(READ), Some(NDJSON), "/api/workers");
    assert_eq!(read.lines().len(), 2, "{}", read.body);
    assert!(!read.body.contains(id));
    let run = get(server.addr(), Some(RUN), Some(NDJSON), "/api/workers");
    assert!(run.body.contains(id), "{}", run.body);
}

#[test]
fn a_huge_limit_is_clamped_and_never_streams_without_bound() {
    let (server, run) = boot_with(520, 0);
    let runs = get(
        server.addr(),
        Some(READ),
        Some(NDJSON),
        "/api/runs?limit=10000",
    )
    .lines();
    assert_eq!(runs.len(), 500 + 1, "runs are capped at 500 per request");
    let trials = get(
        server.addr(),
        Some(READ),
        Some(NDJSON),
        &format!("/api/runs/{run}/trials?limit=10000"),
    )
    .lines();
    assert!(trials.len() <= 500 + 1);
    let json = get(server.addr(), Some(READ), None, "/api/runs?limit=10000").json();
    assert_eq!(json["runs"].as_array().unwrap().len(), 500);
}
