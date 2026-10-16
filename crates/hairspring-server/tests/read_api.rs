//! The read endpoints over real HTTP: roles, id validation, paging limits, the `guest` fence and download headers.
use hairspring_engine::config::{Budget, Config};
use hairspring_engine::engine::Engine;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_props::record::TrialRecord as KeptRecord;
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
const HOSTILE: &str = "<script>alert(1)</script>";
const OUTPUT: &str = "<img src=x onerror=alert(1)>\u{1b}[31mred";

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

fn get(addr: SocketAddr, token: Option<&str>, path: &str) -> Resp {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut r = agent.get(format!("http://{addr}{path}"));
    if let Some(t) = token {
        r = r.header("Authorization", format!("Bearer {t}"));
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

fn props(msg: &str, falses: u64) -> TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let witness = if falses > 0 {
        r#"{"thread":"w-0","ord":5,"vals":[1,2,3,4]}"#
    } else {
        "null"
    };
    let text = [
        ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
        ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"b.broken","type":"always","must_hit":true,"msg":{}}}"#, serde_json::to_string(msg).unwrap())),
        ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"b.broken","hits":{},"trues":{},"falses":{falses},"first_false":{witness},"first_true":null}}"#, 4, 4 - falses)),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
    ]
    .concat();
    TrialEvents::from_output(text.as_bytes()).fold()
}

/// A finished run of three trials; the second failed and kept its detail and output. Returns the run id.
fn seed_run(svc: &Service, engine: &Engine) -> String {
    let run = svc
        .start_run(
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
        .unwrap();
    let worker = Service::local_worker_id();
    let job = svc.lease(&worker).unwrap().expect("a job");
    let mut catalog = RunCatalog::new();
    let mut reports = Vec::new();
    for (seq, falses) in [0u64, 2, 0].into_iter().enumerate() {
        let p = props(HOSTILE, falses);
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
            detail: violation.then(|| String::from_utf8(KeptRecord::encode(&p).unwrap()).unwrap()),
            output: violation
                .then(|| serde_json::json!({ "stdout": OUTPUT, "stderr": "e" }).to_string()),
        });
    }
    svc.ingest_trials(&job.job_id, &worker, &reports).unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(3),
        ..RunOutcome::passed(3, 3, 5)
    };
    svc.complete(&job.job_id, &worker, &outcome).unwrap();
    assert_eq!(run.id, job.run_id);
    run.id
}

fn boot() -> (RunningServer, String) {
    boot_limited(0)
}

