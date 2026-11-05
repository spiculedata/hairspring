//! `GET /api/search` over real HTTP: the role, the rate limit, what may be asked, what comes back (grouped, bounded,
//! ids and names only, guest text marked), and that an error never repeats the query.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{
    CatalogRowRecord, NewFailure, NewRun, NewTrial, SqliteStore, Store, TrialBatch, TrialVerdict,
    MAX_DIGEST_BYTES_PER_RUN, MAX_KEPT_BYTES_PER_RUN,
};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const RUN_TOKEN: &str = "run-token-0123456789abcdef";
const SECRET_OUTPUT: &str = "SECRET-GUEST-OUTPUT-MARKER";
const SECRET_MESSAGE: &str = "SECRET-FAILURE-MESSAGE-MARKER";
const FINGERPRINT: &str = "0123456789abcdef";
const HOSTILE_PROPERTY: &str = "db.commit\n\u{202e}<img src=x onerror=alert(1)>";

struct Reply {
    status: u16,
    head: String,
    text: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text))
    }
}

/// One GET with the path exactly as given, so percent-escapes that are not valid UTF-8 reach the server.
fn get(addr: SocketAddr, token: Option<&str>, path: &str) -> Reply {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Connection: close\r\n\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let out = String::from_utf8_lossy(&raw).into_owned();
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status line: {out:?}"));
    let (head, body) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
    Reply {
        status,
        head: head.to_ascii_lowercase(),
        text: body.to_owned(),
    }
}

fn search(addr: SocketAddr, q: &str) -> Reply {
    get(addr, Some(READ), &format!("/api/search?q={q}"))
}

fn catalog(idx: u32, id: &str) -> CatalogRowRecord {
    CatalogRowRecord {
        idx,
        prop_id: id.into(),
        kind: "always".into(),
        must_hit: true,
        msg: SECRET_MESSAGE.into(),
        loc_file: None,
        loc_line: None,
        loc_func: None,
        decided: true,
    }
}

fn trial(seq: u64, seed: u64) -> NewTrial {
    NewTrial {
        seq,
        seed,
        verdict: TrialVerdict::Pass,
        reason: String::new(),
        exit_code: Some(0),
        signal: None,
        timed_out: false,
        degraded: false,
        perturbing: false,
        report_missing: false,
        rated: true,
        claimable: true,
        behaviour_hash: None,
        schedule_fp: Some(FINGERPRINT.into()),
        syscalls: 1,
        work_ops: 1,
        elapsed_ms: 1,
        max_idx: Some(0),
        digest: vec![1, 0, 0, 0],
        detail: None,
        output: Some(format!("{{\"stdout\":\"{SECRET_OUTPUT}\",\"stderr\":\"\"}}").into_bytes()),
        on_digest_cap: None,
    }
}

struct Fixture {
    server: RunningServer,
    runs: Vec<String>,
    failure: String,
}