fn boot_limited(rate_limit_per_min: u32) -> (RunningServer, String) {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store);
    let run = seed_run(&svc, &engine);
    let auth = Auth::from_spec(&format!("{READ}:read,{RUN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = rate_limit_per_min;
    (start(cfg, svc, engine).expect("server starts"), run)
}

fn strings_containing(v: &Value, needle: &str, path: &str, out: &mut Vec<String>) {
    match v {
        Value::String(s) if s.contains(needle) => out.push(path.to_owned()),
        Value::Array(a) => a
            .iter()
            .for_each(|x| strings_containing(x, needle, &format!("{path}[]"), out)),
        Value::Object(o) => o
            .iter()
            .for_each(|(k, x)| strings_containing(x, needle, &format!("{path}.{k}"), out)),
        _ => {}
    }
}

#[test]
fn every_read_endpoint_needs_a_token_and_the_read_role_is_enough() {
    let (server, run) = boot();
    let addr = server.addr();
    for path in [
        format!("/api/runs/{run}/properties"),
        format!("/api/runs/{run}/properties/0"),
        format!("/api/runs/{run}/trials"),
        format!("/api/runs/{run}/trials/1"),
        format!("/api/runs/{run}/trials/1/output/stdout"),
        format!("/api/runs/{run}/coverage"),
        format!("/api/runs/{run}/strip"),
    ] {
        assert_eq!(get(addr, None, &path).status, 401, "{path}");
        assert_eq!(
            get(addr, Some("wrong-token-0123456789abcdef"), &path).status,
            401,
            "{path}"
        );
        assert_eq!(get(addr, Some(READ), &path).status, 200, "{path}");
    }
}

#[test]
fn ids_are_validated_before_anything_is_looked_up() {
    let (server, run) = boot();
    let addr = server.addr();
    for bad in [
        "nope",
        "run_xyz",
        "run_0123456789abcdeg",
        "..%2f..",
        "run_0123456789abcdef0",
    ] {
        for tail in ["properties", "trials", "coverage", "strip"] {
            let r = get(addr, Some(READ), &format!("/api/runs/{bad}/{tail}"));
            assert_eq!(r.status, 404, "{bad}/{tail}");
        }
    }
    assert_eq!(
        get(
            addr,
            Some(READ),
            "/api/runs/run_0000000000000000/properties"
        )
        .status,
        404
    );
    for bad in ["x", "-1", "1.5", "99999999999", "%00"] {
        assert_eq!(
            get(
                addr,
                Some(READ),
                &format!("/api/runs/{run}/properties/{bad}")
            )
            .status,
            404,
            "{bad}"
        );
        assert_eq!(
            get(addr, Some(READ), &format!("/api/runs/{run}/trials/{bad}")).status,
            404,
            "{bad}"
        );
    }
    assert_eq!(
        get(addr, Some(READ), &format!("/api/runs/{run}/properties/77")).status,
        404
    );
    assert_eq!(
        get(addr, Some(READ), &format!("/api/runs/{run}/trials/77")).status,
        404
    );
    assert_eq!(
        get(
            addr,
            Some(READ),
            &format!("/api/runs/{run}/trials/1/output/other")
        )
        .status,
        400
    );
}

#[test]
fn bad_query_parameters_are_a_400_that_does_not_echo_them() {
    let (server, run) = boot();
    let addr = server.addr();
    for q in [
        "status=%3Cscript%3E",
        "kind=zzz",
        "sort=zzz",
        "cursor=abc",
        "limit=abc",
        "limit=-1",
    ] {
        let r = get(addr, Some(READ), &format!("/api/runs/{run}/properties?{q}"));
        assert_eq!(r.status, 400, "{q}");
        assert!(
            !r.body.contains("script") && !r.body.contains("zzz"),
            "{}",
            r.body
        );
    }
    let r = get(
        addr,
        Some(READ),
        &format!("/api/runs/{run}/trials?verdict=zzz"),
    );
    assert_eq!(r.status, 400);
}

#[test]
fn the_property_table_is_served_with_rate_cells_and_a_headline() {
    let (server, run) = boot();
    let r = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/properties?limit=1000"),
    );
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["headline"], "1 property failed; first failing seed 11");
    assert_eq!(v["total"], 1);
    assert_eq!(v["properties"][0]["status"], "violation");
    assert_eq!(v["properties"][0]["first_failing_seed"], "11");
    assert_eq!(v["properties"][0]["rate"]["kind"], "rate");
    assert_eq!(v["claim"]["text"], Value::Null);
    assert!(v["claim"]["chips"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "label not declared"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
}

#[test]
fn guest_text_is_under_a_guest_object_on_every_endpoint() {
    let (server, run) = boot();
    for path in [
        format!("/api/runs/{run}"),
        format!("/api/runs/{run}/properties"),
        format!("/api/runs/{run}/properties/0?seeds=failing"),
        format!("/api/runs/{run}/trials"),
        format!("/api/runs/{run}/trials/1"),
    ] {
        let r = get(server.addr(), Some(READ), &path);
        assert_eq!(r.status, 200, "{path}");
        let v = r.json();
        let mut found = Vec::new();
        for needle in [HOSTILE, "b.broken"] {
            strings_containing(&v, needle, "", &mut found);
        }
        for p in found {
            assert!(
                p.contains(".guest"),
                "{path}: guest text outside the guest fence at {p}"
            );
        }
    }
}

#[test]
fn a_property_detail_pages_its_seeds_as_strings() {
    let (server, run) = boot();
    let r = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/properties/0?seeds=failing&limit=1"),
    );
    let v = r.json();
    assert_eq!(v["seeds"][0]["seed"], "11");
    assert_eq!(v["seeds_kind"], "failing");
}

#[test]
fn trials_are_listed_and_filtered() {
    let (server, run) = boot();
    let v = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/trials?verdict=violation"),
    )
    .json();
    assert_eq!(v["trials"].as_array().unwrap().len(), 1);
    assert_eq!(v["trials"][0]["seed"], "11");
    let v = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/trials?limit=2"),
    )
    .json();
    assert_eq!(v["trials"].as_array().unwrap().len(), 2);
    assert_eq!(v["next_cursor"], "1");
}

#[test]
fn a_trial_is_found_by_its_exact_seed_and_a_bad_seed_is_a_400() {
    let (server, run) = boot();
    let v = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/trials?seed=11&limit=1"),
    )
    .json();
    assert_eq!(v["trials"].as_array().unwrap().len(), 1);
    assert_eq!(v["trials"][0]["verdict"], "violation");
    for bad in ["x", "-1", "11%20", "99999999999999999999999"] {
        let r = get(
            server.addr(),
            Some(READ),
            &format!("/api/runs/{run}/trials?seed={bad}"),
        );
        assert_eq!(r.status, 400, "{bad}");
    }
}

#[test]
fn the_strip_is_a_code_per_trial_and_pages_with_a_cursor() {
    let (server, run) = boot();
    let v = get(server.addr(), Some(READ), &format!("/api/runs/{run}/strip")).json();
    assert_eq!(v["codes"], "pvp");
    assert_eq!(v["first_seq"], 0);
    assert_eq!(v["next_cursor"], Value::Null);
    assert!(v["legend"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["code"] == "v" && l["name"] == "violation"));
    let page = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/strip?limit=2"),
    )
    .json();
    assert_eq!(page["codes"], "pv");
    assert_eq!(page["next_cursor"], "1");
    let rest = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/strip?limit=2&cursor=1"),
    )
    .json();
    assert_eq!(
        (rest["first_seq"].as_u64(), rest["codes"].as_str()),
        (Some(2), Some("p"))
    );
}

#[test]
fn bad_strip_and_filter_parameters_are_a_400_that_does_not_echo_them() {
    let (server, run) = boot();
    for q in ["cursor=abc", "limit=abc", "limit=-1", "cursor=%3Cscript%3E"] {
        let r = get(
            server.addr(),
            Some(READ),
            &format!("/api/runs/{run}/strip?{q}"),
        );
        assert_eq!(r.status, 400, "{q}");
        assert!(
            !r.body.contains("script") && !r.body.contains("abc"),
            "{}",
            r.body
        );
    }
    // A fingerprint is 16 to 64 hex digits; the filter refuses anything else without repeating it.
    for fp in [
        "4c16",
        "zzzzzzzzzzzzzzzz",
        "..%2f..%2fetc%2fpasswd",
        "%3Cscript%3E%3Cscript%3E",
    ] {
        let r = get(
            server.addr(),
            Some(READ),
            &format!("/api/runs/{run}/trials?fp={fp}"),
        );
        assert_eq!(r.status, 400, "{fp}");
        assert!(
            !r.body.contains("passwd") && !r.body.contains("script") && !r.body.contains("zzzz")
        );
    }
    let ok = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/trials?fp=0000000000000000"),
    );
    assert_eq!(ok.status, 200);
    assert_eq!(ok.json()["trials"].as_array().unwrap().len(), 2);
}

#[test]
fn a_trial_row_carries_its_class_and_failing_ids_and_a_property_can_be_read_without_seeds() {
    let (server, run) = boot();
    let v = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/trials?verdict=violation"),
    )
    .json();
    let row = &v["trials"][0];
    assert_eq!(row["class"], "violation");
    assert_eq!(row["failing_count"], 1);
    assert_eq!(row["failing_idx"][0], 0);
    assert_eq!(row["guest"]["failing_ids"][0], "b.broken");
    let p = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/properties/0?seeds=none"),
    );
    assert_eq!(p.status, 200);
    let p = p.json();
    assert_eq!(p["seeds_kind"], "none");
    assert_eq!(p["seeds"].as_array().unwrap().len(), 0);
    assert_eq!(p["scanned_trials"], 0);
}

#[test]
fn output_is_a_plain_text_attachment_with_a_server_built_file_name() {
    let (server, run) = boot();
    let r = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/trials/1/output/stdout"),
    );
    assert_eq!(r.status, 200);
    assert_eq!(r.body, OUTPUT);
    assert_eq!(r.header("content-type"), Some("text/plain; charset=utf-8"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        r.header("content-security-policy"),
        Some("sandbox"),
        "program output must not run script if a browser renders it"
    );
    let disposition = r.header("content-disposition").unwrap();
    assert_eq!(
        disposition,
        format!("attachment; filename=\"{run}-1-stdout.txt\"")
    );
    assert!(!disposition.contains("script"));
    // A trial that kept nothing has nothing to download.
    assert_eq!(
        get(
            server.addr(),
            Some(READ),
            &format!("/api/runs/{run}/trials/0/output/stdout")
        )
        .status,
        404
    );
}

#[test]
fn coverage_is_served() {
    let (server, run) = boot();
    let v = get(
        server.addr(),
        Some(READ),
        &format!("/api/runs/{run}/coverage"),
    )
    .json();
    assert_eq!(v["trials"], 3);
    assert_eq!(v["points"].as_array().unwrap().last().unwrap()["seq"], 2);
}

#[test]
fn no_worker_id_reaches_a_read_caller() {
    let (server, run) = boot();
    for path in [
        format!("/api/runs/{run}/properties"),
        format!("/api/runs/{run}/trials"),
        format!("/api/runs/{run}/trials/1"),
        format!("/api/runs/{run}"),
    ] {
        let body = get(server.addr(), Some(READ), &path).body;
        assert!(!body.contains(&Service::local_worker_id()), "{path}");
        assert!(!body.contains("worker_id"), "{path}");
    }
}

#[test]
fn the_read_endpoints_sit_behind_the_rate_limiter() {
    let (server, run) = boot_limited(3);
    let path = format!("/api/runs/{run}/properties");
    let statuses: Vec<u16> = (0..6)
        .map(|_| get(server.addr(), Some(READ), &path).status)
        .collect();
    assert_eq!(&statuses[..3], &[200, 200, 200]);
    assert!(statuses[3..].iter().all(|s| *s == 429), "{statuses:?}");
}