fn boot(limit: u32) -> Fixture {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let runs: Vec<String> = (0..7)
        .map(|i| {
            store
                .create_run(
                    &NewRun {
                        target: "lock-lease".into(),
                        backend: "sim".into(),
                        seed: 424242,
                        config_json: "{}".into(),
                    },
                    100 + i,
                )
                .unwrap()
                .id
        })
        .collect();
    let newest = &runs[6];
    store.enqueue_job(newest, "run", "{}", 200).unwrap();
    let job = store.lease_job("w1", 600, 201).unwrap().unwrap();
    store.mark_running(newest, 202).unwrap();
    let batch = TrialBatch {
        catalog: vec![catalog(0, HOSTILE_PROPERTY), catalog(1, "cache.hit")],
        trials: vec![trial(0, 777001)],
        kept_bytes_cap: MAX_KEPT_BYTES_PER_RUN,
        digest_bytes_cap: MAX_DIGEST_BYTES_PER_RUN,
    };
    store
        .ingest_trials(&job.id, "w1", true, &batch, 203)
        .unwrap();
    let failure = store
        .add_failure(
            &NewFailure {
                run_id: runs[0].clone(),
                target: "lock-lease".into(),
                message: SECRET_MESSAGE.into(),
                original_len: 3,
                shrunk_len: 1,
                bundle_hash: "0".repeat(64),
                steps_json: "[]".into(),
            },
            300,
        )
        .unwrap()
        .id;
    let auth = Auth::from_spec(&format!("{READ}:read,{RUN_TOKEN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = limit;
    Fixture {
        server: start(cfg, Service::new(store), engine).expect("server starts"),
        runs,
        failure,
    }
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[test]
fn it_needs_a_token_and_any_role_may_search() {
    let f = boot(0);
    let addr = f.server.addr();
    assert_eq!(get(addr, None, "/api/search?q=run_").status, 401);
    assert_eq!(
        get(
            addr,
            Some("wrong-token-0123456789abcdef"),
            "/api/search?q=run_"
        )
        .status,
        401
    );
    assert_eq!(get(addr, Some(READ), "/api/search?q=run_").status, 200);
    assert_eq!(get(addr, Some(RUN_TOKEN), "/api/search?q=run_").status, 200);
}

#[test]
fn the_rate_limit_counts_searches_like_any_other_request() {
    let f = boot(2);
    let addr = f.server.addr();
    let statuses: Vec<u16> = (0..4)
        .map(|_| get(addr, Some(READ), "/api/search?q=run_").status)
        .collect();
    assert_eq!(statuses, vec![200, 200, 429, 429]);
}

#[test]
fn a_query_that_is_missing_empty_long_or_outside_the_allowed_characters_is_a_400_that_does_not_repeat_it(
) {
    let f = boot(0);
    let addr = f.server.addr();
    let long = "a".repeat(65);
    let refused = [
        "/api/search".to_owned(),
        "/api/search?q=".to_owned(),
        "/api/search?other=run_".to_owned(),
        format!("/api/search?q={long}"),
        "/api/search?q=a%20b".to_owned(),
        "/api/search?q=a+b".to_owned(),
        "/api/search?q=%25".to_owned(),
        "/api/search?q=a%25b".to_owned(),
        "/api/search?q=%00".to_owned(),
        "/api/search?q=%0a".to_owned(),
        "/api/search?q=x%27%20OR%201%3D1--".to_owned(),
        "/api/search?q=%3Cscript%3Ealert(1)%3C/script%3E".to_owned(),
        "/api/search?q=caf%C3%A9".to_owned(),
        "/api/search?q=%E2%80%AErun_".to_owned(),
        "/api/search?q=%FF%FE".to_owned(),
    ];
    for path in refused {
        let r = get(addr, Some(READ), &path);
        assert_eq!(r.status, 400, "{path}: {}", r.text);
        let body = r.json();
        assert_eq!(body["error"], "`q` is not valid", "{path}");
        assert!(!r.text.contains(&long), "{path}");
        for echoed in ["script", "OR 1", "caf", "%FF", "run_"] {
            assert!(
                !r.text.contains(echoed),
                "{path} echoed {echoed}: {}",
                r.text
            );
        }
    }
}

#[test]
fn a_run_id_prefix_lists_the_newest_runs_up_to_the_limit_and_says_there_are_more() {
    let f = boot(0);
    let r = search(f.server.addr(), "run_").json();
    let runs = r["runs"].as_array().unwrap();
    let want: Vec<&String> = f.runs.iter().rev().take(5).collect();
    assert_eq!(
        runs.iter()
            .map(|h| h["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        want.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "newest first, five at most"
    );
    assert_eq!(keys(&runs[0]), vec!["id", "state", "target"]);
    assert_eq!(runs[0]["target"], "lock-lease");
    assert_eq!(r["more"], serde_json::json!(["runs"]));
    assert_eq!(
        r["scope"],
        serde_json::json!({"per_group": 5, "recent_runs": 1000, "property_runs": 100})
    );
    let whole = search(f.server.addr(), &f.runs[0]).json();
    assert_eq!(whole["runs"][0]["id"], f.runs[0].as_str());
    assert_eq!(whole["more"], serde_json::json!([]));
}

#[test]
fn every_group_is_present_and_a_miss_is_an_empty_answer_not_an_error() {
    let f = boot(0);
    let r = search(f.server.addr(), "zzzz").json();
    assert_eq!(
        keys(&r),
        vec![
            "failures",
            "more",
            "properties",
            "runs",
            "schedules",
            "scope",
            "seeds",
            "targets"
        ]
    );
    for group in [
        "runs",
        "failures",
        "targets",
        "properties",
        "seeds",
        "schedules",
    ] {
        assert_eq!(r[group], serde_json::json!([]), "{group}");
    }
}

#[test]
fn failures_targets_seeds_and_schedules_are_found_by_their_ids() {
    let f = boot(0);
    let addr = f.server.addr();
    let by_failure = search(addr, &f.failure).json();
    assert_eq!(by_failure["failures"][0]["id"], f.failure.as_str());
    assert_eq!(by_failure["failures"][0]["run_id"], f.runs[0].as_str());
    assert_eq!(
        keys(&by_failure["failures"][0]),
        vec!["id", "run_id", "target"]
    );

    let by_target = search(addr, "lock").json();
    assert_eq!(by_target["targets"][0]["name"], "lock-lease");

    let by_seed = search(addr, "777001").json();
    assert_eq!(
        by_seed["seeds"][0],
        serde_json::json!({"run_id": f.runs[6], "seed": 777001, "trial": 0})
    );
    let by_run_seed = search(addr, "424242").json();
    assert_eq!(by_run_seed["seeds"][0]["trial"], Value::Null);
    assert_eq!(by_run_seed["seeds"].as_array().unwrap().len(), 5);
    assert_eq!(by_run_seed["more"], serde_json::json!(["seeds"]));

    let by_schedule = search(addr, FINGERPRINT).json();
    assert_eq!(
        by_schedule["schedules"][0],
        serde_json::json!({"fingerprint": FINGERPRINT, "run_id": f.runs[6], "trial": 0})
    );
}

#[test]
fn a_property_id_is_guest_text_so_it_is_one_line_and_under_a_guest_object() {
    let f = boot(0);
    let r = search(f.server.addr(), "db.").json();
    let hit = &r["properties"][0];
    assert_eq!(keys(hit), vec!["guest", "run_id"]);
    assert_eq!(hit["run_id"], f.runs[6].as_str());
    assert_eq!(keys(&hit["guest"]), vec!["id", "note"]);
    let id = hit["guest"]["id"].as_str().unwrap();
    assert!(id.starts_with("db.commit"), "{id}");
    assert!(!id.contains('\n') && !id.contains('\u{202e}'), "{id:?}");
    assert!(hit["guest"]["note"].as_str().unwrap().contains("untrusted"));
    let plain = search(f.server.addr(), "cache.").json();
    assert_eq!(plain["properties"][0]["guest"]["id"], "cache.hit");
}

#[test]
fn nothing_the_program_wrote_is_ever_in_an_answer() {
    let f = boot(0);
    let addr = f.server.addr();
    for q in [
        "run_",
        "fail_",
        "db.",
        "cache.",
        "lock",
        "777001",
        "424242",
        FINGERPRINT,
    ] {
        let text = search(addr, q).text;
        assert!(!text.contains(SECRET_OUTPUT), "{q}: {text}");
        assert!(!text.contains(SECRET_MESSAGE), "{q}: {text}");
        assert!(!text.contains("stdout"), "{q}: {text}");
    }
}

#[test]
fn a_search_is_never_cached() {
    let f = boot(0);
    let r = search(f.server.addr(), "run_");
    assert!(r.head.contains("cache-control: no-store"), "{}", r.head);
}

#[test]
fn the_text_is_a_name_not_a_pattern() {
    let f = boot(0);
    for q in ["run_%25", "run__", "fail_%25"] {
        let r = search(f.server.addr(), q);
        assert_eq!(r.status, if q.contains("%25") { 400 } else { 200 }, "{q}");
    }
    let r = search(f.server.addr(), "run__").json();
    assert_eq!(r["runs"], serde_json::json!([]), "_ is not a wildcard");
}
